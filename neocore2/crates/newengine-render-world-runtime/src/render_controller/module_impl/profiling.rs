#![forbid(unsafe_op_in_unsafe_fn)]

use std::time::Instant;

const PROFILER_SAMPLE_TOPIC: &str = "newengine.diagnostics.profiler.sample.v1";

/// Small frame-local timing collector used by render orchestration.
///
/// It is intentionally allocation-light and domain-agnostic: callers decide the
/// semantic labels, this type only records elapsed milliseconds and formats a
/// compact diagnostic suffix.
#[derive(Debug)]
pub(super) struct FrameCpuProfile {
    started: Instant,
    last_mark: Instant,
    parts: Vec<(&'static str, f32)>,
}

impl FrameCpuProfile {
    #[inline]
    pub(super) fn new() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_mark: now,
            parts: Vec::with_capacity(8),
        }
    }

    #[inline]
    pub(super) fn mark(&mut self, label: &'static str) {
        let now = Instant::now();
        self.parts.push((
            label,
            now.duration_since(self.last_mark).as_secs_f32() * 1000.0,
        ));
        self.last_mark = now;
    }

    #[inline]
    pub(super) fn total_ms(&self) -> f32 {
        self.started.elapsed().as_secs_f32() * 1000.0
    }

    pub(super) fn breakdown(&self) -> String {
        self.parts
            .iter()
            .map(|(label, ms)| format!("{}={:.2}ms", label, ms))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug)]
pub(super) struct TimedBreakdown {
    started: Instant,
    parts: Vec<(String, f32)>,
}

impl TimedBreakdown {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            parts: Vec::new(),
        }
    }

    pub(super) fn time<T, E>(
        &mut self,
        label: impl Into<String>,
        work: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        let started = Instant::now();
        let result = work();
        self.parts
            .push((label.into(), started.elapsed().as_secs_f32() * 1000.0));
        result
    }

    #[inline]
    pub(super) fn total_ms(&self) -> f32 {
        self.started.elapsed().as_secs_f32() * 1000.0
    }

    #[inline]
    pub(super) fn push_measurement(&mut self, label: impl Into<String>, elapsed_ms: f32) {
        self.parts.push((label.into(), elapsed_ms.max(0.0)));
    }

    pub(super) fn breakdown(&self) -> String {
        self.parts
            .iter()
            .map(|(label, ms)| format!("{}={:.2}ms", label, ms))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[inline]
pub(super) fn trace_ms_threshold() -> f32 {
    newengine_runtime_policy::diagnostics_policy().render_trace_ms
}

#[inline]
pub(super) fn warn_ms_threshold() -> f32 {
    newengine_runtime_policy::diagnostics_policy().render_warn_ms
}

#[inline]
pub(super) fn profiler_outlier_ms_threshold() -> f32 {
    newengine_runtime_policy::diagnostics_policy().render_profiler_outlier_ms
}

#[inline]
pub(super) fn slow_profile_log_interval_frames() -> u64 {
    newengine_runtime_policy::diagnostics_policy().render_slow_profile_interval_frames
}

#[inline]
pub(super) fn profiler_sample_interval_frames() -> u64 {
    newengine_runtime_policy::diagnostics_policy().render_profiler_sample_interval_frames
}

/// Returns true only when `emit_timed_profile` would produce either a profiler
/// sample or a human-readable trace line for this frame.
///
/// Keep this predicate cheap: render orchestration uses it to avoid allocating
/// breakdown/UI strings on frames where diagnostics are completely idle.
pub(super) fn timed_profile_due(frame_index: u64, trace_frame: bool, total_ms: f32) -> bool {
    let policy = newengine_runtime_policy::diagnostics_policy();
    let slow = total_ms >= policy.render_warn_ms;
    let traceable = trace_frame || total_ms >= policy.render_trace_ms;
    let sample_due = policy.render_profiler_samples
        && (trace_frame
            || frame_index.is_multiple_of(policy.render_profiler_sample_interval_frames)
            || total_ms >= policy.render_profiler_outlier_ms);
    let log_due = if !traceable && !slow {
        false
    } else if slow
        && !trace_frame
        && !frame_index.is_multiple_of(policy.render_slow_profile_interval_frames)
    {
        false
    } else {
        true
    };
    sample_due || log_due
}

/// Moves diagnostic serialization/event fan-out off the render critical path.
///
/// Profiler event sinks are synchronous ABI callbacks and may perform JSON
/// decoding, record expansion and mutex-protected retention. Sampling must not
/// turn those diagnostics into a periodic frame hitch. The engine-wide worker
/// pool owns this work; under background-lane backpressure a diagnostic sample
/// is dropped rather than falling back to synchronous render-thread execution.
pub(super) fn emit_timed_profile_deferred(
    thread_pool: Option<&newengine_core::ThreadPoolHandle>,
    label: &'static str,
    frame_index: u64,
    trace_frame: bool,
    total_ms: f32,
    breakdown: String,
    suffix: String,
) {
    if !timed_profile_due(frame_index, trace_frame, total_ms) {
        return;
    }

    if let Some(jobs) = thread_pool {
        const MAX_PENDING_DIAGNOSTIC_JOBS: usize = 8;
        if jobs.pending_for_lane(newengine_core::TaskLane::Background) < MAX_PENDING_DIAGNOSTIC_JOBS
        {
            let _ = jobs.submit_lane(
                newengine_core::TaskLane::Background,
                "render.profiler.emit",
                move || {
                    emit_timed_profile(
                        label,
                        frame_index,
                        trace_frame,
                        total_ms,
                        breakdown,
                        suffix,
                    );
                },
            );
        }
        return;
    }

    emit_timed_profile(label, frame_index, trace_frame, total_ms, breakdown, suffix);
}

pub(super) fn emit_timed_profile(
    label: &'static str,
    frame_index: u64,
    trace_frame: bool,
    total_ms: f32,
    breakdown: impl AsRef<str>,
    suffix: impl AsRef<str>,
) {
    let slow = total_ms >= warn_ms_threshold();
    let traceable = trace_frame || total_ms >= trace_ms_threshold();
    let sample_interval = profiler_sample_interval_frames();
    let profiler_outlier = total_ms >= profiler_outlier_ms_threshold();
    let should_sample =
        trace_frame || frame_index.is_multiple_of(sample_interval) || profiler_outlier;
    if should_sample {
        emit_profiler_sample(
            label,
            frame_index,
            total_ms,
            breakdown.as_ref(),
            suffix.as_ref(),
            slow,
        );
    }

    if !traceable && !slow {
        return;
    }

    if slow && !trace_frame && !frame_index.is_multiple_of(slow_profile_log_interval_frames()) {
        return;
    }

    let include_breakdown = trace_frame || slow || newengine_ulog_api::ulog::debug_enabled();
    let line = if include_breakdown {
        let suffix = suffix.as_ref();
        if suffix.is_empty() {
            format!(
                "{}: frame={} total_ms={:.2} {}",
                label,
                frame_index,
                total_ms,
                breakdown.as_ref(),
            )
        } else {
            format!(
                "{}: frame={} total_ms={:.2} {} {}",
                label,
                frame_index,
                total_ms,
                breakdown.as_ref(),
                suffix,
            )
        }
    } else {
        format!("{}: frame={} total_ms={:.2}", label, frame_index, total_ms)
    };

    if slow {
        newengine_ulog_api::ulog::warn!("{}", line);
    } else {
        newengine_ulog_api::ulog::debug!("{}", line);
    }
}

fn emit_profiler_sample(
    label: &'static str,
    frame_index: u64,
    total_ms: f32,
    breakdown: &str,
    suffix: &str,
    slow: bool,
) {
    if !newengine_runtime_policy::diagnostics_policy().render_profiler_samples {
        return;
    }
    let frame_budget_ms = warn_ms_threshold();
    let payload = serde_json::json!({
        "schema": "newengine.diagnostics.profiler.sample.v1",
        "category": "render",
        "source": "render_controller",
        "name": label,
        "detail": suffix,
        "lane": "render-prep",
        "priority": "interactive",
        "dependency_group": format!("frame.{frame_index}.render"),
        "frame_index": frame_index,
        "elapsed_ms": total_ms,
        "budget_ms": frame_budget_ms,
        "frame_budget_ms": frame_budget_ms,
        "exceeded_frame_budget": total_ms > frame_budget_ms,
        "slow": slow,
        "breakdown": breakdown,
    });
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        let _ = newengine_plugin_host::emit_plugin_event(PROFILER_SAMPLE_TOPIC, &bytes);
    }
}
