#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_core::render::{
    BufferDesc, BufferId, BufferUsage, MemoryHint, RenderApi, RenderExecutionCapabilities,
};
use newengine_core::{EngineError, EngineResult};

use super::gpu_scene_tables::{GpuGeometryRecord, GpuMaterialRecord, GpuObjectRecord};
use crate::render_controller::resource_lifetime::RenderGpuLifetimeQueue;

const GPU_SCENE_TABLE_FRAME_SLOTS: usize = 8;
const MIN_TABLE_BUFFER_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Default)]
struct TableBufferSlot {
    buffer: Option<BufferId>,
    capacity_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct SceneTableFrameSlot {
    geometry: TableBufferSlot,
    materials: TableBufferSlot,
    objects: TableBufferSlot,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::render_controller) struct GpuSceneTableBuffers {
    pub(in crate::render_controller) frame_index: u64,
    pub(in crate::render_controller) ring_slot: u8,
    pub(in crate::render_controller) geometry: BufferId,
    pub(in crate::render_controller) materials: BufferId,
    pub(in crate::render_controller) objects: BufferId,
    pub(in crate::render_controller) geometry_count: u32,
    pub(in crate::render_controller) material_count: u32,
    pub(in crate::render_controller) object_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::render_controller) struct GpuSceneTableUploadStats {
    pub(in crate::render_controller) uploads: u64,
    pub(in crate::render_controller) bytes_written: u64,
    pub(in crate::render_controller) buffer_grows: u64,
    pub(in crate::render_controller) active_ring_slot: u8,
    pub(in crate::render_controller) last_upload_frame: u64,
}

#[derive(Debug)]
pub(in crate::render_controller) struct GpuSceneTableUploader {
    frame_slots: [SceneTableFrameSlot; GPU_SCENE_TABLE_FRAME_SLOTS],
    disabled_reason: Option<String>,
    stats: GpuSceneTableUploadStats,
}

impl Default for GpuSceneTableUploader {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuSceneTableUploader {
    pub(in crate::render_controller) fn new() -> Self {
        Self {
            frame_slots: std::array::from_fn(|_| SceneTableFrameSlot::default()),
            disabled_reason: None,
            stats: GpuSceneTableUploadStats::default(),
        }
    }

    #[inline]
    pub(in crate::render_controller) fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    #[inline]
    pub(in crate::render_controller) fn stats(&self) -> GpuSceneTableUploadStats {
        self.stats
    }

    /// Uploads one immutable CPU scene-table snapshot into a host-visible frame-ring slot.
    ///
    /// The ring depth is validated against backend-declared execution semantics. Buffer growth
    /// retires the previous generation through the same `FrameCompleted` authority used by
    /// render-target and streamed-mesh lifetime management.
    pub(in crate::render_controller) fn upload_fail_open(
        &mut self,
        render: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        execution: RenderExecutionCapabilities,
        frame_index: u64,
        geometry: &[GpuGeometryRecord],
        materials: &[GpuMaterialRecord],
        objects: &[GpuObjectRecord],
    ) -> Option<GpuSceneTableBuffers> {
        if self.disabled_reason.is_some() {
            return None;
        }
        match self.upload(
            render,
            lifetime,
            execution,
            frame_index,
            geometry,
            materials,
            objects,
        ) {
            Ok(buffers) => Some(buffers),
            Err(error) => {
                let reason = error.to_string();
                newengine_ulog_api::ulog::warn!(
                    "render gpu scene tables: disabling optional storage-buffer upload path err='{}'; legacy draw submission remains authoritative",
                    reason,
                );
                self.disabled_reason = Some(reason);
                None
            }
        }
    }

    fn upload(
        &mut self,
        render: &mut dyn RenderApi,
        lifetime: &mut RenderGpuLifetimeQueue,
        execution: RenderExecutionCapabilities,
        frame_index: u64,
        geometry: &[GpuGeometryRecord],
        materials: &[GpuMaterialRecord],
        objects: &[GpuObjectRecord],
    ) -> EngineResult<GpuSceneTableBuffers> {
        let required_ring = execution.host_visible_ring_slots() as usize;
        if required_ring > GPU_SCENE_TABLE_FRAME_SLOTS {
            return Err(EngineError::other(format!(
                "gpu scene table frame ring too shallow required={} available={} backend_frames_in_flight={}",
                required_ring,
                GPU_SCENE_TABLE_FRAME_SLOTS,
                execution.normalized_frames_in_flight(),
            )));
        }

        let ring_slot = frame_index as usize % GPU_SCENE_TABLE_FRAME_SLOTS;
        let geometry_bytes = records_as_bytes(geometry);
        let material_bytes = records_as_bytes(materials);
        let object_bytes = records_as_bytes(objects);
        let slot = &mut self.frame_slots[ring_slot];

        let geometry_buffer = ensure_table_buffer(
            render,
            lifetime,
            frame_index,
            &mut slot.geometry,
            geometry_bytes.len() as u64,
            "gpu_scene.geometry",
            &mut self.stats.buffer_grows,
        )?;
        let material_buffer = ensure_table_buffer(
            render,
            lifetime,
            frame_index,
            &mut slot.materials,
            material_bytes.len() as u64,
            "gpu_scene.materials",
            &mut self.stats.buffer_grows,
        )?;
        let object_buffer = ensure_table_buffer(
            render,
            lifetime,
            frame_index,
            &mut slot.objects,
            object_bytes.len() as u64,
            "gpu_scene.objects",
            &mut self.stats.buffer_grows,
        )?;

        if !geometry_bytes.is_empty() {
            render.write_buffer(geometry_buffer, 0, geometry_bytes)?;
        }
        if !material_bytes.is_empty() {
            render.write_buffer(material_buffer, 0, material_bytes)?;
        }
        if !object_bytes.is_empty() {
            render.write_buffer(object_buffer, 0, object_bytes)?;
        }

        let bytes_written = geometry_bytes
            .len()
            .saturating_add(material_bytes.len())
            .saturating_add(object_bytes.len()) as u64;
        self.stats.uploads = self.stats.uploads.saturating_add(1);
        self.stats.bytes_written = self.stats.bytes_written.saturating_add(bytes_written);
        self.stats.active_ring_slot = ring_slot as u8;
        self.stats.last_upload_frame = frame_index;

        Ok(GpuSceneTableBuffers {
            frame_index,
            ring_slot: ring_slot as u8,
            geometry: geometry_buffer,
            materials: material_buffer,
            objects: object_buffer,
            geometry_count: geometry.len().min(u32::MAX as usize) as u32,
            material_count: materials.len().min(u32::MAX as usize) as u32,
            object_count: objects.len().min(u32::MAX as usize) as u32,
        })
    }
}

fn ensure_table_buffer(
    render: &mut dyn RenderApi,
    lifetime: &mut RenderGpuLifetimeQueue,
    frame_index: u64,
    slot: &mut TableBufferSlot,
    required_bytes: u64,
    label: &str,
    grow_counter: &mut u64,
) -> EngineResult<BufferId> {
    let required_bytes = required_bytes.max(MIN_TABLE_BUFFER_BYTES);
    if let Some(buffer) = slot.buffer {
        if slot.capacity_bytes >= required_bytes {
            return Ok(buffer);
        }
    }

    let capacity = next_table_capacity(required_bytes)?;
    let buffer = render.create_buffer(
        BufferDesc::new(capacity, BufferUsage::Storage, MemoryHint::CpuToGpu).with_label(format!(
            "{label}.frame_slot.capacity_{}kb",
            capacity / 1024
        )),
    )?;
    if let Some(previous) = slot.buffer.replace(buffer) {
        lifetime.retire_buffer_after_frame(previous, frame_index);
    }
    slot.capacity_bytes = capacity;
    *grow_counter = grow_counter.saturating_add(1);
    Ok(buffer)
}

#[inline]
fn next_table_capacity(required: u64) -> EngineResult<u64> {
    required
        .checked_next_power_of_two()
        .ok_or_else(|| EngineError::other("gpu scene table capacity overflow"))
}

#[inline]
fn records_as_bytes<T>(records: &[T]) -> &[u8] {
    let byte_len = core::mem::size_of_val(records);
    let ptr = records.as_ptr().cast::<u8>();
    // SAFETY: scene-table records are repr(C), contain no references, and the returned read-only
    // byte slice has exactly the source slice lifetime.
    unsafe { core::slice::from_raw_parts(ptr, byte_len) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploader_ring_exceeds_current_backend_safety_margin() {
        let execution = RenderExecutionCapabilities {
            frames_in_flight: 3,
            ..RenderExecutionCapabilities::default()
        };
        assert_eq!(execution.host_visible_ring_slots(), 5);
        assert!(GPU_SCENE_TABLE_FRAME_SLOTS >= execution.host_visible_ring_slots() as usize);
    }

    #[test]
    fn capacity_is_power_of_two_and_never_below_minimum() {
        assert_eq!(next_table_capacity(MIN_TABLE_BUFFER_BYTES).unwrap(), 4096);
        assert_eq!(next_table_capacity(4097).unwrap(), 8192);
    }
}

impl super::RuntimeRenderController {
    pub(in crate::render_controller) fn prepare_gpu_scene_tables(
        &mut self,
        render: &mut dyn RenderApi,
        scene: &newengine_scene::Scene,
        runtime: bool,
    ) {
        if !newengine_runtime_env::var_bool("NEWENGINE_GPU_SCENE_TABLES_ENABLE", false) {
            self.gpu.table_buffers = None;
            self.gpu.indirect_stream.clear_current();
            return;
        }

        // Capturing here happens after bounded geometry residency. Later shadow/GBuffer passes
        // reuse this exact Arc-backed snapshot, so enabling the data plane does not add a second
        // ECS scan within the same frame.
        let _ = self.primitive_scene_snapshot(scene, runtime);
        let frame_index = self.frame.frame_index;
        let execution = self.backend_execution;
        let indirect_requested = newengine_runtime_env::var_bool(
            "NEWENGINE_GPU_DRIVEN_INDIRECT_ENABLE",
            false,
        );
        let indirect_backend_ready = self.gpu_driven_backend_ready();
        let gpu = &mut self.gpu;
        let buffers = gpu.table_uploader.upload_fail_open(
            render,
            &mut gpu.lifetimes.resources,
            execution,
            frame_index,
            gpu.tables.geometry_records(),
            gpu.tables.material_records(),
            gpu.tables.object_records(),
        );
        gpu.table_buffers = buffers;

        if indirect_requested && indirect_backend_ready && gpu.table_buffers.is_some() {
            let geometry = &gpu.geometry;
            let tables = &gpu.tables;
            let lifetime = &mut gpu.lifetimes.resources;
            let builder = &mut gpu.indirect_stream;
            let _ = builder.build_fail_open(
                render,
                lifetime,
                execution,
                frame_index,
                geometry,
                tables,
            );
        } else {
            gpu.indirect_stream.clear_current();
        }

        if newengine_ulog_api::ulog::trace_enabled()
            && indirect_requested
            && (frame_index <= 3 || frame_index.is_multiple_of(300))
        {
            let stream = gpu.indirect_stream.current();
            let stats = gpu.indirect_stream.stats();
            newengine_ulog_api::ulog::trace!(
                "render gpu indirect stream: frame={} requested={} backend_ready={} table_buffers_ready={} stream_ready={} migration_ready={} pages={} active={} represented={} builds={} incomplete_frames={} disabled_reason={:?}",
                frame_index,
                indirect_requested,
                indirect_backend_ready,
                gpu.table_buffers.is_some(),
                stream.is_some(),
                stream.is_some_and(|stream| stream.migration_ready()),
                stream.map(|stream| stream.pages.len()).unwrap_or(0),
                stream.map(|stream| stream.active_object_count).unwrap_or(0),
                stream.map(|stream| stream.represented_object_count).unwrap_or(0),
                stats.builds,
                stats.incomplete_frames,
                gpu.indirect_stream.disabled_reason(),
            );
        }

        if newengine_ulog_api::ulog::trace_enabled()
            && (frame_index <= 3 || frame_index.is_multiple_of(300))
        {
            let table_stats = gpu.tables.stats();
            let upload_stats = gpu.table_uploader.stats();
            newengine_ulog_api::ulog::trace!(
                "render gpu scene table upload: frame={} ready={} ring_slot={} geometry={}/{} materials={}/{} objects={}/{} bytes_total={} grows={} disabled_reason={:?}",
                frame_index,
                gpu.table_buffers.is_some(),
                upload_stats.active_ring_slot,
                table_stats.geometry_resident,
                table_stats.geometry_slots,
                table_stats.material_resident,
                table_stats.material_slots,
                table_stats.object_resident,
                table_stats.object_slots,
                upload_stats.bytes_written,
                upload_stats.buffer_grows,
                gpu.table_uploader.disabled_reason(),
            );
        }
    }
}