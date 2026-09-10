use serde::{Deserialize, Serialize};

use crate::{BufferId, FrameCameraContext};

/// Engine-owned SSBO record consumed by GPU visibility/indirect preparation.
///
/// This is a provider-neutral buffer ABI: no Vulkan/D3D/Metal handles cross the render API.
/// `sphere = center.xyz + radius`, `meta.x & 1` marks the record as cullable. Remaining lanes are
/// reserved for LOD/residency/mesh-table indices so the layout can evolve without changing stride.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuVisibilityIndirectCandidate {
    pub sphere: [f32; 4],
    pub meta: [u32; 4],
}

impl GpuVisibilityIndirectCandidate {
    pub const STRIDE: u32 = 32;
    pub const FLAG_CULLABLE: u32 = 1;

    #[inline]
    pub const fn cullable(center: [f32; 3], radius: f32) -> Self {
        Self {
            sphere: [center[0], center[1], center[2], radius],
            meta: [Self::FLAG_CULLABLE, 0, 0, 0],
        }
    }

    #[inline]
    pub const fn fail_open() -> Self {
        Self {
            sphere: [0.0, 0.0, 0.0, 0.0],
            meta: [0, 0, 0, 0],
        }
    }
}

/// Byte-identical to the backend's indexed indirect command record.
///
/// Keeping this DTO in the engine-facing render contract lets CPU initialization and GPU compute
/// share one 20-byte ABI while the native backend remains free to translate its BufferId handles.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuDrawIndexedIndirectCommand {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
}

impl GpuDrawIndexedIndirectCommand {
    pub const STRIDE: u32 = 20;

    #[inline]
    pub const fn new(index_count: u32, instance_count: u32) -> Self {
        Self {
            index_count,
            instance_count,
            first_index: 0,
            vertex_offset: 0,
            first_instance: 0,
        }
    }
}

/// One compute dispatch that tests a packed batch-bound stream against the backend's previous-frame
/// depth hierarchy and writes visibility directly into indexed-indirect `instance_count` fields.
///
/// The buffers are ordinary RenderApi ids, not native handles. Backends that cannot provide a
/// previous-depth hierarchy must fail open and leave the initialized indirect commands untouched.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuVisibilityIndirectCullArgs {
    pub candidate_buffer: BufferId,
    pub candidate_offset: u64,
    pub indirect_buffer: BufferId,
    pub indirect_offset: u64,
    pub candidate_count: u32,
    pub candidate_stride: u32,
    pub command_stride: u32,
    pub viewport_extent: [u32; 2],
    pub camera: FrameCameraContext,
}

impl GpuVisibilityIndirectCullArgs {
    #[inline]
    pub const fn new(
        candidate_buffer: BufferId,
        candidate_offset: u64,
        indirect_buffer: BufferId,
        indirect_offset: u64,
        candidate_count: u32,
        viewport_extent: [u32; 2],
        camera: FrameCameraContext,
    ) -> Self {
        Self {
            candidate_buffer,
            candidate_offset,
            indirect_buffer,
            indirect_offset,
            candidate_count,
            candidate_stride: GpuVisibilityIndirectCandidate::STRIDE,
            command_stride: GpuDrawIndexedIndirectCommand::STRIDE,
            viewport_extent,
            camera,
        }
    }
}

#[inline]
pub fn gpu_visibility_candidates_as_bytes(
    candidates: &[GpuVisibilityIndirectCandidate],
) -> &[u8] {
    let byte_len = core::mem::size_of_val(candidates);
    let ptr = candidates.as_ptr().cast::<u8>();
    // SAFETY: both record types are repr(C), this view is read-only, and it cannot outlive input.
    unsafe { core::slice::from_raw_parts(ptr, byte_len) }
}

#[inline]
pub fn gpu_indexed_indirect_commands_as_bytes(
    commands: &[GpuDrawIndexedIndirectCommand],
) -> &[u8] {
    let byte_len = core::mem::size_of_val(commands);
    let ptr = commands.as_ptr().cast::<u8>();
    // SAFETY: `GpuDrawIndexedIndirectCommand` is repr(C) and has no padding-dependent references.
    unsafe { core::slice::from_raw_parts(ptr, byte_len) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_visibility_candidate_layout_is_stable() {
        assert_eq!(core::mem::size_of::<GpuVisibilityIndirectCandidate>(), 32);
        assert_eq!(core::mem::align_of::<GpuVisibilityIndirectCandidate>(), 16);
        assert_eq!(GpuVisibilityIndirectCandidate::STRIDE, 32);
    }

    #[test]
    fn indexed_indirect_command_layout_matches_native_abi() {
        assert_eq!(core::mem::size_of::<GpuDrawIndexedIndirectCommand>(), 20);
        assert_eq!(GpuDrawIndexedIndirectCommand::STRIDE, 20);
    }
}
