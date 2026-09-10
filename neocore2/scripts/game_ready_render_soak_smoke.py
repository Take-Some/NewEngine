#!/usr/bin/env python3
"""Gameplay render-CPU soak with an explicit post-warmup frame budget gate."""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import statistics
import subprocess
import sys
import time
from collections import Counter
from ctypes import wintypes
from pathlib import Path

WM_CLOSE = 0x0010
SW_RESTORE = 9
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
DESIGN_WIDTH = 1600
DESIGN_HEIGHT = 900
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


class Rect(ctypes.Structure):
    _fields_=[('left',ctypes.c_long),('top',ctypes.c_long),('right',ctypes.c_long),('bottom',ctypes.c_long)]


class Point(ctypes.Structure):
    _fields_=[('x',ctypes.c_long),('y',ctypes.c_long)]


def args() -> argparse.Namespace:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe',type=Path)
    parser.add_argument('--duration',type=float,default=60.0)
    parser.add_argument('--startup-timeout',type=float,default=45.0)
    parser.add_argument('--gameplay-timeout',type=float,default=120.0)
    parser.add_argument('--warmup-frames',type=int,default=120)
    parser.add_argument('--gameplay-warmup-seconds',type=float,default=8.0)
    parser.add_argument('--budget-ms',type=float,default=16.67)
    parser.add_argument('--render-warn-ms',type=float,default=1000.0)
    parser.add_argument('--max-frame-ms',type=float,default=33.34)
    parser.add_argument('--max-over-budget-ratio',type=float,default=0.05)
    parser.add_argument('--min-samples',type=int,default=8)
    parser.add_argument('--memory-warmup-seconds',type=float,default=30.0)
    parser.add_argument('--max-memory-growth-mib',type=float,default=256.0)
    parser.add_argument('--heartbeat-seconds',type=float,default=300.0)
    parser.add_argument('--max-stderr-mib',type=float,default=128.0)
    return parser.parse_args()


def percentile(values:list[float], q:float)->float:
    if not values:
        return math.nan
    ordered=sorted(values)
    if len(ordered)==1:
        return ordered[0]
    pos=(len(ordered)-1)*q
    lo=math.floor(pos); hi=math.ceil(pos)
    if lo==hi:
        return ordered[lo]
    return ordered[lo]+(ordered[hi]-ordered[lo])*(pos-lo)


def transition(source:str,target:str,trigger:str)->str:
    return (
        "screen profile: presentation flow transition flow='game.frontend' "
        f"from='{source}' to='{target}' trigger='{trigger}'"
    )


def working_set_bytes(pid:int)->int|None:
    kernel32=ctypes.windll.kernel32
    psapi=ctypes.windll.psapi
    handle=kernel32.OpenProcess(PROCESS_QUERY_INFORMATION|PROCESS_VM_READ,False,pid)
    if not handle:
        return None
    try:
        counters=ProcessMemoryCounters(); counters.cb=ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(handle,ctypes.byref(counters),counters.cb):
            return None
        return int(counters.WorkingSetSize)
    finally:
        kernel32.CloseHandle(handle)


def main()->int:
    if os.name!='nt':
        print('Windows is required.',file=sys.stderr)
        return 2
    opt=args()
    root=Path(__file__).resolve().parents[1]
    exe=(opt.exe or root/'target/release/NewEngine.exe').resolve()
    if not exe.is_file():
        print(f'executable not found: {exe}',file=sys.stderr)
        return 2
    smoke=root/'target/smoke'; smoke.mkdir(parents=True,exist_ok=True)
    stdout_path=smoke/'game-ready-render-soak.stdout.log'
    stderr_path=smoke/'game-ready-render-soak.stderr.log'
    profiler_path=root/'cache/profiler/profiler_report_latest.json'
    verdict_path=smoke/'game-ready-render-soak-report.json'
    profiler_before=(profiler_path.stat().st_mtime_ns,profiler_path.stat().st_size) if profiler_path.exists() else None
    log_dir=root/'cache/logs'; log_dir.mkdir(parents=True,exist_ok=True)
    before_shards={p.resolve() for p in log_dir.glob('current.ulog.*.ndjson') if not p.name.endswith('.bootstrap.ndjson') and '.orphan.' not in p.name}
    user32=ctypes.windll.user32
    process:subprocess.Popen[str]|None=None
    memory_samples:list[tuple[float,int]]=[]
    soak_started_unix_ms:int|None=None

    def stderr_text()->str:
        try: return stderr_path.read_text(encoding='utf-8',errors='replace')
        except FileNotFoundError: return ''

    def wait_for(needle:str,timeout:float,label:str)->None:
        wait_for_any((needle,),timeout,label)

    def wait_for_any(needles:tuple[str,...],timeout:float,label:str)->str:
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            if process is not None and process.poll() is not None:
                raise RuntimeError(f'process exited {process.returncode} while waiting for {label}')
            text=stderr_text()
            for needle in needles:
                if needle in text:
                    print(f'PASS {label}: {needle}',flush=True)
                    return needle
            time.sleep(.2)
        raise RuntimeError(f'timeout waiting for {label}: {needles}')

    def windows(pid:int):
        found=[]
        callback_type=ctypes.WINFUNCTYPE(ctypes.c_bool,wintypes.HWND,wintypes.LPARAM)
        @callback_type
        def callback(hwnd,_):
            owner=ctypes.c_ulong(); user32.GetWindowThreadProcessId(hwnd,ctypes.byref(owner))
            if owner.value==pid and user32.IsWindow(hwnd):
                rect=Rect(); user32.GetClientRect(hwnd,ctypes.byref(rect))
                area=max(0,rect.right-rect.left)*max(0,rect.bottom-rect.top)
                found.append((area,bool(user32.IsWindowVisible(hwnd)),int(hwnd),rect))
            return True
        user32.EnumWindows(callback,0)
        return sorted(found,key=lambda item:(item[1],item[0]),reverse=True)

    def game_window()->tuple[int,Rect]:
        assert process is not None
        candidates=[item for item in windows(process.pid) if item[1] and item[0]>100_000]
        if not candidates: raise RuntimeError('game window not found')
        return candidates[0][2],candidates[0][3]

    def click(hwnd:int,x:int,y:int)->None:
        rect=Rect(); user32.GetClientRect(hwnd,ctypes.byref(rect))
        width=max(1,rect.right-rect.left); height=max(1,rect.bottom-rect.top)
        point=Point(round(x*width/DESIGN_WIDTH),round(y*height/DESIGN_HEIGHT))
        user32.ClientToScreen(hwnd,ctypes.byref(point))
        user32.ShowWindow(hwnd,SW_RESTORE); user32.SetForegroundWindow(hwnd); user32.SetFocus(hwnd)
        user32.SetCursorPos(point.x,point.y); time.sleep(.15)
        user32.mouse_event(MOUSEEVENTF_LEFTDOWN,0,0,0,0); time.sleep(.07)
        user32.mouse_event(MOUSEEVENTF_LEFTUP,0,0,0,0)

    env=os.environ.copy(); env['RUST_BACKTRACE']='1'
    env['NEWENGINE_RENDER_PROFILER_SAMPLE_INTERVAL_FRAMES']='30'
    env['NEWENGINE_RENDER_SLOW_PROFILE_INTERVAL_FRAMES']='30'
    # Slow-frame warning emission is diagnostic I/O and must not perturb the benchmark.
    # Frame-budget pass/fail is computed from StarProfiler samples below.
    env['NEWENGINE_RENDER_WARN_MS']=str(max(opt.render_warn_ms,opt.max_frame_ms))
    env['NEWENGINE_PLUGIN_ENGINE_PROFILER_STARPROFILER__diagnostics__max_recent_jobs']='16384'
    for key in ('NEWENGINE_PLUGIN_DIR','NEWENGINE_PLUGINS_DIR','NEWENGINE_PLATFORM_RUNTIME_DIR','NEWENGINE_PLATFORM_EARLY_LOG','NEWENGINE_WINIT_EARLY_LOG'):
        env.pop(key,None)

    result=0
    started=time.monotonic()
    verdict:dict[str,object]={
        'schema':'northstar.game-ready.render-soak.v1',
        'duration_seconds':opt.duration,
        'gameplay_warmup_seconds':opt.gameplay_warmup_seconds,
        'budget_ms':opt.budget_ms,
        'render_warn_ms':max(opt.render_warn_ms,opt.max_frame_ms),
        'frame_budget':None,
        'memory_plateau':None,
        'ulog':None,
    }
    try:
        with stdout_path.open('w',encoding='utf-8') as out, stderr_path.open('w',encoding='utf-8') as err:
            process=subprocess.Popen([str(exe),'--no-startup-window'],cwd=root,env=env,stdout=out,stderr=err,text=True)
            print(f'PROCESS pid={process.pid}',flush=True)
            legacy_menu = "authored game .neui mounted ref='ui/frontend/main_menu.neui@surface'"
            direct_gameplay = (
                "render controller: scene launch gate released; loading overlay deactivated; "
                "deferring first world present to next frame"
            )
            route=wait_for_any(
                (direct_gameplay,legacy_menu),
                max(opt.startup_timeout,opt.gameplay_timeout),
                'playable route',
            )
            if route==legacy_menu:
                hwnd,_=game_window(); click(hwnd,240,336)
                wait_for(transition('main_menu','loading','game.start'),10,'main menu -> loading')
                wait_for(transition('loading','gameplay','runtime_ready'),opt.gameplay_timeout,'loading -> gameplay')
                wait_for("authored game .neui mounted ref='ui/game/game_hud.neui@surface'",30,'game HUD mounted')
                print('PLAYABLE_ROUTE legacy-menu',flush=True)
            else:
                print('PLAYABLE_ROUTE direct-gameplay',flush=True)
            gameplay_warmup=max(0.0,opt.gameplay_warmup_seconds)
            if gameplay_warmup>0.0:
                print(f'GAMEPLAY_WARMUP duration={gameplay_warmup:.1f}s',flush=True)
                warmup_deadline=time.monotonic()+gameplay_warmup
                while time.monotonic()<warmup_deadline:
                    if process.poll() is not None:
                        raise RuntimeError(f'process exited during gameplay warmup: {process.returncode}')
                    time.sleep(min(.25,max(0.0,warmup_deadline-time.monotonic())))
            print(f'SOAK duration={opt.duration:.1f}s',flush=True)
            soak_started=time.monotonic(); soak_started_unix_ms=int(time.time()*1000); deadline=soak_started+opt.duration
            next_heartbeat=soak_started+max(1.0,opt.heartbeat_seconds)
            max_stderr_bytes=max(1,int(opt.max_stderr_mib*1024*1024))
            while time.monotonic()<deadline:
                if process.poll() is not None:
                    raise RuntimeError(f'process exited during soak: {process.returncode}')
                now=time.monotonic()
                ws=working_set_bytes(process.pid)
                if ws is not None:
                    memory_samples.append((now-soak_started,ws))
                if stderr_path.exists() and stderr_path.stat().st_size>max_stderr_bytes:
                    raise RuntimeError(
                        f'stderr exceeded {opt.max_stderr_mib:.1f} MiB during soak'
                    )
                if now>=next_heartbeat:
                    current_mib=(ws/1024/1024) if ws is not None else float('nan')
                    print(
                        f'SOAK_HEARTBEAT elapsed={now-soak_started:.1f}s '
                        f'working_set_mib={current_mib:.1f}',
                        flush=True,
                    )
                    next_heartbeat=now+max(1.0,opt.heartbeat_seconds)
                time.sleep(min(.5,max(0.0,deadline-time.monotonic())))
    except Exception as error:
        print(f'FAIL {error}',file=sys.stderr)
        result=1
    finally:
        if process is not None and process.poll() is None:
            for item in windows(process.pid): user32.PostMessageW(item[2],WM_CLOSE,0,0)
            try: process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=5)
        if process is not None:
            print(f'PROCESS exit_code={process.returncode}',flush=True)
            if process.returncode!=0: result=1

    deadline=time.monotonic()+15
    while time.monotonic()<deadline:
        current=(profiler_path.stat().st_mtime_ns,profiler_path.stat().st_size) if profiler_path.exists() else None
        if current is not None and current!=profiler_before: break
        time.sleep(.25)
    else:
        print('FAIL profiler report was not refreshed',file=sys.stderr)
        return 1

    report=json.loads(profiler_path.read_text(encoding='utf-8'))
    samples=[job for job in report.get('completed_jobs',[]) if job.get('name')=='render cpu profile' and isinstance(job.get('frame_id'),(int,float)) and job['frame_id']>=opt.warmup_frames and (soak_started_unix_ms is None or (job.get('ended_unix_ms') or 0)>=soak_started_unix_ms)]
    values=[float(job['elapsed_ms']) for job in samples if isinstance(job.get('elapsed_ms'),(int,float))]
    if len(values)<opt.min_samples:
        print(f'FAIL insufficient post-warmup samples: {len(values)} < {opt.min_samples}',file=sys.stderr)
        result=1
    if values:
        over=sum(value>opt.budget_ms for value in values)
        ratio=over/len(values)
        avg=statistics.fmean(values); p95=percentile(values,.95); maximum=max(values)
        print(f'FRAME_BUDGET samples={len(values)} warmup_frames={opt.warmup_frames} average_ms={avg:.3f} p95_ms={p95:.3f} max_ms={maximum:.3f} budget_ms={opt.budget_ms:.3f} over_budget={over} over_ratio={ratio:.3f}')
        verdict['frame_budget']={
            'samples':len(values),
            'warmup_frames':opt.warmup_frames,
            'average_ms':avg,
            'p95_ms':p95,
            'max_ms':maximum,
            'budget_ms':opt.budget_ms,
            'over_budget':over,
            'over_ratio':ratio,
        }
        if p95>opt.budget_ms:
            print(f'FAIL p95 {p95:.3f}ms exceeds {opt.budget_ms:.3f}ms',file=sys.stderr); result=1
        if maximum>opt.max_frame_ms:
            print(f'FAIL max {maximum:.3f}ms exceeds {opt.max_frame_ms:.3f}ms',file=sys.stderr); result=1
        if ratio>opt.max_over_budget_ratio:
            print(f'FAIL over-budget ratio {ratio:.3f} exceeds {opt.max_over_budget_ratio:.3f}',file=sys.stderr); result=1

    platform_samples=[job for job in report.get('completed_jobs',[]) if job.get('category')=='platform.frame' and isinstance(job.get('frame_id'),(int,float)) and isinstance(job.get('ended_unix_ms'),(int,float)) and (soak_started_unix_ms is None or job['ended_unix_ms']>=soak_started_unix_ms)]
    platform_samples.sort(key=lambda job:job['ended_unix_ms'])
    if len(platform_samples)>=2:
        frame_delta=float(platform_samples[-1]['frame_id'])-float(platform_samples[0]['frame_id'])
        time_delta=(float(platform_samples[-1]['ended_unix_ms'])-float(platform_samples[0]['ended_unix_ms']))/1000.0
        observed_fps=frame_delta/time_delta if time_delta>0.0 else math.nan
        print(f'OBSERVED_FPS fps={observed_fps:.3f} frame_delta={frame_delta:.0f} span_seconds={time_delta:.3f}')
        verdict['observed_fps']={
            'fps':observed_fps,
            'frame_delta':frame_delta,
            'span_seconds':time_delta,
            'samples':len(platform_samples),
        }

    memory_warmup=min(max(0.0,opt.memory_warmup_seconds),max(0.0,opt.duration*0.5))
    warm_memory=[(elapsed,value) for elapsed,value in memory_samples if elapsed>=memory_warmup]
    if len(warm_memory)<10:
        print(f'FAIL insufficient post-warmup memory samples: {len(warm_memory)}',file=sys.stderr)
        result=1
    else:
        window=max(1,len(warm_memory)//4)
        baseline=statistics.median(value for _,value in warm_memory[:window])
        tail=statistics.median(value for _,value in warm_memory[-window:])
        peak=max(value for _,value in warm_memory)
        growth=tail-baseline
        allowed=max(0.0,opt.max_memory_growth_mib)*1024*1024
        span=max(1e-6,warm_memory[-1][0]-warm_memory[0][0])
        slope_mib_hour=(growth/1024/1024)/(span/3600.0)
        print(
            f'MEMORY_PLATEAU samples={len(warm_memory)} warmup_seconds={memory_warmup:.1f} '
            f'baseline_mib={baseline/1024/1024:.1f} tail_mib={tail/1024/1024:.1f} '
            f'peak_mib={peak/1024/1024:.1f} growth_mib={growth/1024/1024:.1f} '
            f'slope_mib_hour={slope_mib_hour:.1f} allowed_growth_mib={opt.max_memory_growth_mib:.1f}'
        )
        verdict['memory_plateau']={
            'samples':len(warm_memory),
            'warmup_seconds':memory_warmup,
            'baseline_mib':baseline/1024/1024,
            'tail_mib':tail/1024/1024,
            'peak_mib':peak/1024/1024,
            'growth_mib':growth/1024/1024,
            'slope_mib_hour':slope_mib_hour,
            'allowed_growth_mib':opt.max_memory_growth_mib,
        }
        if growth>allowed:
            print(
                f'FAIL memory growth {growth/1024/1024:.1f} MiB exceeds '
                f'{opt.max_memory_growth_mib:.1f} MiB',
                file=sys.stderr,
            )
            result=1

    new_shards=[p for p in log_dir.glob('current.ulog.*.ndjson') if p.resolve() not in before_shards and not p.name.endswith('.bootstrap.ndjson') and '.orphan.' not in p.name]
    if not new_shards:
        print('FAIL no run ULOG shard',file=sys.stderr); result=1
    else:
        shard=max(new_shards,key=lambda p:p.stat().st_mtime_ns)
        records=[]; bad=0
        for line in shard.read_text(encoding='utf-8',errors='replace').splitlines():
            if not line.strip(): continue
            try: records.append(json.loads(line))
            except json.JSONDecodeError: bad+=1
        levels=Counter(str(record.get('level','')).upper() for record in records)
        print(f'ULOG path={shard} rows={len(records)} bad_json={bad} levels={dict(levels)}')
        verdict['ulog']={
            'path':str(shard),
            'rows':len(records),
            'bad_json':bad,
            'levels':dict(levels),
        }
        if bad or levels.get('ERROR',0) or levels.get('FATAL',0): result=1

    elapsed=time.monotonic()-started
    verdict['elapsed_seconds']=elapsed
    verdict['status']='passed' if result==0 else 'failed'
    verdict_path.write_text(json.dumps(verdict,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(f'VERDICT {verdict_path}')
    print(f'ELAPSED seconds={elapsed:.1f}')
    if result==0: print('RENDER_SOAK_SMOKE_OK')
    else:
        print(f'STDOUT {stdout_path}')
        print(f'STDERR {stderr_path}')
    return result


if __name__=='__main__':
    raise SystemExit(main())
