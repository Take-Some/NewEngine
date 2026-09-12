# North Star render/performance gap audit — 2026-09-10

## Scope and evidence boundary

Target under review: `NewEngine/neocore2` + first-party `PluginsSrc/VulkanRenderer` and `PluginsSrc/GameReadyRuntime`.

The requested local reference installation `C:\Games\Cyberpunk 2077` was not exported into the current SpringSuite workspace roots, so no claim in this audit is based on reading or disassembling the user's local Cyberpunk binaries, archives, shaders, or runtime memory. REDengine 4 comparisons are based on publicly documented CD PROJEKT RED / GDC material and are architecture references, not binary-equivalence claims.

## Baseline conclusions

North Star already has several production-grade building blocks:

- Vulkan deferred renderer and explicit frame graph;
- instanced batching in the world scene pass;
- persistent pipeline cache;
- asynchronous transfer/upload path with timeline synchronization and a dedicated pure-transfer queue when available;
- GPU Hi-Z pyramid and asynchronous three-slot visibility feedback ring;
- bounded visibility candidate selection using partial top-K selection rather than a full sort;
- asynchronous material texture decode with frame pump/upload budgets;
- indirect-draw capability exposed by the render API and Vulkan backend.

The remaining gap to a high-density open-world renderer is primarily architectural parallelism and submission scalability, not absence of individual visual effects.

## Confirmed gaps

### P0 — Visibility budget was spent on subjects that policy cannot reject

Before this audit, world visibility candidates were prioritized by screen coverage/proximity and submitted into a provider budget capped at 4096 subjects. The culling policy subsequently refuses to reject subjects closer than 6 m or with coverage >= 10%. In a dense scene, those near/large objects could therefore occupy the most valuable Hi-Z slots while being guaranteed visible by policy.

Implemented correction in:

`crates/newengine-render-world-runtime/src/render_controller/module_impl/visibility_control.rs`

The query builder now excludes subjects that cannot be culled and clears stale occlusion confirmation when a subject becomes ineligible. This preserves conservative visibility semantics while increasing useful coverage of the fixed GPU query budget.

Validation:

- `cargo check -p newengine-render-world-runtime --target-dir target/render-audit-verify` — PASS;
- targeted unit test `visibility_query_budget_excludes_subjects_that_cannot_be_culled` — PASS on a clean target directory;
- the original shared incremental target produced a Windows `rustc` `STATUS_ACCESS_VIOLATION (0xc0000005)` with no Rust diagnostic; repeating on a clean target succeeded, so that crash is not attributed to the patch.

### P0/P1 — World draw stream remains CPU-authored

The neutral render API and Vulkan backend expose indexed indirect and indexed-indirect-count capability, but `newengine-render-world-runtime` does not use `draw_indexed_indirect_count` for the world scene. The scene pass performs CPU visibility admission, CPU batch construction/sorting, then issues `draw_indexed(...)` per instanced batch.

Required end-state for very high object density:

`instance/object table -> GPU frustum/Hi-Z cull -> GPU compaction -> indirect command/count buffers -> vkCmdDrawIndexedIndirectCount`

The existing CPU path should remain as compatibility/fallback for hardware without the required feature set.

### P0/P1 — No real bindless/material table path in Vulkan provider

The render architecture describes scalable descriptor/material capabilities, but the current Vulkan provider contains no bindless implementation. Material/texture state therefore still fragments submission by bound resource state and prevents a fully GPU-generated heterogeneous draw stream.

Required end-state:

- descriptor indexing / bindless texture table where supported;
- stable material IDs indexing a GPU material table;
- stable geometry IDs indexing vertex/index metadata;
- generation/version-safe handles so streaming replacement cannot leave stale GPU references;
- fallback descriptor-set path for legacy hardware.

### P0/P1 — Secondary command buffers are not parallel command recording

The Vulkan graph executor can create secondary command buffers, but recording currently happens synchronously on the same execution path immediately before `cmd_execute_commands`. This is command organization, not CPU-parallel recording. GBuffer work is therefore not decomposed into independently recorded worker shards in the style needed to reduce render-prep/submit pressure at large draw counts.

Required end-state:

- per-worker/per-frame Vulkan command pools;
- immutable pass recording packets produced before backend recording;
- deterministic sharding of large opaque/depth/shadow draw sets;
- worker recording jobs;
- primary command buffer only executes completed secondary buffers in graph order;
- thresholding so small passes stay primary-only.

### P1 — No dedicated compute queue topology for graphics/compute overlap

Current Vulkan device selection creates queue state for graphics and transfer only. There is no selected/created dedicated compute queue in `QueueFamilySelection` / `create_device_profiled()`. Compute work therefore cannot be independently submitted to overlap suitable HZB/SSAO/post/lighting compute with graphics work.

Required end-state:

- select a compute-capable queue, preferring compute-without-graphics when available;
- create queue + compute command pools;
- frame-graph queue class (`graphics`, `async_compute`, `transfer`);
- timeline-semaphore cross-queue dependencies;
- resource ownership transitions only when queue families differ;
- async eligibility metadata per pass;
- automatic graphics fallback when overlap is not beneficial or no independent compute queue exists.

### P1 — Asset decode QoS shares one worker pool

`AssetIo` has a logical concurrency cap and frame-budget-aware scheduling, which is good, but it still shares the common worker pool with interactive jobs. A long decode that has already started is not preemptible, so historical 100–800 ms YTD/YDD decode jobs can retain worker capacity until completion.

Required end-state:

- a physically separate decode/I/O executor or hard worker reservation;
- cancellation/generation checks before expensive decode stages;
- streaming urgency queues based on visibility/camera time-to-need;
- bounded decompression/decode chunking where codecs permit it;
- explicit p95/p99 decode and upload backlog telemetry.

### P1 — Hi-Z provider-level draw pruning is not fully wired

The Vulkan provider has a real R32F Hi-Z pyramid and nonblocking feedback ring. However provider snapshot state still reports `occlusion_query_pool_ready=false`, and stable per-draw visibility handles are not wired for provider-side pruning. Engine-side world culling does consume visibility feedback before batching, so Hi-Z is useful, but the backend cannot yet compact/prune its own submitted draw stream.

This should converge with the GPU-driven indirect pipeline rather than creating a second independent culling architecture.

## Profiler interpretation correction

Do not interpret the current report's lane percentages as exclusive CPU cost. `frame-cadence` is a frame-to-frame wall-clock interval while `host.frame` and `engine.frame` are nested envelopes. Summing them as independent jobs double-counts elapsed time.

Optimization decisions should use leaf/exclusive scopes and GPU timestamps. The profiler should eventually report explicit `inclusive_ms` and `exclusive_ms` and identify parent/child span relationships.

## Reference architecture direction

Public CD PROJEKT RED material for Night City emphasizes redesigned world hierarchy/streaming, CPU and memory work for city scale, multiple command-list preparation for GBuffer work, and compute/graphics overlap for suitable passes. North Star should use these as design principles, not copy implementation details blindly.

The highest-return sequence is:

1. finish useful-visibility telemetry and retain the query-budget fix;
2. introduce stable GPU object/material/geometry tables;
3. add GPU cull + compact + `draw_indirect_count` for opaque/depth/shadow families;
4. make descriptor indexing/bindless an optional hardware capability with fallback;
5. implement true worker-parallel secondary recording for draw-heavy passes;
6. add real async-compute queue scheduling with timeline dependencies;
7. isolate streaming/decode QoS from frame-critical worker capacity;
8. benchmark and tune only after the architecture exposes exclusive CPU and GPU pass timing.

## Acceptance gates for the dense-scene benchmark

These are North Star engineering targets, not claimed REDengine measurements.

- no synchronous asset decode or upload wait on the frame-critical thread;
- zero Vulkan validation errors in a 10-minute dense-scene soak;
- visibility telemetry reports candidate count, eligible count, queried count, confirmed-occluded count and actually-culled primitive count;
- indirect path reports generated commands vs executed commands and fallback reason;
- render preparation p95 <= 2.0 ms on the designated benchmark host;
- CPU submit/command replay p95 <= 0.75 ms once GPU-driven submission is enabled;
- no streaming hitch > 16.67 ms attributable to a single decode/upload job during steady traversal;
- steady 60 Hz target: frame p99 <= 16.67 ms at the benchmark quality/resolution when GPU workload itself is within budget;
- frame-time variance and 1%/0.1% low are recorded alongside average FPS.

## Current audit status

Implemented and verified: visibility-query budget correction.

Not yet claimed complete: GPU-driven world submission, bindless material table, true parallel command recording, async-compute queue scheduling, decode executor isolation.

A fresh GameReadyRuntime release staging build was attempted. The first build crashed inside `rustc.exe` with Windows `STATUS_ACCESS_VIOLATION (0xc0000005)` while compiling `newengine-game-module-fps`; no Rust compile diagnostic was emitted and the installed runtime DLL was not modified. A second clean-target/single-job staging build was started to distinguish source errors from host/toolchain instability.

## 2026-09-10 continuation — visibility data-plane vertical slice

### Persistent world-space bounds cache

`PrimitiveSceneSnapshot` now carries a derived `world_bounds` sphere. It is refreshed on initial capture, authored `Bounds` changes, transform-journal updates and player-pose-driven render-model updates. World visibility admission, primitive GBuffer/forward admission and coarse static-shadow admission consume this cached sphere rather than repeating the same local-to-world transform in every pass.

This is source-patched but the final Rust compile gate is currently blocked by the host-wide `0xc0000005` failure described below. A regression test was added to verify that transform-journal updates move the cached world-space sphere while preserving radius.

### Visibility effectiveness telemetry

The world visibility control now records an explicit funnel:

`snapshot primitives -> occlusion-eligible -> queried after 4096 cap -> provider results -> confirmed occluded -> actually culled`

The final `actually_culled` count is emitted from the scene pass, where a visibility result truly prevents primitive admission. This separates provider activity from realized draw-list savings.

### Vulkan VisibilityCull execution contract

The public frame graph already defined `RenderGraphPassKind::VisibilityCull` / `StandardRenderPhase::VisibilityCull`, but Vulkan omitted it from its native phase catalog and execution registry. It therefore fell through to the generic graphics-scope route.

Source changes now give VisibilityCull:

- an explicit `RendererPhaseClass::Visibility`;
- `ComputeScope` execution;
- a direct native execution route to `execute_compute_scope_pass`;
- a precise `ComputeToIndirect` barrier profile;
- Vulkan stages `COMPUTE_SHADER -> DRAW_INDIRECT`;
- access masks `SHADER_WRITE -> INDIRECT_COMMAND_READ`.

The standard runtime recipe intentionally remains unchanged: `VisibilityCull` is still opt-in until the engine owns a validated indirect stream.

### Fail-open GPU indirect visibility cull

The neutral render API already contained the required ABI (`GpuVisibilityIndirectCandidate`, `GpuDrawIndexedIndirectCommand`, `GpuVisibilityIndirectCullArgs`, `draw_indexed_indirect_count`). Rather than adding a second API, the Vulkan provider now has a source-level implementation of the existing command.

A new renderer-owned shader `shaders/visibility/indirect_cull.comp` reads a candidate sphere table and the previous Hi-Z pyramid and mutates only `VkDrawIndexedIndirectCommand.instanceCount`. An occluded candidate becomes `instanceCount=0`; all other command fields remain CPU-initialized. Non-cullable, invalid, unsupported, stale-camera or unavailable-Hi-Z cases are fail-open and leave the command stream untouched.

Safety rules in the source implementation:

- command must be recorded in `VisibilityCull`;
- exact 32-byte candidate and 20-byte indirect-command ABI strides are required;
- offsets and buffer ranges are validated;
- candidate buffer requires STORAGE usage;
- indirect buffer requires STORAGE + INDIRECT usage;
- provider candidate cap remains 4096;
- optional indirect-cull failure does not disable the existing asynchronous Hi-Z feedback service;
- descriptor sets use the real fenced Vulkan frame slot, not `engine_frame % N`;
- until depth reprojection/current-frame depth is implemented, previous-frame Hi-Z is accepted only for an effectively static camera (centimetre-scale translation and extremely small orientation/FOV deltas).

The shader was independently validated with the repository-local `glslangValidator -V`: PASS, SPIR-V size 9688 bytes. The generated validation artifact is under `Intermediate/render-audit`, not the source tree.

### Why this does not yet make the world renderer GPU-driven

Current `InstanceBatchKey` includes the concrete vertex buffer, index buffer, bind group and texture/sampler state. Therefore heterogeneous batches cannot be collapsed into one useful multi-draw stream merely by replacing each direct draw with an indirect draw. Doing so would preserve the same CPU submission cardinality and add buffer-management overhead.

The next required data model is therefore:

1. a shared geometry arena (or equivalent portable geometry addressing) so `firstIndex` / `vertexOffset` select meshes inside common bound buffers;
2. a generation-safe geometry slot table integrated with `PrimitiveGpuEvictionQueue` and deferred resource retirement;
3. stable material slots, followed by descriptor-indexing/bindless texture indices on supported hardware;
4. stable object slots carrying geometry/material handles, transforms and world-space bounds;
5. GPU cull/compact producing indirect streams grouped only by truly unavoidable state (pipeline/material fallback class), rather than by individual mesh buffers.

A slot must increment generation on eviction/reuse. A bare `PrimitiveId -> index` table is not sufficient because streamed primitive GPU entries are explicitly removed while native buffers are retired after frame completion.

### Host/toolchain blocker

Rust compile/runtime validation is currently blocked by a reproducible host-level `STATUS_ACCESS_VIOLATION (0xc0000005)` that is not localized to North Star source:

- `cargo.exe` has crashed before emitting a diagnostic;
- Rust 1.98.1 MSVC runs crashed in different unrelated crates on consecutive attempts;
- Rust 1.97 GNU crashed in a serde build-script path;
- `rustfmt.exe` subsequently crashed on a small source file;
- a Python process probe also failed once with the same Windows exception class.

Because the failure moves between unrelated executables, crates and Rust toolchains, it is not being classified as a render compile error. `git diff --check` passes in both `NewEngine` and `VulkanRenderer`; GLSL validation passes. No GameReady runtime is being published while this host condition persists.

### Updated status

Verified before host instability: first visibility-query budget correction (`cargo check` + targeted unit test).

Source-complete and statically reviewed in this continuation: persistent world-bounds cache, visibility effectiveness telemetry, Vulkan VisibilityCull compute routing/barrier, fail-open indirect-cull provider vertical slice, regression tests for route/camera/push-layout behaviour.

Independently shader-verified: `indirect_cull.comp` via glslangValidator.

Still not runtime-claimed: geometry arena/stable GPU geometry table, bindless material table, engine-side indirect stream generation, moving-camera Hi-Z reprojection/current-depth cull, parallel command recording, dedicated async-compute queue, streaming executor isolation, Sanctuary p95/p99 benchmark.


## 2026-09-11 continuation — opt-in GBuffer + shadow indirect migration

### GBuffer migration boundary

The first GPU-driven GBuffer migration is deliberately restricted to a fidelity-safe subset: static `WorldOpaque` primitives using world transforms, `ReadWrite` depth, back-face culling and opaque sorting, with resident arena geometry, no foliage/environment dome/authored-PBR requirement and no material texture bindings. Material flags also reject alpha-test, alpha-blend and double-sided surfaces. Because the GPU-driven constants-only fragment path evaluates CSM shadow visibility, GBuffer migration additionally requires `RECEIVE_SHADOWS`; cast-only materials stay on the legacy path.

The neutral bind-group ABI now supports sequential `storage0..storage2` bindings, and the Vulkan provider maps repeated `StorageBuffer` layout entries onto those slots. The GPU-driven GBuffer pipeline consumes the existing Lit set 0 plus object/material SSBOs in set 1. `firstInstance` is the stable object-table slot. Legacy GBuffer submission suppresses only entity slots that were successfully recorded into the current-frame indirect stream; any preparation/recording failure leaves legacy submission authoritative.

### VisibilityCull / compacted indirect stream

`VisibilityCull` is an opt-in compute graph phase placed after particle simulation and before the opaque scene chain. The V2 compact command path resets its draw count, compacts camera-visible commands and relies on an explicit compute-write -> indirect-read barrier. Previous-frame Hi-Z remains fail-open and is used for indirect compaction only when the camera is effectively static; moving-camera reprojection/current-depth culling remains future work.

### Separate shadow indirect stream

Directional shadow migration is intentionally independent from camera Hi-Z. `GpuShadowIndirectState` owns an eight-frame ring of per-cascade/per-geometry-page command and count buffers. Admission starts from the existing shadow policy/distance/light-space/LOD scan, then repeats the final precise mesh-bound `ShadowCasterCull` and projected-radius test using the geometry table before emitting a command. The final eligibility count is taken after this precise cull, so coarse-vs-precise bounds differences do not unnecessarily disable an otherwise complete cascade stream.

The migrated shadow subset is intentionally narrower than the legacy caster set: resident static `WorldOpaque`, world transform, back-face culling, `CastOnly` / `CastAndReceive` / `ProfileControlled` shadow policy, material `CAST_SHADOWS`, and no alpha-test, alpha-blend, double-sided, foliage or environment-dome semantics. Masked foliage, double-sided surfaces, skinned casters and every incompatible material remain legacy. The shadow shader uses `firstInstance = object_slot`, reads only the object SSBO, reproduces the existing caster depth bias and writes the same R32F shadow-depth value. Camera Hi-Z never removes shadow casters.

Legacy shadow batches suppress a migrated entity only after the complete indirect subset has been successfully recorded. Missing residency, stale generations, page overflow, pipeline errors or any incomplete stream fail open to the legacy path.

### PreStart safety gates

The experimental settings remain fail-closed and hierarchical:

`GPU scene tables` -> `VisibilityCull + indirect` -> `Opaque shadow indirect`

All three settings default to OFF. The typed PreStart setting now exports `NEWENGINE_GPU_DRIVEN_SHADOW_INDIRECT_ENABLE`; normalization automatically clears the child shadow toggle if either parent is disabled.

### Validation matrix

- `newengine-render-world-runtime` cargo check — PASS after GBuffer and shadow integration.
- GBuffer/shadow material eligibility regression tests — PASS 2/2.
- GPU shadow indirect contract tests — PASS 2/2.
- Frame-graph VisibilityCull tests — PASS 2/2.
- V2 compact visibility binary tag-17 round-trip — PASS 1/1.
- `newengine-core` + `newengine-startup-window-egui` cargo check — PASS.
- PreStart fail-closed GPU-driven settings test — PASS 1/1.
- Vulkan provider `engine-render-vulkan` cargo check — PASS.
- Vulkan visibility/provider suite — PASS 11/11.
- `gpu_driven_gbuffer.vert`, `gpu_driven_gbuffer.frag`, `gpu_driven_shadow.vert`, `gpu_driven_shadow.frag` — PASS through repository-local `glslangValidator -V`.
- targeted engine/provider `git diff --check` — PASS; only normal LF/CRLF warnings remain.

Intermittent Windows `rustc` `STATUS_ACCESS_VIOLATION (0xc0000005)` still occurs in unrelated crates on some cold-target invocations, but warmed-target engine/provider checks and the targeted tests above completed successfully. It remains a host/toolchain stability concern, not an unresolved render-source diagnostic.

### Deployment boundary

The installed GameReady runtime remains untouched and bit-identical to the pre-audit baseline:

- size: `48,959,356` bytes
- SHA-256: `b68a5d3e4d7f047b76f525f9bf086b42b8994970aadda50ca5c61eb80b4bb2c4`

No production recipe activation or runtime DLL publication has been performed.

### Remaining gate before any production activation

The next step is an isolated runtime smoke using staging binaries/configuration with all production-installed artifacts preserved. That smoke must verify actual Vulkan pipeline creation, descriptor binding, `VisibilityCull` execution, indirect-count draws, GBuffer migrated-slot suppression, per-cascade shadow migrated-slot suppression, fail-open fallback behavior and zero validation errors. Only after that should Sanctuary dense-scene p95/p99/1%/0.1% benchmark work begin. Production defaults and installed GameReady must remain unchanged until those gates pass.