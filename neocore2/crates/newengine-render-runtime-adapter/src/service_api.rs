use std::sync::Mutex;

use newengine_core::render::{
    BeginFrameDesc, BeginRenderTargetDesc, BindGroupDesc, BindGroupId, BindGroupLayoutDesc,
    BindGroupLayoutId, BufferDesc, BufferId, BufferSlice, ComputePipelineDesc, DispatchArgs,
    DrawArgs, DrawIndexedArgs, DrawIndexedIndirectArgs, DrawIndexedIndirectCountArgs,
    GpuVisibilityIndirectCullArgs, IndexFormat, PipelineDesc, PipelineId, PipelineWarmupDesc,
    PipelineWarmupReport, RectI32, RenderApi, RenderBackendEvent, RenderDiagnosticsSnapshot,
    RenderDrawListKind, RenderFrameEnvelope, RenderGraphCompileReport, RenderGraphDesc,
    RenderGraphPassKind, RenderGraphSubmitReport, RenderGraphValidationReport, RenderTargetDesc,
    RenderTargetId, RenderWorkBudget, SamplerDesc, SamplerId, ShaderDesc, ShaderId,
    ShaderRuntimeCacheStats, TextureDesc, TextureId, TextureResidencySnapshot, UiTexId,
    UploadPumpDesc, UploadPumpReport, Viewport,
};
use newengine_core::{EngineError, EngineResult};
use newengine_render_api::{
    RenderCommand, RenderCommandResponse, RenderServiceRequest, RenderServiceResponse,
};

use crate::client::RenderServiceClient;

pub(crate) struct ServiceBackedRenderApi {
    client: RenderServiceClient,
    pending_unit_commands: Mutex<Vec<RenderCommand>>,
}

impl ServiceBackedRenderApi {
    #[inline]
    pub(crate) fn new(client: RenderServiceClient) -> Self {
        Self {
            client,
            pending_unit_commands: Mutex::new(Vec::with_capacity(128)),
        }
    }

    #[inline]
    fn pending(&self) -> std::sync::MutexGuard<'_, Vec<RenderCommand>> {
        match self.pending_unit_commands.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    #[inline]
    fn queue_unit(&self, req: RenderCommand) -> EngineResult<()> {
        self.pending().push(req);
        Ok(())
    }

    fn flush_unit_batch(&self) -> EngineResult<()> {
        let pending = {
            let mut guard = self.pending();
            if guard.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *guard)
        };

        let responses = self
            .client
            .command_batch(pending)
            .map_err(EngineError::other)?;
        for response in responses {
            match response {
                RenderCommandResponse::Unit => {}
                other => {
                    return Err(EngineError::other(format!(
                        "render service protocol error: expected unit response in command batch, got {:?}",
                        other
                    )));
                }
            }
        }
        Ok(())
    }

    #[inline]
    fn unit(&self, req: RenderCommand) -> EngineResult<()> {
        match self.command(req)? {
            RenderCommandResponse::Unit => Ok(()),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected unit response, got {:?}",
                other
            ))),
        }
    }

    #[inline]
    fn command(&self, req: RenderCommand) -> EngineResult<RenderCommandResponse> {
        self.flush_unit_batch()?;
        self.client.command(req).map_err(EngineError::other)
    }

    #[inline]
    fn invoke_service(&self, req: RenderServiceRequest) -> EngineResult<RenderServiceResponse> {
        self.flush_unit_batch()?;
        match self.client.invoke(req).map_err(EngineError::other)? {
            RenderServiceResponse::Problem(problem) => Err(EngineError::other(format!(
                "render service problem {} at {:?}: {}",
                problem.code, problem.phase, problem.detail
            ))),
            response => Ok(response),
        }
    }
}

impl RenderApi for ServiceBackedRenderApi {
    fn begin_frame(&mut self, desc: BeginFrameDesc) -> EngineResult<()> {
        self.unit(RenderCommand::BeginFrame(desc))
    }

    fn set_debug_text(&mut self, text: String) {
        if !text.trim().is_empty() {
            newengine_ulog_api::ulog::warn!("engine.render: SetDebugText ignored; UI presentation must be published through engine.ui");
        }
    }

    fn end_frame(&mut self) -> EngineResult<()> {
        self.unit(RenderCommand::EndFrame)
    }

    fn abort_frame(&mut self) -> EngineResult<()> {
        {
            let mut pending = self.pending();
            pending.clear();
        }
        match self.invoke_service(RenderServiceRequest::AbortFrame)? {
            RenderServiceResponse::Unit => Ok(()),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected Unit, got {:?}",
                other
            ))),
        }
    }

    fn resize(&mut self, width: u32, height: u32) -> EngineResult<()> {
        self.unit(RenderCommand::Resize { width, height })
    }

    fn create_render_target(&mut self, desc: RenderTargetDesc) -> EngineResult<RenderTargetId> {
        match self.command(RenderCommand::CreateRenderTarget(desc))? {
            RenderCommandResponse::RenderTargetId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected RenderTargetId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_render_target(&mut self, id: RenderTargetId) {
        let _ = self.unit(RenderCommand::DestroyRenderTarget { id });
    }

    fn render_target_ui_tex_id(&self, id: RenderTargetId) -> EngineResult<UiTexId> {
        match self.command(RenderCommand::RenderTargetUiTexId { id })? {
            RenderCommandResponse::UiTexId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected UiTexId, got {:?}",
                other
            ))),
        }
    }

    fn render_target_color_texture_id(&self, id: RenderTargetId) -> EngineResult<TextureId> {
        match self.command(RenderCommand::RenderTargetColorTextureId { id })? {
            RenderCommandResponse::TextureId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected TextureId, got {:?}",
                other
            ))),
        }
    }

    fn begin_render_target(&mut self, desc: BeginRenderTargetDesc) -> EngineResult<()> {
        self.unit(RenderCommand::BeginRenderTarget(desc))
    }

    fn end_render_target(&mut self) -> EngineResult<()> {
        self.unit(RenderCommand::EndRenderTarget)
    }

    fn create_buffer(&mut self, desc: BufferDesc) -> EngineResult<BufferId> {
        match self.command(RenderCommand::CreateBuffer(desc))? {
            RenderCommandResponse::BufferId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected BufferId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_buffer(&mut self, id: BufferId) {
        let _ = self.unit(RenderCommand::DestroyBuffer { id });
    }

    fn write_buffer(&mut self, id: BufferId, offset: u64, data: &[u8]) -> EngineResult<()> {
        self.queue_unit(RenderCommand::WriteBuffer {
            id,
            offset,
            data: data.to_vec(),
        })
    }

    fn create_texture(&mut self, desc: TextureDesc) -> EngineResult<TextureId> {
        self.flush_unit_batch()?;
        self.client.create_texture(desc).map_err(EngineError::other)
    }

    fn destroy_texture(&mut self, id: TextureId) {
        let _ = self.unit(RenderCommand::DestroyTexture { id });
    }

    fn create_sampler(&mut self, desc: SamplerDesc) -> EngineResult<SamplerId> {
        match self.command(RenderCommand::CreateSampler(desc))? {
            RenderCommandResponse::SamplerId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected SamplerId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_sampler(&mut self, id: SamplerId) {
        let _ = self.unit(RenderCommand::DestroySampler { id });
    }

    fn create_shader(&mut self, desc: ShaderDesc) -> EngineResult<ShaderId> {
        match self.command(RenderCommand::CreateShader(desc))? {
            RenderCommandResponse::ShaderId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected ShaderId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_shader(&mut self, id: ShaderId) {
        let _ = self.unit(RenderCommand::DestroyShader { id });
    }

    fn create_pipeline(&mut self, desc: PipelineDesc) -> EngineResult<PipelineId> {
        match self.command(RenderCommand::CreatePipeline(desc))? {
            RenderCommandResponse::PipelineId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected PipelineId, got {:?}",
                other
            ))),
        }
    }

    fn create_compute_pipeline(&mut self, desc: ComputePipelineDesc) -> EngineResult<PipelineId> {
        match self.command(RenderCommand::CreateComputePipeline(desc))? {
            RenderCommandResponse::PipelineId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected PipelineId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_pipeline(&mut self, id: PipelineId) {
        let _ = self.unit(RenderCommand::DestroyPipeline { id });
    }

    fn create_bind_group_layout(
        &mut self,
        desc: BindGroupLayoutDesc,
    ) -> EngineResult<BindGroupLayoutId> {
        match self.command(RenderCommand::CreateBindGroupLayout(desc))? {
            RenderCommandResponse::BindGroupLayoutId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected BindGroupLayoutId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_bind_group_layout(&mut self, id: BindGroupLayoutId) {
        let _ = self.unit(RenderCommand::DestroyBindGroupLayout { id });
    }

    fn create_bind_group(&mut self, desc: BindGroupDesc) -> EngineResult<BindGroupId> {
        match self.command(RenderCommand::CreateBindGroup(desc))? {
            RenderCommandResponse::BindGroupId(id) => Ok(id),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected BindGroupId, got {:?}",
                other
            ))),
        }
    }

    fn destroy_bind_group(&mut self, id: BindGroupId) {
        let _ = self.unit(RenderCommand::DestroyBindGroup { id });
    }

    fn set_viewport(&mut self, vp: Viewport) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetViewport(vp))
    }

    fn set_scissor(&mut self, rect: RectI32) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetScissor(rect))
    }

    fn set_pipeline(&mut self, pipeline: PipelineId) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetPipeline { pipeline })
    }

    fn set_bind_group(&mut self, index: u32, group: BindGroupId) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetBindGroup { index, group })
    }

    fn set_vertex_buffer(&mut self, slot: u32, slice: BufferSlice) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetVertexBuffer { slot, slice })
    }

    fn set_index_buffer(&mut self, slice: BufferSlice, format: IndexFormat) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetIndexBuffer { slice, format })
    }

    fn draw(&mut self, args: DrawArgs) -> EngineResult<()> {
        self.queue_unit(RenderCommand::Draw(args))
    }

    fn draw_indexed(&mut self, args: DrawIndexedArgs) -> EngineResult<()> {
        self.queue_unit(RenderCommand::DrawIndexed(args))
    }

    fn draw_indexed_indirect(&mut self, args: DrawIndexedIndirectArgs) -> EngineResult<()> {
        self.queue_unit(RenderCommand::DrawIndexedIndirect(args))
    }

    fn draw_indexed_indirect_count(
        &mut self,
        args: DrawIndexedIndirectCountArgs,
    ) -> EngineResult<()> {
        self.queue_unit(RenderCommand::DrawIndexedIndirectCount(args))
    }

    fn dispatch_visibility_indirect_cull(
        &mut self,
        args: GpuVisibilityIndirectCullArgs,
    ) -> EngineResult<()> {
        self.queue_unit(RenderCommand::DispatchVisibilityIndirectCull(args))
    }

    fn dispatch(&mut self, args: DispatchArgs) -> EngineResult<()> {
        self.queue_unit(RenderCommand::Dispatch(args))
    }

    fn set_render_phase(&mut self, phase: Option<RenderGraphPassKind>) -> EngineResult<()> {
        // Render phase/draw-list switches are hot-path recording commands.
        // Queue them with draws so a provider receives one ordered binary batch
        // instead of a JSON gateway round-trip for every begin/end draw list.
        self.queue_unit(RenderCommand::SetRenderPhase { phase })
    }

    fn set_draw_list_kind(&mut self, kind: Option<RenderDrawListKind>) -> EngineResult<()> {
        self.queue_unit(RenderCommand::SetDrawListKind { kind })
    }

    fn discard_recorded_commands(&mut self) -> EngineResult<()> {
        {
            let mut pending = self.pending();
            pending.clear();
        }
        match self.invoke_service(RenderServiceRequest::DiscardRecordedCommands)? {
            RenderServiceResponse::Unit => Ok(()),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected Unit, got {:?}",
                other
            ))),
        }
    }

    fn compile_render_graph(
        &mut self,
        graph: RenderGraphDesc,
    ) -> EngineResult<RenderGraphCompileReport> {
        match self.invoke_service(RenderServiceRequest::CompileRenderGraph(graph))? {
            RenderServiceResponse::GraphCompileReport(report) => Ok(report),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected GraphCompileReport, got {:?}",
                other
            ))),
        }
    }

    fn validate_render_graph(
        &mut self,
        graph: RenderGraphDesc,
    ) -> EngineResult<RenderGraphValidationReport> {
        match self.invoke_service(RenderServiceRequest::ValidateRenderGraph(graph))? {
            RenderServiceResponse::GraphValidationReport(report) => Ok(report),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected GraphValidationReport, got {:?}",
                other
            ))),
        }
    }

    fn submit_render_graph(
        &mut self,
        graph: RenderGraphDesc,
    ) -> EngineResult<RenderGraphSubmitReport> {
        match self.invoke_service(RenderServiceRequest::SubmitRenderGraph(graph))? {
            RenderServiceResponse::GraphSubmitReport(report) => Ok(report),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected GraphSubmitReport, got {:?}",
                other
            ))),
        }
    }

    fn submit_frame(
        &mut self,
        frame: RenderFrameEnvelope,
    ) -> EngineResult<RenderGraphSubmitReport> {
        let trace_ui_only =
            frame.label.as_deref() == Some("ui_layer_only") && frame.frame_index <= 2;
        if trace_ui_only {
            newengine_ulog_api::ulog::info!(
                "render service adapter: submit_frame begin frame={} graph_passes={} ui_packets={} payload_mesh_vertices={} payload_mesh_indices={}",
                frame.frame_index,
                frame.graph.passes.len(),
                frame.ui_layers.packets.len(),
                frame.ui_layers.packets.iter().map(|packet| packet.draw_list.mesh.vertices.len()).sum::<usize>(),
                frame.ui_layers.packets.iter().map(|packet| packet.draw_list.mesh.indices.len()).sum::<usize>(),
            );
        }
        match self.invoke_service(RenderServiceRequest::SubmitFrame(Box::new(frame)))? {
            RenderServiceResponse::GraphSubmitReport(report) => {
                if trace_ui_only {
                    newengine_ulog_api::ulog::info!(
                        "render service adapter: submit_frame complete executed_passes={} skipped_passes={} compile_passes={} execution_order={}",
                        report.executed_passes,
                        report.skipped_passes,
                        report.compile.pass_count,
                        report.compile.execution_order.len(),
                    );
                }
                Ok(report)
            }
            other => Err(EngineError::other(format!(
                "render service protocol error: expected GraphSubmitReport, got {:?}",
                other
            ))),
        }
    }

    fn set_work_budget(&mut self, budget: RenderWorkBudget) -> EngineResult<()> {
        match self.invoke_service(RenderServiceRequest::SetWorkBudget(budget))? {
            RenderServiceResponse::Unit => Ok(()),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected Unit, got {:?}",
                other
            ))),
        }
    }

    fn pump_uploads(&mut self, desc: UploadPumpDesc) -> EngineResult<UploadPumpReport> {
        match self.invoke_service(RenderServiceRequest::PumpUploads(desc))? {
            RenderServiceResponse::UploadPumpReport(report) => Ok(report),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected UploadPumpReport, got {:?}",
                other
            ))),
        }
    }

    fn texture_residency(&self, id: TextureId) -> EngineResult<TextureResidencySnapshot> {
        match self.command(RenderCommand::TextureResidency { id })? {
            RenderCommandResponse::TextureResidency(snapshot) => Ok(snapshot),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected TextureResidency, got {:?}",
                other
            ))),
        }
    }

    fn warmup_pipelines(&mut self, desc: PipelineWarmupDesc) -> EngineResult<PipelineWarmupReport> {
        match self.command(RenderCommand::WarmupPipelines(desc))? {
            RenderCommandResponse::PipelineWarmupReport(report) => Ok(report),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected PipelineWarmupReport, got {:?}",
                other
            ))),
        }
    }

    fn shader_cache_stats(&self) -> EngineResult<ShaderRuntimeCacheStats> {
        match self.command(RenderCommand::ShaderCacheStats)? {
            RenderCommandResponse::ShaderCacheStats(stats) => Ok(stats),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected ShaderCacheStats, got {:?}",
                other
            ))),
        }
    }

    fn drain_backend_events(&mut self) -> EngineResult<Vec<RenderBackendEvent>> {
        match self.invoke_service(RenderServiceRequest::DrainBackendEvents)? {
            RenderServiceResponse::BackendEvents(events) => Ok(events),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected BackendEvents, got {:?}",
                other
            ))),
        }
    }

    fn diagnostics_snapshot(&self) -> EngineResult<RenderDiagnosticsSnapshot> {
        match self.invoke_service(RenderServiceRequest::DiagnosticsSnapshot)? {
            RenderServiceResponse::DiagnosticsSnapshot(snapshot) => Ok(*snapshot),
            other => Err(EngineError::other(format!(
                "render service protocol error: expected DiagnosticsSnapshot, got {:?}",
                other
            ))),
        }
    }
}
