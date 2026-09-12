use serde::{Deserialize, Serialize};

use crate::StandardRenderPhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFrameFeatureSet {
    pub shadows: bool,
    pub local_shadows: bool,
    pub deferred: bool,
    #[serde(default)]
    pub visibility_cull: bool,
    #[serde(default)]
    pub hair: bool,
    pub postfx: bool,
    #[serde(default)]
    pub bloom: bool,
    #[serde(default)]
    pub screen_space_reflections: bool,
    #[serde(default)]
    pub froxel_fog: bool,
    pub ui_composite: bool,
    pub ui_backdrop_blur: bool,
    pub debug_overlay: bool,
}

impl RuntimeFrameFeatureSet {
    #[inline]
    pub const fn forward(
        shadows: bool,
        postfx: bool,
        ui_composite: bool,
        debug_overlay: bool,
    ) -> Self {
        Self {
            shadows,
            local_shadows: false,
            deferred: false,
            visibility_cull: false,
            hair: false,
            postfx,
            bloom: postfx,
            screen_space_reflections: false,
            froxel_fog: false,
            ui_composite,
            ui_backdrop_blur: false,
            debug_overlay,
        }
    }

    #[inline]
    pub const fn deferred(
        shadows: bool,
        postfx: bool,
        ui_composite: bool,
        debug_overlay: bool,
    ) -> Self {
        Self {
            shadows,
            local_shadows: false,
            deferred: true,
            visibility_cull: false,
            hair: false,
            postfx,
            bloom: postfx,
            screen_space_reflections: false,
            froxel_fog: false,
            ui_composite,
            ui_backdrop_blur: false,
            debug_overlay,
        }
    }
    #[inline]
    pub const fn with_ui_backdrop_blur(mut self, enabled: bool) -> Self {
        self.ui_backdrop_blur = enabled;
        self
    }

    #[inline]
    pub const fn with_local_shadows(mut self, enabled: bool) -> Self {
        self.local_shadows = enabled;
        self
    }

    #[inline]
    pub const fn with_visibility_cull(mut self, enabled: bool) -> Self {
        self.visibility_cull = enabled;
        self
    }

    #[inline]
    pub const fn with_hair(mut self, enabled: bool) -> Self {
        self.hair = enabled;
        self
    }

    #[inline]
    pub const fn with_bloom(mut self, enabled: bool) -> Self {
        self.bloom = enabled;
        self
    }

    #[inline]
    pub const fn with_screen_space_reflections(mut self, enabled: bool) -> Self {
        self.screen_space_reflections = enabled;
        self
    }

    #[inline]
    pub const fn with_froxel_fog(mut self, enabled: bool) -> Self {
        self.froxel_fog = enabled;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPhaseRecipeStep {
    pub phase: StandardRenderPhase,
    pub enabled: bool,
}

impl RenderPhaseRecipeStep {
    #[inline]
    pub const fn enabled(phase: StandardRenderPhase) -> Self {
        Self {
            phase,
            enabled: true,
        }
    }

    #[inline]
    pub const fn optional(phase: StandardRenderPhase, enabled: bool) -> Self {
        Self { phase, enabled }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderFrameRecipe {
    pub label: String,
    pub features: RuntimeFrameFeatureSet,
    #[serde(default)]
    pub steps: Vec<RenderPhaseRecipeStep>,
}

impl RenderFrameRecipe {
    pub fn standard_runtime(features: RuntimeFrameFeatureSet) -> Self {
        Self::standard_runtime_with_shadow_mode(features, false)
    }

    pub fn standard_runtime_with_shadow_mode(
        features: RuntimeFrameFeatureSet,
        cascaded_shadows: bool,
    ) -> Self {
        let mut steps = Vec::with_capacity(19);
        steps.push(RenderPhaseRecipeStep::enabled(
            StandardRenderPhase::BeginFrame,
        ));
        // Hair state participates in the directional shadow atlas, so current-frame
        // simulation must complete before any shadow caster phase consumes it.
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::HairSimulation,
            features.hair,
        ));
        if cascaded_shadows {
            steps.push(RenderPhaseRecipeStep::optional(
                StandardRenderPhase::ShadowCascadeMap,
                features.shadows,
            ));
        } else {
            steps.push(RenderPhaseRecipeStep::optional(
                StandardRenderPhase::ShadowMap,
                features.shadows,
            ));
        }
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::LocalShadowMap,
            features.local_shadows,
        ));
        // Particle simulation is compute work consumed by the transparent draw list.
        // Schedule it before the scene raster chain: Vulkan compute execution ends an
        // active legacy render pass, so placing it between ForwardOpaque and Transparent
        // would force a direct-surface continuation to reopen the swapchain CLEAR pass
        // and erase the opaque scene that was just rendered.
        steps.push(RenderPhaseRecipeStep::enabled(
            StandardRenderPhase::ParticleSimulation,
        ));
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::VisibilityCull,
            features.visibility_cull,
        ));
        // GPU-driven visibility is not part of the unconditional production recipe yet.
        // It must be injected only after backend capability negotiation and synchronized
        // deployment of the matching renderer provider; otherwise an older installed backend
        // can reject the new phase and leave only the frame clear visible.
        if features.deferred {
            // Native GBuffer writes the authoritative scene depth together with the MRTs.
            // A separate depth prepass would create a second depth resource/domain and is
            // therefore forbidden for the standard deferred path.
            steps.push(RenderPhaseRecipeStep::enabled(
                StandardRenderPhase::ViewportGBuffer,
            ));
            steps.push(RenderPhaseRecipeStep::enabled(
                StandardRenderPhase::DeferredLighting,
            ));
            // Deferred extraction still records OpaqueForward for roles that are not
            // representable in the GBuffer (sky, view-model and other forward overlays).
            // Give that bucket a real graph scope before transparent/postfx so it can
            // never leak into VulkanRenderApi::end_frame()'s generic residual flush.
            steps.push(RenderPhaseRecipeStep::enabled(
                StandardRenderPhase::ViewportForward,
            ));
        } else {
            steps.push(RenderPhaseRecipeStep::enabled(
                StandardRenderPhase::ViewportForward,
            ));
        }
        steps.push(RenderPhaseRecipeStep::enabled(
            StandardRenderPhase::ParticleGBuffer,
        ));
        steps.push(RenderPhaseRecipeStep::enabled(
            StandardRenderPhase::ParticleComposite,
        ));
        steps.push(RenderPhaseRecipeStep::enabled(
            StandardRenderPhase::Transparent,
        ));
        // Volumetric fog is a graph-owned logical 3D volume. The current provider
        // packs the Z slices into a 2D atlas, but the scheduling contract remains volumetric.
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::FroxelFog,
            features.postfx && features.froxel_fog,
        ));
        // SSR is deferred-only and produces a side-signal from the authoritative
        // GBuffer. Root PostFX composites it before atmosphere/lens/display.
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::ScreenSpaceReflections,
            features.deferred && features.postfx && features.screen_space_reflections,
        ));
        // Bloom is a linear-HDR side chain. It must complete before the root PostFX
        // pass so tonemap can composite its result rather than re-blurring display color.
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::BloomExtract,
            features.postfx && features.bloom,
        ));
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::PostFx,
            features.postfx,
        ));
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::UiBackdropBlur,
            features.ui_composite && features.ui_backdrop_blur,
        ));
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::UiComposite,
            features.ui_composite,
        ));
        steps.push(RenderPhaseRecipeStep::optional(
            StandardRenderPhase::DebugOverlay,
            features.debug_overlay,
        ));
        steps.push(RenderPhaseRecipeStep::enabled(
            StandardRenderPhase::EndFrame,
        ));

        Self {
            label: "runtime.standard_frame".to_owned(),
            features,
            steps,
        }
    }

    #[inline]
    pub fn enabled_phases(&self) -> impl Iterator<Item = StandardRenderPhase> + '_ {
        self.steps
            .iter()
            .filter(|step| step.enabled)
            .map(|step| step.phase)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeRecipeBuildParams {
    pub shadow_resolution: u32,
    pub shadow_cascade_count: u32,
    pub froxel_tile_size_px: u32,
    pub froxel_depth_slices: u32,
}

impl RuntimeRecipeBuildParams {
    #[inline]
    pub const fn new(shadow_resolution: u32) -> Self {
        Self {
            shadow_resolution,
            shadow_cascade_count: 1,
            froxel_tile_size_px: 16,
            froxel_depth_slices: 64,
        }
    }

    #[inline]
    pub const fn with_shadow_cascade_count(mut self, cascade_count: u32) -> Self {
        self.shadow_cascade_count = cascade_count;
        self
    }

    #[inline]
    pub const fn with_froxel_grid(mut self, tile_size_px: u32, depth_slices: u32) -> Self {
        self.froxel_tile_size_px = tile_size_px;
        self.froxel_depth_slices = depth_slices;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_compute_precedes_forward_surface_raster_chain() {
        let recipe = RenderFrameRecipe::standard_runtime(RuntimeFrameFeatureSet::forward(
            true, false, false, false,
        ));
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        let particle = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ParticleSimulation)
            .expect("particle simulation phase");
        let forward = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ViewportForward)
            .expect("forward phase");
        let particle_gbuffer = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ParticleGBuffer)
            .expect("particle gbuffer phase");
        let particle_composite = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ParticleComposite)
            .expect("particle composite phase");
        let transparent = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::Transparent)
            .expect("transparent phase");

        assert!(
            particle < forward,
            "compute must finish before surface raster begins"
        );
        assert_eq!(
            particle_gbuffer,
            forward + 1,
            "particle GBuffer is the first graphics phase after opaque"
        );
        assert_eq!(
            particle_composite,
            particle_gbuffer + 1,
            "particle composite must immediately consume the particle GBuffer"
        );
        assert_eq!(
            transparent,
            particle_composite + 1,
            "transparent continues after the LOAD-preserving particle composite"
        );
    }

    #[test]
    fn standard_runtime_does_not_enable_visibility_cull_without_backend_gate() {
        let recipe = RenderFrameRecipe::standard_runtime(RuntimeFrameFeatureSet::deferred(
            true, false, false, false,
        ));
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        assert!(
            !phases.contains(&StandardRenderPhase::VisibilityCull),
            "visibility cull must remain opt-in until the negotiated backend data-plane is deployed"
        );
        assert!(phases.contains(&StandardRenderPhase::ViewportGBuffer));
    }

    #[test]
    fn visibility_cull_precedes_gbuffer_when_opted_in() {
        let recipe = RenderFrameRecipe::standard_runtime(
            RuntimeFrameFeatureSet::deferred(true, false, false, false)
                .with_visibility_cull(true),
        );
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        let particle = phases.iter().position(|phase| *phase == StandardRenderPhase::ParticleSimulation).expect("particle simulation phase");
        let visibility = phases.iter().position(|phase| *phase == StandardRenderPhase::VisibilityCull).expect("visibility cull phase");
        let gbuffer = phases.iter().position(|phase| *phase == StandardRenderPhase::ViewportGBuffer).expect("gbuffer phase");
        assert!(particle < visibility && visibility < gbuffer);
    }

    #[test]
    fn froxel_fog_precedes_ssr_bloom_and_root_postfx_when_enabled() {
        let recipe = RenderFrameRecipe::standard_runtime(
            RuntimeFrameFeatureSet::deferred(true, true, false, false)
                .with_froxel_fog(true)
                .with_screen_space_reflections(true),
        );
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        let fog = phases
            .iter()
            .position(|p| *p == StandardRenderPhase::FroxelFog)
            .expect("froxel fog phase");
        let ssr = phases
            .iter()
            .position(|p| *p == StandardRenderPhase::ScreenSpaceReflections)
            .expect("SSR phase");
        let bloom = phases
            .iter()
            .position(|p| *p == StandardRenderPhase::BloomExtract)
            .expect("bloom phase");
        let postfx = phases
            .iter()
            .position(|p| *p == StandardRenderPhase::PostFx)
            .expect("root postfx phase");
        assert!(fog < ssr && ssr < bloom && bloom < postfx);
    }

    #[test]
    fn ssr_is_deferred_only_and_precedes_bloom_and_postfx() {
        let deferred = RenderFrameRecipe::standard_runtime(
            RuntimeFrameFeatureSet::deferred(true, true, false, false)
                .with_screen_space_reflections(true),
        );
        let phases = deferred.enabled_phases().collect::<Vec<_>>();
        let ssr = phases.iter().position(|p| *p == StandardRenderPhase::ScreenSpaceReflections).unwrap();
        let bloom = phases.iter().position(|p| *p == StandardRenderPhase::BloomExtract).unwrap();
        let postfx = phases.iter().position(|p| *p == StandardRenderPhase::PostFx).unwrap();
        assert!(ssr < bloom && bloom < postfx);

        let forward = RenderFrameRecipe::standard_runtime(
            RuntimeFrameFeatureSet::forward(true, true, false, false)
                .with_screen_space_reflections(true),
        );
        assert!(!forward.enabled_phases().any(|p| p == StandardRenderPhase::ScreenSpaceReflections));
    }

    #[test]
    fn bloom_side_chain_precedes_root_postfx_when_enabled() {
        let recipe = RenderFrameRecipe::standard_runtime(
            RuntimeFrameFeatureSet::forward(true, true, false, false).with_bloom(true),
        );
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        let bloom = phases.iter().position(|p| *p == StandardRenderPhase::BloomExtract).unwrap();
        let postfx = phases.iter().position(|p| *p == StandardRenderPhase::PostFx).unwrap();
        assert!(bloom < postfx);
    }

    #[test]
    fn hair_compute_precedes_cascaded_shadow_when_enabled() {
        let recipe = RenderFrameRecipe::standard_runtime_with_shadow_mode(
            RuntimeFrameFeatureSet::forward(true, false, false, false).with_hair(true),
            true,
        );
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        let hair = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::HairSimulation)
            .expect("hair simulation phase");
        let shadow = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ShadowCascadeMap)
            .expect("cascaded shadow phase");
        assert!(
            hair < shadow,
            "current-frame hair state must exist before CSM raster"
        );
    }

    #[test]
    fn hair_compute_precedes_surface_raster_when_enabled() {
        let recipe = RenderFrameRecipe::standard_runtime(
            RuntimeFrameFeatureSet::forward(true, false, false, false).with_hair(true),
        );
        let phases = recipe.enabled_phases().collect::<Vec<_>>();
        let hair = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::HairSimulation)
            .expect("hair simulation phase");
        let forward = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ViewportForward)
            .expect("forward phase");
        let particle_gbuffer = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ParticleGBuffer)
            .expect("particle gbuffer phase");
        let particle_composite = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::ParticleComposite)
            .expect("particle composite phase");
        let transparent = phases
            .iter()
            .position(|phase| *phase == StandardRenderPhase::Transparent)
            .expect("transparent phase");

        assert!(
            hair < forward,
            "hair compute must finish before surface raster begins"
        );
        assert_eq!(particle_gbuffer, forward + 1);
        assert_eq!(particle_composite, particle_gbuffer + 1);
        assert_eq!(transparent, particle_composite + 1);
    }
}
