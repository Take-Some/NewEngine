#![forbid(unsafe_op_in_unsafe_fn)]

use std::collections::BTreeMap;

use newengine_core::render::{
    BufferDesc, BufferId, BufferUsage, MemoryHint, RenderApi, RenderExecutionCapabilities,
};
use newengine_core::{EngineError, EngineResult};
use newengine_render_api::{
    gpu_indexed_indirect_commands_as_bytes, gpu_visibility_candidates_as_bytes,
    GpuDrawIndexedIndirectCommand, GpuVisibilityIndirectCandidate,
};

use super::gpu_scene_tables::GpuSceneTables;
use crate::render_controller::gpu::GeometryArena;
use crate::render_controller::resource_lifetime::RenderGpuLifetimeQueue;

const INDIRECT_FRAME_SLOTS: usize = 8;
const MAX_COMMANDS_PER_PAGE: usize = 4096;
const MIN_BUFFER_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Default)]
struct BufferSlot {
    id: Option<BufferId>,
    capacity_bytes: u64,
}

#[derive(Debug, Default)]
struct IndirectPageSlot {
    page: u32,
    candidates: BufferSlot,
    source: BufferSlot,
    output: BufferSlot,
    count: BufferSlot,
}

#[derive(Debug, Default)]
struct IndirectFrameSlot {
    pages: Vec<IndirectPageSlot>,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::render_controller) struct GpuIndirectPageStream {
    pub(in crate::render_controller) page: u32,
    pub(in crate::render_controller) vertex_buffer: BufferId,
    pub(in crate::render_controller) index_buffer: BufferId,
    pub(in crate::render_controller) candidate_buffer: BufferId,
    pub(in crate::render_controller) source_indirect_buffer: BufferId,
    pub(in crate::render_controller) output_indirect_buffer: BufferId,
    pub(in crate::render_controller) count_buffer: BufferId,
    pub(in crate::render_controller) draw_count: u32,
}

#[derive(Clone, Debug, Default)]
pub(in crate::render_controller) struct GpuIndirectFrameStream {
    pub(in crate::render_controller) frame_index: u64,
    pub(in crate::render_controller) ring_slot: u8,
    /// False means at least one active object could not be represented. Such a stream must never
    /// replace legacy submission for the frame.
    pub(in crate::render_controller) complete: bool,
    pub(in crate::render_controller) active_object_count: u32,
    pub(in crate::render_controller) represented_object_count: u32,
    pub(in crate::render_controller) pages: Vec<GpuIndirectPageStream>,
    pub(in crate::render_controller) object_slots: Vec<u32>,
}

impl GpuIndirectFrameStream {
    #[inline]
    pub(in crate::render_controller) fn migration_ready(&self) -> bool {
        self.complete
            && self.active_object_count > 0
            && self.active_object_count == self.represented_object_count
            && !self.pages.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::render_controller) struct GpuIndirectStreamStats {
    pub(in crate::render_controller) builds: u64,
    pub(in crate::render_controller) represented_objects: u64,
    pub(in crate::render_controller) incomplete_frames: u64,
    pub(in crate::render_controller) buffer_grows: u64,
    pub(in crate::render_controller) bytes_written: u64,
    pub(in crate::render_controller) last_frame: u64,
}

#[derive(Debug)]
pub(in crate::render_controller) struct GpuIndirectStreamBuilder {
    frame_slots: [IndirectFrameSlot; INDIRECT_FRAME_SLOTS],
    current: Option<GpuIndirectFrameStream>,
    disabled_reason: Option<String>,
    stats: GpuIndirectStreamStats,
}

impl Default for GpuIndirectStreamBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuIndirectStreamBuilder {
    pub(in crate::render_controller) fn new() -> Self {
        Self {
            frame_slots: std::array::from_fn(|_| IndirectFrameSlot::default()),
            current: None,
            disabled_reason: None,
            stats: GpuIndirectStreamStats::default(),
        }
    }

    #[inline]
    pub(in crate::render_controller) fn current(&self) -> Option<&GpuIndirectFrameStream> {
        self.current.as_ref()
    }

    #[inline]
    pub(in crate::render_controller) fn stats(&self) -> GpuIndirectStreamStats {
        self.stats
    }

    #[inline]
    pub(in crate::render_controller) fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    pub(in crate::render_controller) fn clear_current(&mut self) {
        self.current = None;
    }

    pub(in crate::render_controller) fn build_fail_open(
        &mut self,
        render: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        execution: RenderExecutionCapabilities,
        frame_index: u64,
        geometry: &GeometryArena,
        tables: &GpuSceneTables,
    ) -> Option<&GpuIndirectFrameStream> {
        if self.disabled_reason.is_some() {
            self.current = None;
            return None;
        }
        match self.build(
            render,
            lifetime,
            execution,
            frame_index,
            geometry,
            tables,
        ) {
            Ok(stream) => {
                self.current = Some(stream);
                self.current.as_ref()
            }
            Err(error) => {
                let reason = error.to_string();
                newengine_ulog_api::ulog::warn!(
                    "render gpu indirect stream: disabling optional builder err='{}'; legacy submission remains authoritative",
                    reason,
                );
                self.disabled_reason = Some(reason);
                self.current = None;
                None
            }
        }
    }

    fn build(
        &mut self,
        render: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        execution: RenderExecutionCapabilities,
        frame_index: u64,
        geometry: &GeometryArena,
        tables: &GpuSceneTables,
    ) -> EngineResult<GpuIndirectFrameStream> {
        let required_ring = execution.host_visible_ring_slots() as usize;
        if required_ring > INDIRECT_FRAME_SLOTS {
            return Err(EngineError::other(format!(
                "gpu indirect frame ring too shallow required={} available={}",
                required_ring, INDIRECT_FRAME_SLOTS,
            )));
        }

        #[derive(Default)]
        struct PagePlan {
            candidates: Vec<GpuVisibilityIndirectCandidate>,
            commands: Vec<GpuDrawIndexedIndirectCommand>,
        }

        let mut plans = BTreeMap::<u32, PagePlan>::new();
        let geometry_records = tables.geometry_records();
        let object_records = tables.object_records();
        let mut active_objects = 0u32;
        let mut represented_objects = 0u32;
        let mut represented_slots = Vec::new();
        let mut complete = true;

        for (object_slot, object) in object_records.iter().enumerate() {
            if !object.gbuffer_indirect_eligible() {
                continue;
            }
            active_objects = active_objects.saturating_add(1);
            let [geometry_slot, geometry_generation] = object.geometry_handle_lanes();
            let Some(record) = geometry_records.get(geometry_slot as usize) else {
                complete = false;
                continue;
            };
            if record.meta[0] != geometry_generation || record.meta[1] == 0 {
                complete = false;
                continue;
            }
            let page = record.meta[2];
            if geometry.page_buffers(page).is_none() {
                complete = false;
                continue;
            }
            let plan = plans.entry(page).or_default();
            if plan.commands.len() >= MAX_COMMANDS_PER_PAGE {
                complete = false;
                continue;
            }

            let cullable = object.indirect_cullable();
            plan.candidates.push(GpuVisibilityIndirectCandidate {
                sphere: object.bounds,
                meta: [
                    if cullable {
                        GpuVisibilityIndirectCandidate::FLAG_CULLABLE
                    } else {
                        0
                    },
                    object_slot as u32,
                    geometry_slot,
                    page,
                ],
            });
            plan.commands.push(GpuDrawIndexedIndirectCommand {
                index_count: record.draw[1],
                instance_count: 1,
                first_index: record.draw[0],
                vertex_offset: record.draw[2] as i32,
                first_instance: object_slot as u32,
            });
            represented_objects = represented_objects.saturating_add(1);
            represented_slots.push(object_slot as u32);
        }

        let ring_slot = frame_index as usize % INDIRECT_FRAME_SLOTS;
        let frame_slot = &mut self.frame_slots[ring_slot];
        let mut streams = Vec::with_capacity(plans.len());
        let mut bytes_written = 0u64;

        for (page, plan) in plans {
            if plan.commands.is_empty() {
                continue;
            }
            let (vertex_buffer, index_buffer) = geometry.page_buffers(page).ok_or_else(|| {
                EngineError::other(format!("geometry arena page {page} disappeared during build"))
            })?;
            let slot = page_slot(frame_slot, page);
            let candidate_bytes = gpu_visibility_candidates_as_bytes(&plan.candidates);
            let command_bytes = gpu_indexed_indirect_commands_as_bytes(&plan.commands);
            let draw_count = plan.commands.len().min(u32::MAX as usize) as u32;
            let count_bytes = draw_count.to_ne_bytes();

            let candidate_buffer = ensure_buffer(
                render,
                lifetime,
                frame_index,
                &mut slot.candidates,
                candidate_bytes.len() as u64,
                BufferUsage::Storage,
                &format!("gpu_indirect.page{page}.candidates"),
                &mut self.stats.buffer_grows,
            )?;
            let source_buffer = ensure_buffer(
                render,
                lifetime,
                frame_index,
                &mut slot.source,
                command_bytes.len() as u64,
                BufferUsage::Indirect,
                &format!("gpu_indirect.page{page}.source"),
                &mut self.stats.buffer_grows,
            )?;
            let output_buffer = ensure_buffer(
                render,
                lifetime,
                frame_index,
                &mut slot.output,
                command_bytes.len() as u64,
                BufferUsage::Indirect,
                &format!("gpu_indirect.page{page}.output"),
                &mut self.stats.buffer_grows,
            )?;
            let count_buffer = ensure_buffer(
                render,
                lifetime,
                frame_index,
                &mut slot.count,
                4,
                BufferUsage::Indirect,
                &format!("gpu_indirect.page{page}.count"),
                &mut self.stats.buffer_grows,
            )?;

            render.write_buffer(candidate_buffer, 0, candidate_bytes)?;
            render.write_buffer(source_buffer, 0, command_bytes)?;
            // Fail-open initialization: without a valid V2 dispatch, output/count represent the
            // entire source stream and draw_indexed_indirect_count would draw every command.
            render.write_buffer(output_buffer, 0, command_bytes)?;
            render.write_buffer(count_buffer, 0, &count_bytes)?;

            bytes_written = bytes_written.saturating_add(
                candidate_bytes.len() as u64
                    + command_bytes.len() as u64 * 2
                    + count_bytes.len() as u64,
            );
            streams.push(GpuIndirectPageStream {
                page,
                vertex_buffer,
                index_buffer,
                candidate_buffer,
                source_indirect_buffer: source_buffer,
                output_indirect_buffer: output_buffer,
                count_buffer,
                draw_count,
            });
        }

        self.stats.builds = self.stats.builds.saturating_add(1);
        self.stats.represented_objects = self
            .stats
            .represented_objects
            .saturating_add(u64::from(represented_objects));
        self.stats.bytes_written = self.stats.bytes_written.saturating_add(bytes_written);
        self.stats.last_frame = frame_index;
        if !complete || active_objects != represented_objects {
            self.stats.incomplete_frames = self.stats.incomplete_frames.saturating_add(1);
        }

        Ok(GpuIndirectFrameStream {
            frame_index,
            ring_slot: ring_slot as u8,
            complete: complete && active_objects == represented_objects,
            active_object_count: active_objects,
            represented_object_count: represented_objects,
            pages: streams,
            object_slots: represented_slots,
        })
    }
}

fn page_slot(frame: &mut IndirectFrameSlot, page: u32) -> &mut IndirectPageSlot {
    if let Some(index) = frame.pages.iter().position(|slot| slot.page == page) {
        return &mut frame.pages[index];
    }
    frame.pages.push(IndirectPageSlot {
        page,
        ..IndirectPageSlot::default()
    });
    frame.pages.last_mut().expect("page slot was just inserted")
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
        .ok_or_else(|| EngineError::other("gpu indirect buffer capacity overflow"))?;
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
    fn page_cap_is_provider_visibility_cap() {
        assert_eq!(MAX_COMMANDS_PER_PAGE, 4096);
    }

    #[test]
    fn empty_frame_is_not_migration_ready() {
        assert!(!GpuIndirectFrameStream {
            frame_index: 1,
            ring_slot: 0,
            complete: true,
            active_object_count: 0,
            represented_object_count: 0,
            pages: Vec::new(),
            object_slots: Vec::new(),
        }
        .migration_ready());
    }
}
