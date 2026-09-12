use super::mesh_passes_primitive::{instance_batch_ubo_key, PrimitiveGpuPlan, PrimitivePlanKey};

use newengine_math::{collections::FxHashSet, hash_combine_u64};

use super::scene_mesh_pass::route_diagnostics_due;

use super::*;

/// Stable semantic identity for a shadow render view.
///
/// A light matrix is frame-varying payload and must never enter a persistent
/// per-draw cache key. Cascades and local atlas views still need distinct UBOs
/// inside one CPU frame because their matrices differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShadowUboViewKey {
    DirectionalCascade(usize),
    LocalView(usize),
}

impl ShadowUboViewKey {
    #[inline]
    pub(crate) const fn directional(cascade_index: usize) -> Self {
        Self::DirectionalCascade(cascade_index)
    }

    #[inline]
    pub(crate) const fn local(view_index: usize) -> Self {
        Self::LocalView(view_index)
    }

    #[inline]
    fn cache_discriminator(self) -> u64 {
        match self {
            Self::DirectionalCascade(index) => {
                hash_combine_u64(0xd1ec_710a_0000_0000, index as u64)
            }
            Self::LocalView(index) => hash_combine_u64(0x10ca_15ad_0000_0000, index as u64),
        }
    }
}

#[inline]
pub(in crate::render_controller::module_impl) fn shadow_caster_projected_radius_visible(
    cascade_index: usize,
    cascade_texel_world_size: f32,
    radius_ws: f32,
) -> bool {
    if cascade_index < 2
        || !cascade_texel_world_size.is_finite()
        || cascade_texel_world_size <= 1.0e-6
    {
        return true;
    }
    let projected_radius_texels = radius_ws.abs() / cascade_texel_world_size;
    let min_radius_texels = if cascade_index >= 3 { 0.90 } else { 0.50 };
    projected_radius_texels >= min_radius_texels
}

mod models;
mod primitives;
mod skinned;
mod terrain;

pub use terrain::draw_procedural_terrain_shadow;

#[derive(Clone, Copy, Debug, Default)]
pub struct PrimitiveShadowPassProfile {
    pub total_ms: f32,
    pub skinned_ms: f32,
    pub skinned_draws: usize,
    pub models_ms: f32,
    pub static_ms: f32,
    pub static_body_ms: f32,
    pub static_scan_ms: f32,
    pub static_plan_ms: f32,
    pub static_upload_ms: f32,
    pub static_replay_ms: f32,
}

pub fn draw_primitives_shadow(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    lit: newengine_material_domain_api::LitPipeline,
    light_viewproj: Mat4,
    lights: &PackedLights,
    runtime: bool,
    camera_position: Vec3,
    cascade_index: usize,
    cascade_texel_world_size: f32,
    shadow_ubo_view: ShadowUboViewKey,
) -> newengine_core::EngineResult<PrimitiveShadowPassProfile> {
    let stage_profile = runtime
        && (this.frame.frame_index <= 3 || this.frame.frame_index.is_multiple_of(30))
        && newengine_runtime_policy::render_runtime_policy().primitive_stage_log;
    let total_started = std::time::Instant::now();

    let started = std::time::Instant::now();
    let skinned_draws = skinned::draw_skinned_player_primitives_shadow(
        this,
        r,
        scene,
        lit,
        light_viewproj,
        lights,
        runtime,
        camera_position,
        cascade_index,
        cascade_texel_world_size,
        shadow_ubo_view,
    )?;
    let skinned_ms = started.elapsed().as_secs_f32() * 1000.0;

    let started = std::time::Instant::now();
    models::draw_model_components_shadow(
        this,
        r,
        scene,
        lit,
        light_viewproj,
        lights,
        runtime,
        camera_position,
        cascade_index,
        cascade_texel_world_size,
        shadow_ubo_view,
    )?;
    let models_ms = started.elapsed().as_secs_f32() * 1000.0;

    let started = std::time::Instant::now();
    let static_profile = primitives::draw_primitives_shadow_body(
        this,
        r,
        scene,
        lit,
        light_viewproj,
        lights,
        runtime,
        camera_position,
        cascade_index,
        cascade_texel_world_size,
    )?;
    let static_ms = started.elapsed().as_secs_f32() * 1000.0;
    let total_ms = total_started.elapsed().as_secs_f32() * 1000.0;

    if stage_profile {
        newengine_ulog_api::ulog::info!(
            "primitive.shadow.provider.profile: frame={} cascade={} total_ms={:.3} skinned_ms={:.3} skinned_draws={} models_ms={:.3} static_ms={:.3}",
            this.frame.frame_index,
            cascade_index,
            total_ms,
            skinned_ms,
            skinned_draws,
            models_ms,
            static_ms,
        );
    }

    Ok(PrimitiveShadowPassProfile {
        total_ms,
        skinned_ms,
        skinned_draws,
        models_ms,
        static_ms,
        static_body_ms: static_profile.total_ms,
        static_scan_ms: static_profile.scan_ms,
        static_plan_ms: static_profile.plan_ms,
        static_upload_ms: static_profile.upload_ms,
        static_replay_ms: static_profile.replay_ms,
    })
}

#[cfg(test)]
mod shadow_caster_lod_tests {
    use super::{shadow_caster_projected_radius_visible, ShadowUboViewKey};
    use std::collections::BTreeSet;

    #[test]
    fn shadow_caster_lod_keeps_near_and_rejects_subtexel_distant_casters() {
        assert!(shadow_caster_projected_radius_visible(0, 0.25, 0.05));
        assert!(shadow_caster_projected_radius_visible(1, 0.25, 0.05));
        assert!(!shadow_caster_projected_radius_visible(2, 1.0, 0.40));
        assert!(shadow_caster_projected_radius_visible(2, 1.0, 0.60));
        assert!(!shadow_caster_projected_radius_visible(3, 1.0, 0.80));
        assert!(shadow_caster_projected_radius_visible(3, 1.0, 1.00));
    }

    #[test]
    fn shadow_caster_lod_disables_itself_without_valid_texel_scale() {
        assert!(shadow_caster_projected_radius_visible(3, 0.0, 0.01));
        assert!(shadow_caster_projected_radius_visible(3, f32::NAN, 0.01));
    }
    #[test]
    fn cascade_specific_culls_can_disagree_for_the_same_caster() {
        use newengine_render_feature_api::ShadowCasterCull;
        let near = ShadowCasterCull::directional(newengine_math::Mat4::IDENTITY, 2.0, 0.1, 10.0);
        let far_view =
            newengine_math::Mat4::from_translation(newengine_math::Vec3::new(-20.0, 0.0, 0.0));
        let far = ShadowCasterCull::directional(far_view, 2.0, 0.1, 10.0);
        let caster = newengine_math::Vec3::new(0.0, 0.0, -2.0);
        assert!(near.contains_sphere(caster, 0.5));
        assert!(!far.contains_sphere(caster, 0.5));
    }

    #[test]
    fn shadow_ubo_view_keys_are_bounded_and_domain_separated() {
        let directional = (0..4)
            .map(|index| ShadowUboViewKey::directional(index).cache_discriminator())
            .collect::<BTreeSet<_>>();
        let local = (0..16)
            .map(|index| ShadowUboViewKey::local(index).cache_discriminator())
            .collect::<BTreeSet<_>>();

        assert_eq!(directional.len(), 4);
        assert_eq!(local.len(), 16);
        assert!(directional.is_disjoint(&local));
        assert_eq!(
            ShadowUboViewKey::directional(2).cache_discriminator(),
            ShadowUboViewKey::directional(2).cache_discriminator()
        );
    }
}
