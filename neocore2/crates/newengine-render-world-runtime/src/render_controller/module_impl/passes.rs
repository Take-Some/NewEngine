#![forbid(unsafe_op_in_unsafe_fn)]

#[path = "passes_parts/mesh_passes.rs"]
mod mesh_passes;
#[path = "passes_parts/mesh_visibility.rs"]
pub(super) mod mesh_visibility;

pub(super) use self::mesh_passes::{
    draw_asset_preview_bundle, draw_primitives, draw_primitives_gbuffer, draw_primitives_shadow,
    draw_procedural_terrain, draw_procedural_terrain_gbuffer, draw_procedural_terrain_shadow,
    publish_camera_spawn, shadow_caster_projected_radius_visible, ShadowUboViewKey,
};
