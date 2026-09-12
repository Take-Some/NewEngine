#![forbid(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;

use newengine_core::render::{
    BindGroupDesc, BindGroupId, BindGroupLayoutDesc, BindGroupLayoutId, BindingKind,
    BufferBinding, BufferDesc, BufferId, BufferSlice, BufferUsage, DrawIndexedIndirectCountArgs,
    IndexFormat, MemoryHint, PipelineDepthCompare, PipelineDepthMode, PipelineDesc, PipelineId,
    PrimitiveTopology, RasterCullMode, RenderApi, ShaderDesc, ShaderId, ShaderSourceKind,
    ShaderStage, TextureFormat, VertexAttribute, VertexFormat, VertexLayout,
};
use newengine_core::{EngineError, EngineResult};
use newengine_material_domain_api::LitPipeline;
use newengine_primitives::PrimitiveVertex;
use newengine_render_api::{gpu_indexed_indirect_commands_as_bytes, GpuDrawIndexedIndirectCommand};
use newengine_render_feature_api::{PackedLights, ShadowCasterCull};

use super::gpu_scene_table_upload::GpuSceneTableBuffers;
use super::gpu_scene_tables::GpuSceneTables;
use crate::render_controller::gpu::GeometryArena;
use crate::render_controller::resource_lifetime::RenderGpuLifetimeQueue;
use crate::render_controller::render_quality::SHADOW_MAP_COLOR_FORMAT;

const FRAME_SLOTS: usize = 8;
const MAX_COMMANDS_PER_PAGE: usize = 4096;
const MIN_BUFFER_BYTES: u64 = 4096;
const VERTEX_SHADER: &str = "shaders/gpu_driven_shadow.vert";
const FRAGMENT_SHADER: &str = "shaders/gpu_driven_shadow.frag";
const SHADOW_UBO_KEY_BASE: u64 = 0x4750_5553_4844_0000;

#[derive(Clone, Copy, Debug, Default)]
struct BufferSlot {
    id: Option<BufferId>,
    capacity_bytes: u64,
}

#[derive(Debug, Default)]
struct ShadowPageSlot {
    page: u32,
    commands: BufferSlot,
    count: BufferSlot,
}

#[derive(Debug, Default)]
struct ShadowCascadeSlot {
    cascade: usize,
    pages: Vec<ShadowPageSlot>,
}

#[derive(Debug, Default)]
struct ShadowFrameSlot {
    cascades: Vec<ShadowCascadeSlot>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::render_controller) struct ShadowIndirectPageStream {
    pub(in crate::render_controller) vertex_buffer: BufferId,
    pub(in crate::render_controller) index_buffer: BufferId,
    pub(in crate::render_controller) command_buffer: BufferId,
    pub(in crate::render_controller) count_buffer: BufferId,
    pub(in crate::render_controller) draw_count: u32,
}

#[derive(Clone, Debug, Default)]
pub(in crate::render_controller) struct ShadowIndirectStream {
    pub(in crate::render_controller) pages: Vec<ShadowIndirectPageStream>,
    pub(in crate::render_controller) entity_keys: Vec<u64>,
}

#[derive(Clone, Copy, Debug)]
struct ObjectBindGroupSlot {
    object_buffer: BufferId,
    group: BindGroupId,
}

#[derive(Debug)]
pub(in crate::render_controller) struct GpuShadowIndirectState {
    frame_slots: [ShadowFrameSlot; FRAME_SLOTS],
    table_layout: Option<BindGroupLayoutId>,
    vertex_shader: Option<ShaderId>,
    fragment_shader: Option<ShaderId>,
    pipeline: Option<PipelineId>,
    pipeline_lit_layout: Option<BindGroupLayoutId>,
    object_groups: [Option<ObjectBindGroupSlot>; FRAME_SLOTS],
    builds: u64,
    represented: u64,
    incomplete: u64,
    buffer_grows: u64,
}

impl Default for GpuShadowIndirectState {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuShadowIndirectState {
    pub(in crate::render_controller) fn new() -> Self {
        Self {
            frame_slots: std::array::from_fn(|_| ShadowFrameSlot::default()),
            table_layout: None,
            vertex_shader: None,
            fragment_shader: None,
            pipeline: None,
            pipeline_lit_layout: None,
            object_groups: [None; FRAME_SLOTS],
            builds: 0,
            represented: 0,
            incomplete: 0,
            buffer_grows: 0,
        }
    }

    fn ensure_pipeline(&mut self, r: &mut dyn RenderApi, lit: LitPipeline) -> EngineResult<PipelineId> {
        if self.pipeline_lit_layout.is_some() && self.pipeline_lit_layout != Some(lit.bgl) {
            return Err(EngineError::other(
                "gpu shadow indirect set=0 material layout changed; legacy shadow path remains authoritative",
            ));
        }
        self.pipeline_lit_layout = Some(lit.bgl);

        let table_layout = match self.table_layout {
            Some(layout) => layout,
            None => {
                let layout = r.create_bind_group_layout(
                    BindGroupLayoutDesc::new(vec![BindingKind::StorageBuffer])
                        .with_label("gpu_driven.shadow.object_table"),
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
                    .with_label("gpu_driven.shadow.vs"),
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
                    .with_label("gpu_driven.shadow.fs"),
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
            PipelineDesc::new(vs, fs, SHADOW_MAP_COLOR_FORMAT)
                .with_label("gpu_driven.shadow.opaque")
                .with_cache_key("gpu_driven.shadow.opaque.v1")
                .with_topology(PrimitiveTopology::TriangleList)
                .with_vertex_layouts(vec![primitive_layout])
                .with_bind_group_layouts(vec![lit.bgl, table_layout])
                .with_depth_state(
                    TextureFormat::Depth32Float,
                    PipelineDepthMode::new(true, true, PipelineDepthCompare::LessOrEqual),
                )
                .with_cull_mode(RasterCullMode::Back),
        )?;
        self.pipeline = Some(pipeline);
        Ok(pipeline)
    }

    fn ensure_object_group(
        &mut self,
        r: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        frame_index: u64,
        buffers: GpuSceneTableBuffers,
    ) -> EngineResult<BindGroupId> {
        let layout = self
            .table_layout
            .ok_or_else(|| EngineError::other("gpu shadow indirect object layout missing"))?;
        let ring_slot = buffers.ring_slot as usize;
        if ring_slot >= FRAME_SLOTS {
            return Err(EngineError::other(format!(
                "gpu shadow indirect table ring slot out of range slot={} max={}",
                ring_slot, FRAME_SLOTS,
            )));
        }
        if let Some(slot) = self.object_groups[ring_slot] {
            if slot.object_buffer == buffers.objects {
                return Ok(slot.group);
            }
        }
        let object_bytes = u64::from(buffers.object_count)
            .saturating_mul(std::mem::size_of::<super::gpu_scene_tables::GpuObjectRecord>() as u64)
            .max(16);
        let group = r.create_bind_group(
            BindGroupDesc::new(layout)
                .with_label(format!("gpu_driven.shadow.object_table.slot{ring_slot}"))
                .with_storage0(BufferBinding::new(buffers.objects, 0, object_bytes)),
        )?;
        if let Some(previous) = self.object_groups[ring_slot].replace(ObjectBindGroupSlot {
            object_buffer: buffers.objects,
            group,
        }) {
            lifetime.retire_bind_group_after_frame(previous.group, frame_index);
        }
        Ok(group)
    }

    fn build_stream(
        &mut self,
        r: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        frame_index: u64,
        cascade_index: usize,
        geometry: &GeometryArena,
        tables: &GpuSceneTables,
        admitted_entity_keys: &[u64],
        shadow_cull: Option<ShadowCasterCull>,
        cascade_texel_world_size: f32,
    ) -> EngineResult<Option<ShadowIndirectStream>> {
        #[derive(Default)]
        struct PagePlan {
            commands: Vec<GpuDrawIndexedIndirectCommand>,
        }

        let geometry_records = tables.geometry_records();
        let object_records = tables.object_records();
        let mut plans = BTreeMap::<u32, PagePlan>::new();
        let mut represented_keys = Vec::new();
        let mut eligible_count = 0usize;

        for entity_key in admitted_entity_keys.iter().copied() {
            let Some(handle) = tables.object_handle(entity_key) else {
                continue;
            };
            let Some(object) = object_records.get(handle.slot as usize) else {
                continue;
            };
            if !object.shadow_indirect_eligible() {
                continue;
            }
            let [geometry_slot, geometry_generation] = object.geometry_handle_lanes();
            let Some(record) = geometry_records.get(geometry_slot as usize) else {
                self.incomplete = self.incomplete.saturating_add(1);
                return Ok(None);
            };
            if record.meta[0] != geometry_generation || record.meta[1] == 0 {
                self.incomplete = self.incomplete.saturating_add(1);
                return Ok(None);
            }
            let page = record.meta[2];
            if geometry.page_buffers(page).is_none() {
                self.incomplete = self.incomplete.saturating_add(1);
                return Ok(None);
            }
            let model = newengine_math::Mat4::from_cols_array_2d(&object.model_cols);
            let local_center = newengine_math::Vec3::new(
                record.bounds[0],
                record.bounds[1],
                record.bounds[2],
            );
            let (center_ws, radius_ws) = super::passes::mesh_visibility::transform_sphere(
                model,
                local_center,
                record.bounds[3],
            );
            if !shadow_cull
                .map(|cull| cull.contains_sphere(center_ws, radius_ws))
                .unwrap_or(true)
            {
                continue;
            }
            if !super::passes::shadow_caster_projected_radius_visible(
                cascade_index,
                cascade_texel_world_size,
                radius_ws,
            ) {
                continue;
            }
            eligible_count = eligible_count.saturating_add(1);
            let plan = plans.entry(page).or_default();
            if plan.commands.len() >= MAX_COMMANDS_PER_PAGE {
                self.incomplete = self.incomplete.saturating_add(1);
                return Ok(None);
            }
            plan.commands.push(GpuDrawIndexedIndirectCommand {
                index_count: record.draw[1],
                instance_count: 1,
                first_index: record.draw[0],
                vertex_offset: record.draw[2] as i32,
                first_instance: handle.slot,
            });
            represented_keys.push(entity_key);
        }

        if eligible_count == 0 {
            return Ok(None);
        }
        if represented_keys.len() != eligible_count {
            self.incomplete = self.incomplete.saturating_add(1);
            return Ok(None);
        }

        let ring_slot = frame_index as usize % FRAME_SLOTS;
        let cascade_slot = cascade_slot(&mut self.frame_slots[ring_slot], cascade_index);
        let mut pages = Vec::with_capacity(plans.len());

        for (page, plan) in plans {
            if plan.commands.is_empty() {
                continue;
            }
            let (vertex_buffer, index_buffer) = geometry.page_buffers(page).ok_or_else(|| {
                EngineError::other(format!("shadow indirect geometry page {page} disappeared"))
            })?;
            let page_slot = shadow_page_slot(cascade_slot, page);
            let command_bytes = gpu_indexed_indirect_commands_as_bytes(&plan.commands);
            let draw_count = plan.commands.len().min(u32::MAX as usize) as u32;
            let count_bytes = draw_count.to_ne_bytes();
            let command_buffer = ensure_buffer(
                r,
                lifetime,
                frame_index,
                &mut page_slot.commands,
                command_bytes.len() as u64,
                BufferUsage::Indirect,
                &format!("gpu_shadow.c{cascade_index}.page{page}.commands"),
                &mut self.buffer_grows,
            )?;
            let count_buffer = ensure_buffer(
                r,
                lifetime,
                frame_index,
                &mut page_slot.count,
                4,
                BufferUsage::Indirect,
                &format!("gpu_shadow.c{cascade_index}.page{page}.count"),
                &mut self.buffer_grows,
            )?;
            r.write_buffer(command_buffer, 0, command_bytes)?;
            r.write_buffer(count_buffer, 0, &count_bytes)?;
            pages.push(ShadowIndirectPageStream {
                vertex_buffer,
                index_buffer,
                command_buffer,
                count_buffer,
                draw_count,
            });
        }

        if pages.is_empty() {
            return Ok(None);
        }
        self.builds = self.builds.saturating_add(1);
        self.represented = self
            .represented
            .saturating_add(represented_keys.len() as u64);
        Ok(Some(ShadowIndirectStream {
            pages,
            entity_keys: represented_keys,
        }))
    }
}

impl super::RuntimeRenderController {
    pub(in crate::render_controller) fn record_gpu_indirect_shadow_subset(
        &mut self,
        r: &mut dyn RenderApi,
        lit: LitPipeline,
        light_viewproj: newengine_math::Mat4,
        lights: &PackedLights,
        cascade_index: usize,
        admitted_entity_keys: &[u64],
        shadow_cull: Option<ShadowCasterCull>,
        cascade_texel_world_size: f32,
    ) -> EngineResult<Option<Vec<u64>>> {
        if !newengine_runtime_env::var_bool("NEWENGINE_GPU_DRIVEN_INDIRECT_ENABLE", false)
            || !newengine_runtime_env::var_bool(
                "NEWENGINE_GPU_DRIVEN_SHADOW_INDIRECT_ENABLE",
                false,
            )
            || !self.gpu_driven_backend_ready()
        {
            return Ok(None);
        }
        let frame_index = self.frame.frame_index;
        let Some(buffers) = self.gpu.table_buffers else {
            return Ok(None);
        };
        if buffers.frame_index != frame_index {
            return Ok(None);
        }

        let pipeline = self.gpu.indirect_shadow.ensure_pipeline(r, lit)?;
        let object_group = {
            let gpu = &mut self.gpu;
            gpu.indirect_shadow.ensure_object_group(
                r,
                &mut gpu.lifetimes.resources,
                frame_index,
                buffers,
            )?
        };
        let stream = {
            let gpu = &mut self.gpu;
            gpu.indirect_shadow.build_stream(
                r,
                &mut gpu.lifetimes.resources,
                frame_index,
                cascade_index,
                &gpu.geometry,
                &gpu.tables,
                admitted_entity_keys,
                shadow_cull,
                cascade_texel_world_size,
            )?
        };
        let Some(stream) = stream else {
            return Ok(None);
        };

        let ubo_key = SHADOW_UBO_KEY_BASE ^ ((cascade_index as u64) & 0xffff);
        let per = self.ensure_per_draw_ubo_with_binding(
            r,
            lit,
            ubo_key,
            lit.white_texture,
            lit.flat_normal_texture,
            lit.white_texture,
            lit.white_texture,
            lit.white_texture,
            lit.clamp_sampler,
        )?;
        super::passes_ubo::write_lit_ubo_ex(
            r,
            per.ubo,
            light_viewproj,
            newengine_math::Mat4::IDENTITY,
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 0.0],
            0.0,
            [1.0, 1.0, 0.0, 0.0],
            [1.0, 0.75, 0.0, 1.0],
            lights,
        )?;

        r.set_pipeline(pipeline)?;
        r.set_bind_group(0, per.bg)?;
        r.set_bind_group(1, object_group)?;
        for page in &stream.pages {
            r.set_vertex_buffer(0, BufferSlice::new(page.vertex_buffer, 0))?;
            r.set_index_buffer(BufferSlice::new(page.index_buffer, 0), IndexFormat::U32)?;
            r.draw_indexed_indirect_count(DrawIndexedIndirectCountArgs::new(
                page.command_buffer,
                0,
                page.count_buffer,
                0,
                page.draw_count,
            ))?;
        }
        Ok(Some(stream.entity_keys))
    }
}

fn cascade_slot(frame: &mut ShadowFrameSlot, cascade: usize) -> &mut ShadowCascadeSlot {
    if let Some(index) = frame.cascades.iter().position(|slot| slot.cascade == cascade) {
        return &mut frame.cascades[index];
    }
    frame.cascades.push(ShadowCascadeSlot {
        cascade,
        ..ShadowCascadeSlot::default()
    });
    frame.cascades.last_mut().expect("cascade slot was just inserted")
}

fn shadow_page_slot(cascade: &mut ShadowCascadeSlot, page: u32) -> &mut ShadowPageSlot {
    if let Some(index) = cascade.pages.iter().position(|slot| slot.page == page) {
        return &mut cascade.pages[index];
    }
    cascade.pages.push(ShadowPageSlot {
        page,
        ..ShadowPageSlot::default()
    });
    cascade.pages.last_mut().expect("shadow page slot was just inserted")
}

fn ensure_buffer(
    render: &mut dyn RenderApi,
    lifetime: &mut RenderGpuLifetimeQueue,
    frame_index: u64,
    slot: &mut BufferSlot,
    required_bytes: u64,
    usage: BufferUsage,
    label: &str,
    grow_counter: &mut u64,
) -> EngineResult<BufferId> {
    let required = required_bytes.max(MIN_BUFFER_BYTES);
    if let Some(id) = slot.id {
        if slot.capacity_bytes >= required {
            return Ok(id);
        }
    }
    let capacity = required
        .checked_next_power_of_two()
        .ok_or_else(|| EngineError::other("gpu shadow indirect buffer capacity overflow"))?;
    let id = render.create_buffer(
        BufferDesc::new(capacity, usage, MemoryHint::CpuToGpu).with_label(label.to_owned()),
    )?;
    if let Some(previous) = slot.id.replace(id) {
        lifetime.retire_buffer_after_frame(previous, frame_index);
    }
    slot.capacity_bytes = capacity;
    *grow_counter = grow_counter.saturating_add(1);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_shadow_page_cap_matches_indirect_provider_cap() {
        assert_eq!(MAX_COMMANDS_PER_PAGE, 4096);
    }

    #[test]
    fn shadow_pipeline_uses_primitive_vertex_stride() {
        assert_eq!(std::mem::size_of::<PrimitiveVertex>(), 32);
    }
}
