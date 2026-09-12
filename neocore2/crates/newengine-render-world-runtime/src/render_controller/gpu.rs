#![forbid(unsafe_op_in_unsafe_fn)]

mod debug_lines;
mod geometry_arena;
mod hair;
mod material_registry;
mod primitives;
mod shader_manifest;
mod skinning;
mod types;
mod vfx_particles;

pub(super) use debug_lines::ensure_debug_line_pipeline;
pub(super) use geometry_arena::{GeometryArena, GeometryArenaStats, GeometryHandle, GeometrySlice};
pub(super) use hair::HairGpuRenderer;
pub use material_registry::MaterialGpuRegistry;
pub use newengine_material_domain_api::{
    LitPipeline, MaterialGpuPipeline, MaterialGpuPipelineKey, MaterialGpuPipelineProvider,
    MaterialPipelineBuildProfile, LIT_UBO_SIZE,
};
pub(super) use primitives::{ensure_primitive_gpu, upload_primitive_mesh};
pub(super) use skinning::{ensure_player_skin_gpu, ensure_skin_palette_gpu};
pub(super) use types::{DebugLineGpu, PlayerSkinGpu, PrimitiveGpu, SkinPaletteGpu};

pub(super) use vfx_particles::VfxGpuRenderer;
