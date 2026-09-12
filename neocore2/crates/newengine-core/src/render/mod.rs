use crate::error::{EngineError, EngineResult};
use crate::module::{ApiProvide, ApiVersion};
use parking_lot::{Mutex, MutexGuard};
use std::sync::Arc;

pub use newengine_render_api::*;

pub const RENDER_API_ID: &str = newengine_render_api::RENDER_SERVICE_ID;
pub const RENDER_API_VERSION: ApiVersion = ApiVersion::new(1, 0, 0);
pub const RENDER_API_PROVIDE: ApiProvide = ApiProvide::new(RENDER_API_ID, RENDER_API_VERSION);

/// Coarse CPU timings for one render-controller module frame.
///
/// Kept in `newengine-core::render` so both the renderer-facing runtime module
/// and platform/profiler host can exchange timing data without a crate cycle.
#[derive(Debug, Clone, Default)]
pub struct RenderModuleTimingTelemetry {
    pub frame_index: u64,
    pub total_ms: f64,
    pub pre_begin_ms: f64,
    pub backend_begin_ms: f64,
    pub playable_frame_ms: f64,
    pub diagnostics_before_present_ms: f64,
    pub backend_end_ms: f64,
    pub backend_reported_begin_ms: f32,
    pub backend_frame_slot_wait_ms: f32,
    pub backend_surface_acquire_ms: f32,
    pub backend_image_wait_ms: f32,
    pub backend_reported_end_ms: f32,
    pub backend_gpu_timestamps_enabled: bool,
    pub backend_gpu_timing_frame_index: u64,
    pub backend_gpu_shadow_ms: f32,
    pub backend_gpu_opaque_ms: f32,
    pub backend_gpu_postfx_ms: f32,
    pub backend_gpu_ui_ms: f32,
    pub backend_gpu_profiled_ms: f32,
}

#[derive(Debug, Clone, Default)]
pub struct RenderBackendStatus {
    pub degraded: bool,
    pub phase: Option<&'static str>,
    pub message: Option<String>,
}

impl RenderBackendStatus {
    #[inline]
    pub fn healthy() -> Self {
        Self::default()
    }

    #[inline]
    pub fn degraded(phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            degraded: true,
            phase: Some(phase),
            message: Some(message.into()),
        }
    }
}

/// Runtime-hosted launch/loading status for scenes that must warm resources
/// before the platform hands visual ownership to the playable world.
///
/// This resource is intentionally hosted in `newengine-core`, not in the game
/// runtime crate, so the platform host can publish engine.ui loading state
/// without depending on a concrete scene/game module.
#[derive(Debug, Clone)]
pub struct SceneLaunchStatus {
    pub active: bool,
    pub title: String,
    pub status: String,
    pub detail: String,
    pub progress_01: f32,
}

impl SceneLaunchStatus {
    #[inline]
    pub fn loading(
        title: impl Into<String>,
        status: impl Into<String>,
        detail: impl Into<String>,
        progress_01: f32,
    ) -> Self {
        Self {
            active: true,
            title: title.into(),
            status: status.into(),
            detail: detail.into(),
            progress_01: progress_01.clamp(0.0, 0.995),
        }
    }

    #[inline]
    pub fn inactive() -> Self {
        Self {
            active: false,
            title: String::new(),
            status: String::new(),
            detail: String::new(),
            progress_01: 1.0,
        }
    }
}

pub trait RenderApi: Send {
    fn begin_frame(&mut self, desc: BeginFrameDesc) -> EngineResult<()>;
    /// Sets a tiny engine-runtime debug overlay string. This is deliberately not
    /// game-side UI: applications publish metrics, the renderer owns drawing.
    #[inline]
    fn set_debug_text(&mut self, _text: String) {}

    fn end_frame(&mut self) -> EngineResult<()>;

    /// Aborts the currently opened backend frame without queue submission.
    ///
    /// The default implementation preserves compatibility for in-process/null
    /// backends that only record CPU commands. Native backends must override this
    /// when begin_frame acquires presentation or synchronization resources.
    #[inline]
    fn abort_frame(&mut self) -> EngineResult<()> {
        self.discard_recorded_commands()
    }

    fn resize(&mut self, width: u32, height: u32) -> EngineResult<()>;

    fn create_render_target(&mut self, desc: RenderTargetDesc) -> EngineResult<RenderTargetId>;
    fn destroy_render_target(&mut self, id: RenderTargetId);
    fn render_target_ui_tex_id(&self, id: RenderTargetId) -> EngineResult<UiTexId>;
    fn render_target_color_texture_id(&self, id: RenderTargetId) -> EngineResult<TextureId>;

    fn begin_render_target(&mut self, desc: BeginRenderTargetDesc) -> EngineResult<()>;
    fn end_render_target(&mut self) -> EngineResult<()>;

    fn create_buffer(&mut self, desc: BufferDesc) -> EngineResult<BufferId>;
    fn destroy_buffer(&mut self, id: BufferId);
    fn write_buffer(&mut self, id: BufferId, offset: u64, data: &[u8]) -> EngineResult<()>;

    fn create_texture(&mut self, desc: TextureDesc) -> EngineResult<TextureId>;
    fn destroy_texture(&mut self, id: TextureId);

    fn create_sampler(&mut self, desc: SamplerDesc) -> EngineResult<SamplerId>;
    fn destroy_sampler(&mut self, id: SamplerId);

    fn create_shader(&mut self, desc: ShaderDesc) -> EngineResult<ShaderId>;
    fn destroy_shader(&mut self, id: ShaderId);

    fn create_pipeline(&mut self, desc: PipelineDesc) -> EngineResult<PipelineId>;
    fn create_compute_pipeline(&mut self, desc: ComputePipelineDesc) -> EngineResult<PipelineId>;
    fn destroy_pipeline(&mut self, id: PipelineId);

    fn create_bind_group_layout(
        &mut self,
        desc: BindGroupLayoutDesc,
    ) -> EngineResult<BindGroupLayoutId>;
    fn destroy_bind_group_layout(&mut self, id: BindGroupLayoutId);

    fn create_bind_group(&mut self, desc: BindGroupDesc) -> EngineResult<BindGroupId>;
    fn destroy_bind_group(&mut self, id: BindGroupId);

    fn set_viewport(&mut self, vp: Viewport) -> EngineResult<()>;
    fn set_scissor(&mut self, rect: RectI32) -> EngineResult<()>;

    fn set_pipeline(&mut self, pipeline: PipelineId) -> EngineResult<()>;
    fn set_bind_group(&mut self, index: u32, group: BindGroupId) -> EngineResult<()>;

    fn set_vertex_buffer(&mut self, slot: u32, slice: BufferSlice) -> EngineResult<()>;
    fn set_index_buffer(&mut self, slice: BufferSlice, format: IndexFormat) -> EngineResult<()>;

    fn draw(&mut self, args: DrawArgs) -> EngineResult<()>;
    fn draw_indexed(&mut self, args: DrawIndexedArgs) -> EngineResult<()>;

    #[inline]
    fn draw_indexed_indirect(&mut self, _args: DrawIndexedIndirectArgs) -> EngineResult<()> {
        Err(EngineError::other(
            "render backend does not support indexed indirect draws",
        ))
    }

    #[inline]
    fn draw_indexed_indirect_count(
        &mut self,
        _args: DrawIndexedIndirectCountArgs,
    ) -> EngineResult<()> {
        Err(EngineError::other(
            "render backend does not support indexed indirect-count draws",
        ))
    }

    #[inline]
    fn dispatch_visibility_indirect_cull(
        &mut self,
        _args: GpuVisibilityIndirectCullArgs,
    ) -> EngineResult<()> {
        Err(EngineError::other(
            "render backend does not support GPU visibility indirect culling",
        ))
    }

    #[inline]
    fn dispatch_visibility_indirect_compact_v2(
        &mut self,
        _args: GpuVisibilityIndirectCompactArgsV2,
    ) -> EngineResult<()> {
        Err(EngineError::other(
            "render backend does not support V2 GPU visibility indirect compaction",
        ))
    }
    fn dispatch(&mut self, args: DispatchArgs) -> EngineResult<()>;

    /// Selects the render graph phase that subsequent recorded commands belong to.
    /// Backends may use this to route command recording into phase buckets.
    #[inline]
    fn set_render_phase(&mut self, _phase: Option<RenderGraphPassKind>) -> EngineResult<()> {
        Ok(())
    }

    #[inline]
    fn begin_render_phase(&mut self, phase: RenderGraphPassKind) -> EngineResult<()> {
        self.set_render_phase(Some(phase))
    }

    #[inline]
    fn end_render_phase(&mut self) -> EngineResult<()> {
        self.set_render_phase(None)
    }

    /// Selects a typed draw-list route. Callers describe what they are recording,
    /// while the backend maps the list to the current graph pass.
    #[inline]
    fn set_draw_list_kind(&mut self, kind: Option<RenderDrawListKind>) -> EngineResult<()> {
        self.set_render_phase(kind.map(RenderDrawListKind::default_pass_kind))
    }

    #[inline]
    fn begin_draw_list(&mut self, kind: RenderDrawListKind) -> EngineResult<()> {
        self.set_draw_list_kind(Some(kind))
    }

    #[inline]
    fn end_draw_list(&mut self) -> EngineResult<()> {
        self.set_draw_list_kind(None)
    }

    /// Drops phase-recorded commands for the current frame.
    /// This is a safety valve for graph-submit failure paths; normal frames
    /// execute recorded phase buckets through submit_frame().
    #[inline]
    fn discard_recorded_commands(&mut self) -> EngineResult<()> {
        Ok(())
    }

    /// Compiles a declarative render frame graph without executing it.
    #[inline]
    fn compile_render_graph(
        &mut self,
        graph: RenderGraphDesc,
    ) -> EngineResult<RenderGraphCompileReport> {
        newengine_render_api::compile_render_graph(&graph).map_err(|errors| {
            EngineError::other(format!("render graph validation failed: {:?}", errors))
        })
    }

    /// Validates a declarative render frame graph and returns detailed diagnostics.
    #[inline]
    fn validate_render_graph(
        &mut self,
        graph: RenderGraphDesc,
    ) -> EngineResult<RenderGraphValidationReport> {
        Ok(newengine_render_api::validate_and_compile_render_graph(
            &graph,
        ))
    }

    /// Submits a declarative render frame graph. Prefer submit_frame() at runtime
    /// so the backend receives the graph, draw-list declarations and frame extents
    /// as one stable package.
    #[inline]
    fn submit_render_graph(
        &mut self,
        graph: RenderGraphDesc,
    ) -> EngineResult<RenderGraphSubmitReport> {
        let started = std::time::Instant::now();
        let compile = self.compile_render_graph(graph)?;
        Ok(RenderGraphSubmitReport {
            cpu_record_ms: started.elapsed().as_secs_f32() * 1000.0,
            gpu_submit_ms: 0.0,
            executed_passes: 0,
            skipped_passes: compile.pass_count,
            backend_deferred: false,
            uploads: UploadPumpReport::default(),
            compile,
            draw_list_stats: Vec::new(),
        })
    }

    /// Submits one complete renderer-facing frame package. This is the stable
    /// runtime/backend boundary: frame metadata, graph, draw-list routes and
    /// optional work budget travel together.
    #[inline]
    fn submit_frame(
        &mut self,
        frame: RenderFrameEnvelope,
    ) -> EngineResult<RenderGraphSubmitReport> {
        if let Some(budget) = frame.work_budget {
            self.set_work_budget(budget)?;
        }
        self.submit_render_graph(frame.graph)
    }

    /// Applies a backend-neutral frame work budget. Backends may use it to avoid
    /// long blocking uploads/pipeline builds inside interactive frames.
    #[inline]
    fn set_work_budget(&mut self, _budget: RenderWorkBudget) -> EngineResult<()> {
        Ok(())
    }

    /// Pumps queued GPU upload jobs using a backend-neutral budget. Backends that
    /// support staged uploads should do bounded work here instead of blocking
    /// arbitrary draw-call paths.
    #[inline]
    fn pump_uploads(&mut self, _desc: UploadPumpDesc) -> EngineResult<UploadPumpReport> {
        Ok(UploadPumpReport::default())
    }

    /// Returns the residency state of a texture. This lets material systems bind
    /// fallbacks while the real texture is still being staged/uploaded.
    #[inline]
    fn texture_residency(&self, id: TextureId) -> EngineResult<TextureResidencySnapshot> {
        Ok(TextureResidencySnapshot::ready(id, None))
    }

    /// Warms up render pipelines before the first playable frame. The default
    /// path creates them synchronously; backend implementations may use native
    /// pipeline caches and stricter loading-screen budgets.
    fn warmup_pipelines(&mut self, desc: PipelineWarmupDesc) -> EngineResult<PipelineWarmupReport> {
        let started = std::time::Instant::now();
        let requested = desc.pipelines.len() as u32;
        let mut report = PipelineWarmupReport {
            requested,
            ..PipelineWarmupReport::default()
        };
        for pipeline in desc.pipelines {
            match self.create_pipeline(pipeline) {
                Ok(id) => report.created.push(id),
                Err(_) => report.failed = report.failed.saturating_add(1),
            }
        }
        report.elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
        Ok(report)
    }

    #[inline]
    fn shader_cache_stats(&self) -> EngineResult<ShaderRuntimeCacheStats> {
        Ok(ShaderRuntimeCacheStats::default())
    }

    /// Drains renderer-owned lifecycle events such as fence-completed frames.
    ///
    /// Resource lifetime must be driven from this stream, not from guessed frame
    /// grace windows. Service-backed renderers bridge this to render.api; in-process
    /// renderers may publish directly.
    #[inline]
    fn drain_backend_events(&mut self) -> EngineResult<Vec<RenderBackendEvent>> {
        Ok(Vec::new())
    }

    /// Returns a backend-neutral diagnostics snapshot for frame pacing, upload
    /// queues and live GPU resource counts.
    #[inline]
    fn diagnostics_snapshot(&self) -> EngineResult<RenderDiagnosticsSnapshot> {
        Ok(RenderDiagnosticsSnapshot::default())
    }
}

#[derive(Clone)]
pub struct RenderApiRef(Arc<Mutex<Box<dyn RenderApi + 'static>>>);

impl RenderApiRef {
    #[inline]
    pub fn new(api: impl RenderApi + 'static) -> Self {
        Self::from_box(Box::new(api))
    }

    #[inline]
    pub fn from_box(api: Box<dyn RenderApi + 'static>) -> Self {
        Self(Arc::new(Mutex::new(api)))
    }

    #[inline]
    pub fn lock(&self) -> MutexGuard<'_, Box<dyn RenderApi + 'static>> {
        self.0.lock()
    }
}

#[inline]
pub fn require_render_api<'a, E: Send + 'static>(
    ctx: &'a crate::module::ModuleCtx<'_, E>,
) -> EngineResult<&'a RenderApiRef> {
    ctx.api_required::<RenderApiRef>(RENDER_API_ID)
        .map_err(|_| {
            EngineError::other("Render API is not available (missing render backend module?)")
        })
}
