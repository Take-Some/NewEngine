#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_core::render::{
    BindGroupDesc, BindGroupId, BindGroupLayoutDesc, BindGroupLayoutId, BindingKind,
    BufferBinding, BufferId, BufferSlice, DrawIndexedIndirectCountArgs, IndexFormat,
    PipelineDepthCompare, PipelineDepthMode, PipelineDesc, PipelineId, PrimitiveTopology,
    RasterCullMode, RectI32, RenderApi, RenderGraphPassKind, ShaderDesc, ShaderId,
    ShaderSourceKind, ShaderStage, TextureFormat, VertexAttribute, VertexFormat, VertexLayout,
    Viewport,
};
use newengine_core::EngineResult;
use newengine_material_domain_api::LitPipeline;
use newengine_primitives::PrimitiveVertex;
use newengine_render_feature_api::PackedLights;

use super::gpu_scene_table_upload::GpuSceneTableBuffers;
use crate::render_controller::resource_lifetime::RenderGpuLifetimeQueue;

const FRAME_SLOTS: usize = 8;
const VERTEX_SHADER: &str = "shaders/deferred/gpu_driven_gbuffer.vert";
const FRAGMENT_SHADER: &str = "shaders/deferred/gpu_driven_gbuffer.frag";
const FRAME_UBO_CACHE_KEY: u64 = 0x4750_5544_5249_5645;

#[derive(Clone, Copy, Debug)]
struct TableBindGroupSlot {
    object_buffer: BufferId,
    material_buffer: BufferId,
    group: BindGroupId,
}

#[derive(Debug)]
pub(in crate::render_controller) struct GpuDrivenGbufferState {
    table_layout: Option<BindGroupLayoutId>,
    vertex_shader: Option<ShaderId>,
    fragment_shader: Option<ShaderId>,
    pipeline: Option<PipelineId>,
    pipeline_lit_layout: Option<BindGroupLayoutId>,
    table_groups: [Option<TableBindGroupSlot>; FRAME_SLOTS],
    last_recorded_frame: u64,
}

impl Default for GpuDrivenGbufferState {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuDrivenGbufferState {
    pub(in crate::render_controller) fn new() -> Self {
        Self {
            table_layout: None,
            vertex_shader: None,
            fragment_shader: None,
            pipeline: None,
            pipeline_lit_layout: None,
            table_groups: [None; FRAME_SLOTS],
            last_recorded_frame: 0,
        }
    }

    #[inline]
    pub(in crate::render_controller) fn begin_frame(&mut self, frame_index: u64) {
        if self.last_recorded_frame != frame_index {
            self.last_recorded_frame = 0;
        }
    }

    #[inline]
    pub(in crate::render_controller) fn mark_recorded(&mut self, frame_index: u64) {
        self.last_recorded_frame = frame_index;
    }

    #[inline]
    pub(in crate::render_controller) fn recorded_this_frame(&self, frame_index: u64) -> bool {
        self.last_recorded_frame == frame_index && frame_index != 0
    }

    pub(in crate::render_controller) fn ensure_pipeline(
        &mut self,
        r: &mut dyn RenderApi,
        lit: LitPipeline,
    ) -> EngineResult<PipelineId> {
        if self.pipeline_lit_layout != Some(lit.bgl) {
            if let Some(pipeline) = self.pipeline.take() {
                r.destroy_pipeline(pipeline);
            }
            if let Some(shader) = self.vertex_shader.take() {
                r.destroy_shader(shader);
            }
            if let Some(shader) = self.fragment_shader.take() {
                r.destroy_shader(shader);
            }
            self.pipeline_lit_layout = Some(lit.bgl);
        }

        let table_layout = match self.table_layout {
            Some(layout) => layout,
            None => {
                let layout = r.create_bind_group_layout(
                    BindGroupLayoutDesc::new(vec![
                        BindingKind::StorageBuffer,
                        BindingKind::StorageBuffer,
                    ])
                    .with_label("gpu_driven.gbuffer.scene_tables"),
                )?;
                self.table_layout = Some(layout);
                layout
            }
        };

        let vs = match self.vertex_shader {
            Some(shader) => shader,
            None => {
                let shader = r.create_shader(
                    ShaderDesc::from_asset(
                        ShaderStage::Vertex,
                        "main",
                        VERTEX_SHADER,
                        ShaderSourceKind::Glsl,
                    )
                    .with_label("gpu_driven.gbuffer.vs"),
                )?;
                self.vertex_shader = Some(shader);
                shader
            }
        };
        let fs = match self.fragment_shader {
            Some(shader) => shader,
            None => {
                let shader = r.create_shader(
                    ShaderDesc::from_asset(
                        ShaderStage::Fragment,
                        "main",
                        FRAGMENT_SHADER,
                        ShaderSourceKind::Glsl,
                    )
                    .with_label("gpu_driven.gbuffer.fs"),
                )?;
                self.fragment_shader = Some(shader);
                shader
            }
        };

        if let Some(pipeline) = self.pipeline {
            return Ok(pipeline);
        }

        let primitive_layout = VertexLayout::new(
            std::mem::size_of::<PrimitiveVertex>() as u32,
            vec![
                VertexAttribute::new(0, 0, VertexFormat::Float32x3),
                VertexAttribute::new(1, 12, VertexFormat::Float32x3),
                VertexAttribute::new(2, 24, VertexFormat::Float32x2),
            ],
        );
        let pipeline = r.create_pipeline(
            PipelineDesc::new(vs, fs, TextureFormat::Rgba8Unorm)
                .with_label("gpu_driven.gbuffer.constants_only")
                .with_cache_key("gpu_driven.gbuffer.constants_only.v1")
                .with_topology(PrimitiveTopology::TriangleList)
                .with_vertex_layouts(vec![primitive_layout])
                .with_bind_group_layouts(vec![lit.bgl, table_layout])
                .with_color_formats(vec![
                    TextureFormat::Rgba8Unorm,
                    TextureFormat::Rgba16Float,
                    TextureFormat::Rgba8Unorm,
                ])
                .with_depth_state(
                    TextureFormat::Depth32Float,
                    PipelineDepthMode::new(true, true, PipelineDepthCompare::LessOrEqual),
                )
                .with_cull_mode(RasterCullMode::Back),
        )?;
        self.pipeline = Some(pipeline);
        Ok(pipeline)
    }

    pub(in crate::render_controller) fn ensure_table_group(
        &mut self,
        r: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        frame_index: u64,
        buffers: GpuSceneTableBuffers,
    ) -> EngineResult<BindGroupId> {
        let layout = self
            .table_layout
            .expect("GPU-driven GBuffer table layout must exist before bind-group creation");
        let ring_slot = buffers.ring_slot as usize;
        debug_assert!(ring_slot < FRAME_SLOTS);
        if let Some(slot) = self.table_groups[ring_slot] {
            if slot.object_buffer == buffers.objects && slot.material_buffer == buffers.materials {
                return Ok(slot.group);
            }
        }

        let object_bytes = u64::from(buffers.object_count)
            .saturating_mul(std::mem::size_of::<super::gpu_scene_tables::GpuObjectRecord>() as u64)
            .max(16);
        let material_bytes = u64::from(buffers.material_count)
            .saturating_mul(std::mem::size_of::<super::gpu_scene_tables::GpuMaterialRecord>() as u64)
            .max(16);
        let group = r.create_bind_group(
            BindGroupDesc::new(layout)
                .with_label(format!("gpu_driven.gbuffer.scene_tables.slot{ring_slot}"))
                .with_storage0(BufferBinding::new(buffers.objects, 0, object_bytes))
                .with_storage1(BufferBinding::new(buffers.materials, 0, material_bytes)),
        )?;
        if let Some(previous) = self.table_groups[ring_slot].replace(TableBindGroupSlot {
            object_buffer: buffers.objects,
            material_buffer: buffers.materials,
            group,
        }) {
            lifetime.retire_bind_group_after_frame(previous.group, frame_index);
        }
        Ok(group)
    }
}

impl super::RuntimeRenderController {
    pub(in crate::render_controller) fn record_gpu_indirect_gbuffer(
        &mut self,
        r: &mut dyn RenderApi,
        lit: LitPipeline,
        viewproj: newengine_math::Mat4,
        lights: &PackedLights,
        shadow_texture: newengine_core::render::TextureId,
        local_shadow_texture: newengine_core::render::TextureId,
        viewport_extent: newengine_core::render::Extent2D,
    ) -> EngineResult<bool> {
        let frame_index = self.frame.frame_index;
        self.gpu.indirect_gbuffer.begin_frame(frame_index);
        if !self.gpu_indirect_gbuffer_ready() {
            return Ok(false);
        }
        let Some(buffers) = self.gpu.table_buffers else {
            return Ok(false);
        };
        let Some(stream) = self.gpu.indirect_stream.current().cloned() else {
            return Ok(false);
        };
        if !stream.migration_ready() || stream.pages.is_empty() {
            return Ok(false);
        }

        let pipeline = self.gpu.indirect_gbuffer.ensure_pipeline(r, lit)?;
        let frame_bg = self.ensure_per_draw_ubo_with_binding(
            r,
            lit,
            FRAME_UBO_CACHE_KEY,
            lit.white_texture,
            lit.flat_normal_texture,
            lit.white_texture,
            shadow_texture,
            local_shadow_texture,
            lit.repeat_sampler,
        )?;
        super::passes_ubo::write_lit_ubo_ex(
            r,
            frame_bg.ubo,
            viewproj,
            newengine_math::Mat4::IDENTITY,
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            0.0,
            [1.0, 1.0, 0.0, 0.0],
            [1.0, 0.75, 0.0, 1.0],
            lights,
        )?;

        let table_group = {
            let gpu = &mut self.gpu;
            gpu.indirect_gbuffer.ensure_table_group(
                r,
                &mut gpu.lifetimes.resources,
                frame_index,
                buffers,
            )?
        };

        super::frame_submit::record_render_phase(r, RenderGraphPassKind::GBuffer, |r| {
            r.set_viewport(Viewport::full(viewport_extent))?;
            r.set_scissor(RectI32::new(
                0,
                0,
                viewport_extent.width as i32,
                viewport_extent.height as i32,
            ))?;
            r.set_pipeline(pipeline)?;
            r.set_bind_group(0, frame_bg.bg)?;
            r.set_bind_group(1, table_group)?;
            for page in &stream.pages {
                r.set_vertex_buffer(0, BufferSlice::new(page.vertex_buffer, 0))?;
                r.set_index_buffer(BufferSlice::new(page.index_buffer, 0), IndexFormat::U32)?;
                r.draw_indexed_indirect_count(DrawIndexedIndirectCountArgs::new(
                    page.output_indirect_buffer,
                    0,
                    page.count_buffer,
                    0,
                    page.draw_count,
                ))?;
            }
            Ok(())
        })?;

        self.gpu.indirect_gbuffer.mark_recorded(frame_index);
        Ok(true)
    }

    #[inline]
    pub(in crate::render_controller) fn gpu_indirect_entity_migrated(
        &self,
        entity_key: u64,
    ) -> bool {
        if !self
            .gpu
            .indirect_gbuffer
            .recorded_this_frame(self.frame.frame_index)
        {
            return false;
        }
        let Some(handle) = self.gpu.tables.object_handle(entity_key) else {
            return false;
        };
        self.gpu
            .indirect_stream
            .current()
            .is_some_and(|stream| stream.object_slots.binary_search(&handle.slot).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_vertex_layout_matches_gpu_driven_shader_contract() {
        assert_eq!(std::mem::size_of::<PrimitiveVertex>(), 32);
    }
}
