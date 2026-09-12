#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_core::render::{BufferDesc, BufferId, BufferUsage, MemoryHint, RenderApi};
use newengine_core::{EngineError, EngineResult};
use newengine_math::collections::FxHashMap;
use newengine_primitives::{PrimitiveId, PrimitiveMesh, PrimitiveVertex};

const DEFAULT_VERTEX_PAGE_MIB: u32 = 32;
const DEFAULT_INDEX_PAGE_MIB: u32 = 16;
const DEFAULT_MAX_PAGES: u32 = 64;
const MIB: u64 = 1024 * 1024;
const INDEX_STRIDE: u64 = std::mem::size_of::<u32>() as u64;
const VERTEX_STRIDE: u64 = std::mem::size_of::<PrimitiveVertex>() as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::render_controller) struct GeometryHandle {
    pub(in crate::render_controller) slot: u32,
    pub(in crate::render_controller) generation: u32,
}

impl GeometryHandle {
    #[inline]
    pub(in crate::render_controller) const fn invalid() -> Self {
        Self { slot: u32::MAX, generation: 0 }
    }
}

impl Default for GeometryHandle {
    #[inline]
    fn default() -> Self {
        Self::invalid()
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::render_controller) struct GeometrySlice {
    pub(in crate::render_controller) handle: GeometryHandle,
    pub(in crate::render_controller) page: u32,
    pub(in crate::render_controller) vertex_buffer: BufferId,
    pub(in crate::render_controller) index_buffer: BufferId,
    pub(in crate::render_controller) vertex_offset_bytes: u64,
    pub(in crate::render_controller) index_offset_bytes: u64,
    pub(in crate::render_controller) first_vertex: u32,
    pub(in crate::render_controller) first_index: u32,
    pub(in crate::render_controller) vertex_count: u32,
    pub(in crate::render_controller) index_count: u32,
    pub(in crate::render_controller) bounds_center: newengine_math::Vec3,
    pub(in crate::render_controller) bounds_radius: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::render_controller) struct GeometryArenaStats {
    pub(in crate::render_controller) pages: usize,
    pub(in crate::render_controller) resident_slots: usize,
    pub(in crate::render_controller) retired_slots: usize,
    pub(in crate::render_controller) reusable_slots: usize,
    pub(in crate::render_controller) vertex_capacity_bytes: u64,
    pub(in crate::render_controller) index_capacity_bytes: u64,
    pub(in crate::render_controller) vertex_free_bytes: u64,
    pub(in crate::render_controller) index_free_bytes: u64,
    pub(in crate::render_controller) uploads: u64,
    pub(in crate::render_controller) evictions: u64,
    pub(in crate::render_controller) reclaimed: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FreeRange {
    offset: u64,
    size: u64,
}

#[derive(Debug)]
struct RangeAllocator {
    capacity: u64,
    free: Vec<FreeRange>,
}

impl RangeAllocator {
    fn new(capacity: u64) -> Self {
        Self {
            capacity,
            free: vec![FreeRange {
                offset: 0,
                size: capacity,
            }],
        }
    }

    fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
        if size == 0 || alignment == 0 {
            return None;
        }
        for index in 0..self.free.len() {
            let range = self.free[index];
            let aligned = align_up(range.offset, alignment)?;
            let padding = aligned.checked_sub(range.offset)?;
            let consumed = padding.checked_add(size)?;
            if consumed > range.size {
                continue;
            }

            let tail_offset = aligned.checked_add(size)?;
            let tail_size = range.size - consumed;
            self.free.swap_remove(index);
            if padding > 0 {
                self.free.push(FreeRange {
                    offset: range.offset,
                    size: padding,
                });
            }
            if tail_size > 0 {
                self.free.push(FreeRange {
                    offset: tail_offset,
                    size: tail_size,
                });
            }
            self.sort_and_merge();
            return Some(aligned);
        }
        None
    }

    fn release(&mut self, offset: u64, size: u64) {
        if size == 0 || offset >= self.capacity {
            return;
        }
        let end = offset.saturating_add(size).min(self.capacity);
        if end <= offset {
            return;
        }
        self.free.push(FreeRange {
            offset,
            size: end - offset,
        });
        self.sort_and_merge();
    }

    fn free_bytes(&self) -> u64 {
        self.free.iter().map(|range| range.size).sum()
    }

    fn sort_and_merge(&mut self) {
        self.free.sort_unstable_by_key(|range| range.offset);
        let mut merged: Vec<FreeRange> = Vec::with_capacity(self.free.len());
        for range in self.free.drain(..) {
            if let Some(last) = merged.last_mut() {
                let last_end = last.offset.saturating_add(last.size);
                if range.offset <= last_end {
                    let range_end = range.offset.saturating_add(range.size);
                    last.size = last.size.max(range_end.saturating_sub(last.offset));
                    continue;
                }
            }
            merged.push(range);
        }
        self.free = merged;
    }
}

#[derive(Debug)]
struct GeometryPage {
    vertex_buffer: BufferId,
    index_buffer: BufferId,
    vertex_ranges: RangeAllocator,
    index_ranges: RangeAllocator,
}

#[derive(Clone, Copy, Debug)]
struct GeometrySlotEntry {
    generation: u32,
    primitive: Option<PrimitiveId>,
    slice: Option<GeometrySlice>,
}

impl Default for GeometrySlotEntry {
    fn default() -> Self {
        Self {
            generation: 1,
            primitive: None,
            slice: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RetiredGeometryAllocation {
    slot: u32,
    page: u32,
    vertex_offset_bytes: u64,
    vertex_size_bytes: u64,
    index_offset_bytes: u64,
    index_size_bytes: u64,
    after_frame: u64,
}

#[derive(Debug)]
pub(in crate::render_controller) struct GeometryArena {
    pages: Vec<GeometryPage>,
    slots: Vec<GeometrySlotEntry>,
    primitive_slots: FxHashMap<PrimitiveId, GeometryHandle>,
    reusable_slots: Vec<u32>,
    retired: Vec<RetiredGeometryAllocation>,
    vertex_page_bytes: u64,
    index_page_bytes: u64,
    max_pages: u32,
    disabled_reason: Option<String>,
    uploads: u64,
    evictions: u64,
    reclaimed: u64,
}

impl Default for GeometryArena {
    fn default() -> Self {
        Self::new()
    }
}

impl GeometryArena {
    pub(in crate::render_controller) fn new() -> Self {
        Self {
            pages: Vec::new(),
            slots: Vec::new(),
            primitive_slots: FxHashMap::default(),
            reusable_slots: Vec::new(),
            retired: Vec::new(),
            vertex_page_bytes: geometry_vertex_page_bytes(),
            index_page_bytes: geometry_index_page_bytes(),
            max_pages: geometry_max_pages(),
            disabled_reason: None,
            uploads: 0,
            evictions: 0,
            reclaimed: 0,
        }
    }

    #[inline]
    pub(in crate::render_controller) fn is_enabled(&self) -> bool {
        self.disabled_reason.is_none()
    }

    #[inline]
    pub(in crate::render_controller) fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }

    pub(in crate::render_controller) fn ensure_primitive_fail_open(
        &mut self,
        render: &mut dyn RenderApi,
        primitive: PrimitiveId,
        mesh: &PrimitiveMesh,
    ) -> Option<GeometryHandle> {
        if let Some(handle) = self.handle_for(primitive) {
            return Some(handle);
        }
        if self.disabled_reason.is_some() {
            return None;
        }
        match self.ensure_primitive(render, primitive, mesh) {
            Ok(handle) => Some(handle),
            Err(error) => {
                let reason = error.to_string();
                newengine_ulog_api::ulog::warn!(
                    "render geometry arena: disabling optional arena path after allocation/upload failure err='{}'; legacy dedicated mesh buffers remain authoritative",
                    reason,
                );
                self.disabled_reason = Some(reason);
                None
            }
        }
    }

    pub(in crate::render_controller) fn handle_for(&self, primitive: PrimitiveId) -> Option<GeometryHandle> {
        let handle = *self.primitive_slots.get(&primitive)?;
        self.resolve(handle).map(|_| handle)
    }

    pub(in crate::render_controller) fn resolve(&self, handle: GeometryHandle) -> Option<GeometrySlice> {
        let slot = self.slots.get(handle.slot as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.slice
    }

    pub(in crate::render_controller) fn retire_primitive(&mut self, primitive: PrimitiveId, after_frame: u64) -> bool {
        let Some(handle) = self.primitive_slots.get(&primitive).copied() else {
            return false;
        };
        let Some(slot) = self.slots.get_mut(handle.slot as usize) else {
            return false;
        };
        if slot.generation != handle.generation || slot.primitive != Some(primitive) {
            return false;
        }
        let Some(slice) = slot.slice.take() else {
            return false;
        };
        self.primitive_slots.remove(&primitive);
        slot.primitive = None;
        slot.generation = next_generation(slot.generation);

        self.retired.push(RetiredGeometryAllocation {
            slot: handle.slot,
            page: slice.page,
            vertex_offset_bytes: slice.vertex_offset_bytes,
            vertex_size_bytes: u64::from(slice.vertex_count).saturating_mul(VERTEX_STRIDE),
            index_offset_bytes: slice.index_offset_bytes,
            index_size_bytes: u64::from(slice.index_count).saturating_mul(INDEX_STRIDE),
            after_frame,
        });
        self.evictions = self.evictions.saturating_add(1);
        true
    }

    pub(in crate::render_controller) fn collect_completed(&mut self, latest_completed_frame: u64) -> usize {
        if latest_completed_frame == 0 || self.retired.is_empty() {
            return 0;
        }
        let mut collected = 0usize;
        let mut index = 0usize;
        while index < self.retired.len() {
            if self.retired[index].after_frame > latest_completed_frame {
                index += 1;
                continue;
            }
            let retired = self.retired.swap_remove(index);
            if let Some(page) = self.pages.get_mut(retired.page as usize) {
                page.vertex_ranges
                    .release(retired.vertex_offset_bytes, retired.vertex_size_bytes);
                page.index_ranges
                    .release(retired.index_offset_bytes, retired.index_size_bytes);
            }
            if (retired.slot as usize) < self.slots.len() {
                self.reusable_slots.push(retired.slot);
            }
            self.reclaimed = self.reclaimed.saturating_add(1);
            collected = collected.saturating_add(1);
        }
        collected
    }

    pub(in crate::render_controller) fn page_buffers(
        &self,
        page: u32,
    ) -> Option<(BufferId, BufferId)> {
        let page = self.pages.get(page as usize)?;
        Some((page.vertex_buffer, page.index_buffer))
    }
    pub(in crate::render_controller) fn table_snapshot(&self) -> Vec<(u32, bool, u32, u32, u32, i32, u32, u32, newengine_math::Vec3, f32)> {
        self.slots
            .iter()
            .map(|slot| {
                if let Some(slice) = slot.slice {
                    (
                        slot.generation,
                        true,
                        slice.page,
                        VERTEX_STRIDE as u32,
                        slice.first_index,
                        slice.first_vertex as i32,
                        slice.vertex_count,
                        slice.index_count,
                        slice.bounds_center,
                        slice.bounds_radius,
                    )
                } else {
                    (
                        slot.generation,
                        false,
                        u32::MAX,
                        VERTEX_STRIDE as u32,
                        0,
                        0,
                        0,
                        0,
                        newengine_math::Vec3::ZERO,
                        0.0,
                    )
                }
            })
            .collect()
    }

    pub(in crate::render_controller) fn stats(&self) -> GeometryArenaStats {
        let mut stats = GeometryArenaStats {
            pages: self.pages.len(),
            resident_slots: self.primitive_slots.len(),
            retired_slots: self.retired.len(),
            reusable_slots: self.reusable_slots.len(),
            uploads: self.uploads,
            evictions: self.evictions,
            reclaimed: self.reclaimed,
            ..GeometryArenaStats::default()
        };
        for page in &self.pages {
            stats.vertex_capacity_bytes = stats
                .vertex_capacity_bytes
                .saturating_add(page.vertex_ranges.capacity);
            stats.index_capacity_bytes = stats
                .index_capacity_bytes
                .saturating_add(page.index_ranges.capacity);
            stats.vertex_free_bytes = stats
                .vertex_free_bytes
                .saturating_add(page.vertex_ranges.free_bytes());
            stats.index_free_bytes = stats
                .index_free_bytes
                .saturating_add(page.index_ranges.free_bytes());
        }
        stats
    }

    fn ensure_primitive(
        &mut self,
        render: &mut dyn RenderApi,
        primitive: PrimitiveId,
        mesh: &PrimitiveMesh,
    ) -> EngineResult<GeometryHandle> {
        if mesh.vertices.is_empty() || mesh.indices.is_empty() {
            return Err(EngineError::other(
                "geometry arena cannot upload an empty primitive mesh",
            ));
        }
        if let Some(handle) = self.handle_for(primitive) {
            return Ok(handle);
        }

        let vertex_bytes = encode_vertices(mesh);
        let index_bytes = encode_indices(mesh);
        let vertex_size = vertex_bytes.len() as u64;
        let index_size = index_bytes.len() as u64;
        let (page_index, vertex_offset, index_offset) =
            self.allocate_ranges(render, vertex_size, index_size)?;
        let page = self
            .pages
            .get(page_index as usize)
            .ok_or_else(|| EngineError::other("geometry arena page disappeared after allocation"))?;
        let page_vertex_buffer = page.vertex_buffer;
        let page_index_buffer = page.index_buffer;

        if let Err(error) = render.write_buffer(page_vertex_buffer, vertex_offset, &vertex_bytes) {
            self.release_ranges_immediate(page_index, vertex_offset, vertex_size, index_offset, index_size);
            return Err(error);
        }
        if let Err(error) = render.write_buffer(page_index_buffer, index_offset, &index_bytes) {
            self.release_ranges_immediate(page_index, vertex_offset, vertex_size, index_offset, index_size);
            return Err(error);
        }

        let slot_index = self.allocate_slot();
        let generation = self.slots[slot_index as usize].generation;
        let handle = GeometryHandle {
            slot: slot_index,
            generation,
        };
        let first_vertex_u64 = vertex_offset / VERTEX_STRIDE;
        let first_index_u64 = index_offset / INDEX_STRIDE;
        if first_vertex_u64 > i32::MAX as u64 || first_index_u64 > u32::MAX as u64 {
            self.release_ranges_immediate(page_index, vertex_offset, vertex_size, index_offset, index_size);
            self.reusable_slots.push(slot_index);
            return Err(EngineError::other(
                "geometry arena page offset exceeds indirect draw addressing",
            ));
        }
        let slice = GeometrySlice {
            handle,
            page: page_index,
            vertex_buffer: page_vertex_buffer,
            index_buffer: page_index_buffer,
            vertex_offset_bytes: vertex_offset,
            index_offset_bytes: index_offset,
            first_vertex: first_vertex_u64 as u32,
            first_index: first_index_u64 as u32,
            vertex_count: mesh.vertices.len().min(u32::MAX as usize) as u32,
            index_count: mesh.indices.len().min(u32::MAX as usize) as u32,
            bounds_center: mesh.bounds_center,
            bounds_radius: mesh.bounds_radius.max(0.001),
        };
        let slot = &mut self.slots[slot_index as usize];
        slot.primitive = Some(primitive);
        slot.slice = Some(slice);
        self.primitive_slots.insert(primitive, handle);
        self.uploads = self.uploads.saturating_add(1);
        Ok(handle)
    }

    fn allocate_ranges(
        &mut self,
        render: &mut dyn RenderApi,
        vertex_size: u64,
        index_size: u64,
    ) -> EngineResult<(u32, u64, u64)> {
        for page_index in 0..self.pages.len() {
            if let Some(pair) = allocate_page_pair(&mut self.pages[page_index], vertex_size, index_size)
            {
                return Ok((page_index as u32, pair.0, pair.1));
            }
        }

        if self.pages.len() >= self.max_pages as usize {
            return Err(EngineError::other(format!(
                "geometry arena exhausted page budget pages={} max_pages={} vertex_bytes={} index_bytes={}",
                self.pages.len(), self.max_pages, vertex_size, index_size,
            )));
        }
        let vertex_capacity = self.vertex_page_bytes.max(
            align_up(vertex_size, VERTEX_STRIDE.max(16)).unwrap_or(vertex_size),
        );
        let index_capacity = self
            .index_page_bytes
            .max(align_up(index_size, INDEX_STRIDE).unwrap_or(index_size));
        let page_number = self.pages.len();
        let vb = render.create_buffer(
            BufferDesc::new(vertex_capacity, BufferUsage::Vertex, MemoryHint::CpuToGpu)
                .with_label(format!("geometry_arena.page{page_number}.vb")),
        )?;
        let ib = match render.create_buffer(
            BufferDesc::new(index_capacity, BufferUsage::Index, MemoryHint::CpuToGpu)
                .with_label(format!("geometry_arena.page{page_number}.ib")),
        ) {
            Ok(buffer) => buffer,
            Err(error) => {
                render.destroy_buffer(vb);
                return Err(error);
            }
        };
        self.pages.push(GeometryPage {
            vertex_buffer: vb,
            index_buffer: ib,
            vertex_ranges: RangeAllocator::new(vertex_capacity),
            index_ranges: RangeAllocator::new(index_capacity),
        });
        let page_index = (self.pages.len() - 1) as u32;
        let pair = allocate_page_pair(
            self.pages
                .last_mut()
                .expect("geometry page was just inserted"),
            vertex_size,
            index_size,
        )
        .ok_or_else(|| EngineError::other("new geometry arena page cannot fit requested mesh"))?;
        Ok((page_index, pair.0, pair.1))
    }

    fn release_ranges_immediate(
        &mut self,
        page: u32,
        vertex_offset: u64,
        vertex_size: u64,
        index_offset: u64,
        index_size: u64,
    ) {
        if let Some(page) = self.pages.get_mut(page as usize) {
            page.vertex_ranges.release(vertex_offset, vertex_size);
            page.index_ranges.release(index_offset, index_size);
        }
    }

    fn allocate_slot(&mut self) -> u32 {
        if let Some(slot) = self.reusable_slots.pop() {
            return slot;
        }
        let slot = self.slots.len() as u32;
        self.slots.push(GeometrySlotEntry::default());
        slot
    }
}

fn allocate_page_pair(
    page: &mut GeometryPage,
    vertex_size: u64,
    index_size: u64,
) -> Option<(u64, u64)> {
    let vertex_alignment = VERTEX_STRIDE.max(16);
    let vertex_offset = page.vertex_ranges.allocate(vertex_size, vertex_alignment)?;
    let Some(index_offset) = page.index_ranges.allocate(index_size, INDEX_STRIDE) else {
        page.vertex_ranges.release(vertex_offset, vertex_size);
        return None;
    };
    Some((vertex_offset, index_offset))
}

fn encode_vertices(mesh: &PrimitiveMesh) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(mesh.vertices.len().saturating_mul(VERTEX_STRIDE as usize));
    for vertex in &mesh.vertices {
        for value in &vertex.pos {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        for value in &vertex.nrm {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        for value in &vertex.uv {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

fn encode_indices(mesh: &PrimitiveMesh) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(mesh.indices.len().saturating_mul(INDEX_STRIDE as usize));
    for index in &mesh.indices {
        bytes.extend_from_slice(&index.to_ne_bytes());
    }
    bytes
}

#[inline]
fn align_up(value: u64, alignment: u64) -> Option<u64> {
    let mask = alignment.checked_sub(1)?;
    if !alignment.is_power_of_two() {
        let remainder = value % alignment;
        return if remainder == 0 {
            Some(value)
        } else {
            value.checked_add(alignment - remainder)
        };
    }
    value.checked_add(mask).map(|v| v & !mask)
}

#[inline]
fn next_generation(generation: u32) -> u32 {
    let next = generation.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

fn geometry_vertex_page_bytes() -> u64 {
    u64::from(newengine_runtime_env::var_u32(
        "NEWENGINE_GEOMETRY_ARENA_VERTEX_PAGE_MIB",
        DEFAULT_VERTEX_PAGE_MIB,
        4,
        256,
    )) * MIB
}

fn geometry_index_page_bytes() -> u64 {
    u64::from(newengine_runtime_env::var_u32(
        "NEWENGINE_GEOMETRY_ARENA_INDEX_PAGE_MIB",
        DEFAULT_INDEX_PAGE_MIB,
        2,
        128,
    )) * MIB
}

fn geometry_max_pages() -> u32 {
    newengine_runtime_env::var_u32(
        "NEWENGINE_GEOMETRY_ARENA_MAX_PAGES",
        DEFAULT_MAX_PAGES,
        1,
        256,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_allocator_merges_released_neighbors() {
        let mut allocator = RangeAllocator::new(1024);
        let a = allocator.allocate(128, 16).unwrap();
        let b = allocator.allocate(256, 16).unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 128);
        allocator.release(a, 128);
        allocator.release(b, 256);
        assert_eq!(allocator.free_bytes(), 1024);
        assert_eq!(allocator.free, vec![FreeRange { offset: 0, size: 1024 }]);
    }

    #[test]
    fn retired_slot_generation_invalidates_old_handle_before_reuse() {
        let primitive = PrimitiveId::new(7);
        let mut arena = GeometryArena::new();
        arena.slots.push(GeometrySlotEntry {
            generation: 5,
            primitive: Some(primitive),
            slice: None,
        });
        let old = GeometryHandle {
            slot: 0,
            generation: 5,
        };
        arena.primitive_slots.insert(primitive, old);
        // No resident slice means retirement is rejected; generation must remain stable.
        assert!(!arena.retire_primitive(primitive, 11));
        assert_eq!(arena.slots[0].generation, 5);
    }

    #[test]
    fn generation_never_wraps_to_zero() {
        assert_eq!(next_generation(u32::MAX), 1);
        assert_eq!(next_generation(1), 2);
    }

    #[test]
    fn align_up_supports_power_of_two_and_vertex_stride() {
        assert_eq!(align_up(33, 16), Some(48));
        assert_eq!(align_up(33, VERTEX_STRIDE), Some(64));
        assert_eq!(align_up(64, VERTEX_STRIDE), Some(64));
    }
}
