use newengine_render_api::{
    Extent2D, FrameCameraContext, RenderGraphPassId, RenderTargetId, TextureFormat, UiLayerDomain,
};

use crate::{
    DrawListDesc, FrameGraphBuilder, FrameGraphTargetDesc, FramePlanExecutionMode, RenderFramePlan,
    RenderFrameRecipe, RenderPhaseDesc, RuntimeFrameFeatureSet, RuntimeRecipeBuildParams,
    StandardRenderPhase,
};

#[derive(Debug, Clone)]
pub struct StandardRuntimePipelineDesc {
    pub frame_index: u64,
    pub surface_extent: Extent2D,
    pub viewport_extent: Extent2D,
    pub camera: FrameCameraContext,
    pub viewport_is_surface: bool,
    pub viewport_render_target: Option<RenderTargetId>,
    pub shadow_render_target: Option<RenderTargetId>,
    pub local_shadow_render_target: Option<RenderTargetId>,
    pub local_shadow_extent: Extent2D,
    pub shadow_enabled: bool,
    pub local_shadow_enabled: bool,
    pub shadow_resolution: u32,
    pub shadow_cascade_count: u32,
    pub deferred: bool,
    pub visibility_cull_enabled: bool,
    pub hdr_scene_enabled: bool,
    pub hair_enabled: bool,
    pub postfx_enabled: bool,
    pub bloom_enabled: bool,
    pub screen_space_reflections_enabled: bool,
    pub froxel_fog_enabled: bool,
    pub froxel_tile_size_px: u32,
    pub froxel_depth_slices: u32,
    pub ui_enabled: bool,
    /// Ordered renderer-owned UI domains. Empty keeps the legacy single UI pass.
    pub ui_layers: Vec<UiLayerDomain>,
    pub ui_backdrop_blur_enabled: bool,
    pub debug_overlay_enabled: bool,
    pub execution_mode: FramePlanExecutionMode,
    pub draw_lists: Vec<DrawListDesc>,
}

impl StandardRuntimePipelineDesc {
    #[inline]
    pub fn new(frame_index: u64, surface_extent: Extent2D, viewport_extent: Extent2D) -> Self {
        Self {
            frame_index,
            surface_extent,
            viewport_extent,
            camera: FrameCameraContext::default(),
            viewport_is_surface: false,
            viewport_render_target: None,
            shadow_render_target: None,
            local_shadow_render_target: None,
            local_shadow_extent: Extent2D::new(1, 1),
            shadow_enabled: true,
            local_shadow_enabled: false,
            shadow_resolution: 2048,
            shadow_cascade_count: 1,
            deferred: false,
            visibility_cull_enabled: false,
            hdr_scene_enabled: true,
            hair_enabled: false,
            postfx_enabled: true,
            bloom_enabled: true,
            screen_space_reflections_enabled: false,
            froxel_fog_enabled: false,
            froxel_tile_size_px: 16,
            froxel_depth_slices: 64,
            ui_enabled: true,
            ui_layers: Vec::new(),
            ui_backdrop_blur_enabled: false,
            debug_overlay_enabled: true,
            execution_mode: FramePlanExecutionMode::ImmediateCallbacks,
            draw_lists: Vec::new(),
        }
    }

    #[inline]
    pub fn camera(mut self, camera: FrameCameraContext) -> Self {
        self.camera = camera.sanitized();
        self
    }

    #[inline]
    pub fn viewport_is_surface(mut self, value: bool) -> Self {
        self.viewport_is_surface = value;
        self
    }

    #[inline]
    pub fn viewport_render_target(mut self, target: Option<RenderTargetId>) -> Self {
        self.viewport_render_target = target;
        self
    }

    #[inline]
    pub fn shadow_render_target(mut self, target: Option<RenderTargetId>) -> Self {
        self.shadow_render_target = target;
        self
    }

    #[inline]
    pub fn local_shadow(
        mut self,
        enabled: bool,
        target: Option<RenderTargetId>,
        extent: Extent2D,
    ) -> Self {
        self.local_shadow_enabled = enabled;
        self.local_shadow_render_target = target;
        self.local_shadow_extent = extent;
        self
    }

    #[inline]
    pub fn shadow(mut self, enabled: bool, resolution: u32) -> Self {
        self.shadow_enabled = enabled;
        self.shadow_resolution = resolution;
        self
    }

    #[inline]
    pub fn shadow_cascades(mut self, cascade_count: u32) -> Self {
        self.shadow_cascade_count = cascade_count.clamp(1, 8);
        self
    }

    #[inline]
    pub fn deferred(mut self, value: bool) -> Self {
        self.deferred = value;
        self
    }

    #[inline]
    pub fn visibility_cull(mut self, enabled: bool) -> Self {
        self.visibility_cull_enabled = enabled;
        self
    }

    #[inline]
    pub fn hdr_scene(mut self, enabled: bool) -> Self {
        self.hdr_scene_enabled = enabled;
        self
    }

    #[inline]
    pub fn hair(mut self, enabled: bool) -> Self {
        self.hair_enabled = enabled;
        self
    }

    #[inline]
    pub fn postfx(mut self, enabled: bool) -> Self {
        self.postfx_enabled = enabled;
        self
    }

    #[inline]
    pub fn bloom(mut self, enabled: bool) -> Self {
        self.bloom_enabled = enabled;
        self
    }

    #[inline]
    pub fn screen_space_reflections(mut self, enabled: bool) -> Self {
        self.screen_space_reflections_enabled = enabled;
        self
    }

    #[inline]
    pub fn froxel_fog(mut self, enabled: bool, tile_size_px: u32, depth_slices: u32) -> Self {
        self.froxel_fog_enabled = enabled;
        self.froxel_tile_size_px = tile_size_px.clamp(8, 64);
        self.froxel_depth_slices = depth_slices.clamp(16, 128);
        self
    }

    #[inline]
    pub fn ui(mut self, enabled: bool) -> Self {
        self.ui_enabled = enabled;
        self
    }

    #[inline]
    pub fn ui_layers(mut self, domains: impl IntoIterator<Item = UiLayerDomain>) -> Self {
        self.ui_layers = normalized_ui_domains(domains);
        self
    }

    #[inline]
    pub fn ui_backdrop_blur(mut self, enabled: bool) -> Self {
        self.ui_backdrop_blur_enabled = enabled;
        self
    }

    #[inline]
    pub fn debug_overlay(mut self, enabled: bool) -> Self {
        self.debug_overlay_enabled = enabled;
        self
    }

    #[inline]
    pub fn draw_lists(mut self, lists: impl IntoIterator<Item = DrawListDesc>) -> Self {
        self.draw_lists = lists.into_iter().collect();
        self
    }
}

#[inline]
pub fn standard_runtime_frame(desc: StandardRuntimePipelineDesc) -> RenderFramePlan {
    let mut target = FrameGraphTargetDesc::new(
        desc.surface_extent,
        desc.viewport_extent,
        desc.viewport_is_surface,
    );
    target.color_format = TextureFormat::Bgra8Unorm;
    target.scene_color_format = if desc.hdr_scene_enabled {
        TextureFormat::Rgba16Float
    } else {
        TextureFormat::Bgra8Unorm
    };
    target.depth_format = TextureFormat::Depth32Float;
    target.hdr_scene_enabled = desc.hdr_scene_enabled;
    target.offscreen_scene_enabled = desc.deferred || desc.hdr_scene_enabled || desc.postfx_enabled;
    target.viewport_render_target = desc.viewport_render_target;
    target.shadow_render_target = desc.shadow_render_target;
    target.shadow_extent = directional_shadow_atlas_extent(
        desc.shadow_resolution,
        desc.shadow_cascade_count,
    );
    target.local_shadow_render_target = desc.local_shadow_render_target;
    target.local_shadow_extent = desc.local_shadow_extent;

    let features = if desc.deferred {
        RuntimeFrameFeatureSet::deferred(
            desc.shadow_enabled,
            desc.postfx_enabled,
            desc.ui_enabled,
            desc.debug_overlay_enabled,
        )
        .with_visibility_cull(desc.visibility_cull_enabled)
        .with_ui_backdrop_blur(desc.ui_backdrop_blur_enabled)
        .with_local_shadows(desc.local_shadow_enabled)
        .with_hair(desc.hair_enabled)
        .with_bloom(desc.bloom_enabled)
        .with_screen_space_reflections(desc.screen_space_reflections_enabled)
        .with_froxel_fog(desc.froxel_fog_enabled)
    } else {
        RuntimeFrameFeatureSet::forward(
            desc.shadow_enabled,
            desc.postfx_enabled,
            desc.ui_enabled,
            desc.debug_overlay_enabled,
        )
        .with_visibility_cull(desc.visibility_cull_enabled)
        .with_ui_backdrop_blur(desc.ui_backdrop_blur_enabled)
        .with_local_shadows(desc.local_shadow_enabled)
        .with_hair(desc.hair_enabled)
        .with_bloom(desc.bloom_enabled)
        .with_screen_space_reflections(desc.screen_space_reflections_enabled)
        .with_froxel_fog(desc.froxel_fog_enabled)
    };
    let recipe = RenderFrameRecipe::standard_runtime_with_shadow_mode(
        features,
        desc.shadow_cascade_count > 1,
    );
    let label = recipe.label.clone();

    let mut plan = FrameGraphBuilder::new(label, desc.frame_index, target)
        .camera(desc.camera)
        .execution_mode(desc.execution_mode)
        .draw_lists(desc.draw_lists)
        .apply_runtime_recipe(
            &recipe,
            RuntimeRecipeBuildParams::new(desc.shadow_resolution)
                .with_shadow_cascade_count(desc.shadow_cascade_count)
                .with_froxel_grid(desc.froxel_tile_size_px, desc.froxel_depth_slices),
        )
        .submit();
    expand_ui_composite_layers(&mut plan, &desc.ui_layers);
    declare_graph_draw_lists(&mut plan);
    plan
}

fn declare_graph_draw_lists(plan: &mut RenderFramePlan) {
    let mut required = Vec::new();
    for pass in &plan.graph.passes {
        for &kind in &pass.draw_lists {
            if !required.contains(&kind) {
                required.push(kind);
            }
        }
    }
    for kind in required {
        if !plan.draw_lists.iter().any(|desc| desc.kind == kind) {
            plan.draw_lists.push(DrawListDesc::standard(kind));
        }
    }
}

/// Minimal presentation graph used by bootstrap, UI-only tools and degraded recovery.
///
/// This deliberately skips scene/depth/postfx construction. Retained UI domains are still
/// expanded into the same `ui_composite.<domain>` passes used by normal playable frames,
/// so there is no separate renderer-side singleton UI path.
#[inline]
pub fn ui_layer_only_frame(
    frame_index: u64,
    surface_extent: Extent2D,
    domains: impl IntoIterator<Item = UiLayerDomain>,
) -> RenderFramePlan {
    let domains = normalized_ui_domains(domains);
    let mut target = FrameGraphTargetDesc::new(surface_extent, surface_extent, true)
        .with_hdr_scene(false)
        .with_offscreen_scene(false)
        .with_scene_color_format(TextureFormat::Bgra8Unorm);
    target.color_format = TextureFormat::Bgra8Unorm;
    target.depth_format = TextureFormat::Depth32Float;

    let mut plan = FrameGraphBuilder::new("ui_layer_only", frame_index, target)
        .ui_composite(!domains.is_empty())
        .submit();
    expand_ui_composite_layers(&mut plan, &domains);
    plan
}

#[inline]
fn directional_shadow_atlas_extent(resolution: u32, cascade_count: u32) -> Extent2D {
    let resolution = resolution.clamp(256, 16_284);
    let cascades = cascade_count.clamp(1, 8);
    let columns = if cascades <= 1 {
        1
    } else if cascades <= 4 {
        2
    } else {
        4
    };
    let rows = cascades.div_ceil(columns).max(1);
    Extent2D::new(
        resolution.saturating_mul(columns),
        resolution.saturating_mul(rows),
    )
}

fn normalized_ui_domains(domains: impl IntoIterator<Item = UiLayerDomain>) -> Vec<UiLayerDomain> {
    let mut domains = domains.into_iter().collect::<Vec<_>>();
    domains.sort();
    domains.dedup();
    domains.sort_by_key(|domain| domain.default_composition_order());
    domains
}

fn expand_ui_composite_layers(plan: &mut RenderFramePlan, domains: &[UiLayerDomain]) {
    if domains.is_empty() {
        return;
    }
    let Some(pass_index) = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::UiComposite)
    else {
        return;
    };

    let base_pass = plan.graph.passes[pass_index].clone();
    let layer_passes = domains
        .iter()
        .enumerate()
        .map(|(index, domain)| {
            let mut pass = base_pass.clone();
            pass.id = RenderGraphPassId(900 + index as u64);
            pass.label = format!("ui_composite.{}", domain.as_str());
            // UiLayerDrawPacket is an envelope payload, not a recorded command draw-list.
            // Layered UI therefore never participates in scene draw-list routing.
            pass.draw_lists.clear();
            pass
        })
        .collect::<Vec<_>>();
    plan.graph
        .passes
        .splice(pass_index..=pass_index, layer_passes);

    if let Some(phase_index) = plan
        .phases
        .iter()
        .position(|phase| phase.phase == StandardRenderPhase::UiComposite)
    {
        let phases = domains
            .iter()
            .enumerate()
            .map(|(index, domain)| RenderPhaseDesc {
                phase: StandardRenderPhase::UiComposite,
                pass_id: Some(RenderGraphPassId(900 + index as u64)),
                label: format!("ui_composite.{}", domain.as_str()),
            })
            .collect::<Vec<_>>();
        plan.phases.splice(phase_index..=phase_index, phases);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use newengine_render_api::RenderGraphResourceSemantic;

    #[test]
    fn ldr_standard_pipeline_uses_display_compatible_scene_color() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(1, Extent2D::new(1600, 900), Extent2D::new(488, 236))
                .viewport_render_target(Some(RenderTargetId(
                    std::num::NonZeroU32::new(77).unwrap(),
                )))
                .hdr_scene(false)
                .postfx(false)
                .shadow(false, 1),
        );
        let viewport = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.semantic == RenderGraphResourceSemantic::ViewportColor)
            .expect("viewport color resource");
        assert_eq!(viewport.format, Some(TextureFormat::Bgra8Unorm));
        assert!(!plan
            .graph
            .resources
            .iter()
            .any(|resource| { resource.semantic == RenderGraphResourceSemantic::SceneHdrColor }));
    }

    #[test]
    fn direct_surface_resources_use_srgb_presentation_contract() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(2, Extent2D::new(1600, 900), Extent2D::new(1600, 900))
                .viewport_is_surface(true)
                .hdr_scene(false)
                .postfx(false)
                .shadow(false, 1),
        );
        let surface = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.semantic == RenderGraphResourceSemantic::SurfaceColor)
            .expect("surface color resource");
        let viewport = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.semantic == RenderGraphResourceSemantic::ViewportColor)
            .expect("surface viewport color resource");
        assert_eq!(surface.format, Some(TextureFormat::Bgra8Srgb));
        assert_eq!(viewport.format, Some(TextureFormat::Bgra8Srgb));
    }

    #[test]
    fn ldr_postfx_uses_sampleable_bgra_scene_color_and_depth() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(2, Extent2D::new(1600, 900), Extent2D::new(1600, 900))
                .viewport_is_surface(true)
                .hdr_scene(false)
                .postfx(true)
                .ui(false)
                .debug_overlay(false),
        );
        let scene = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.semantic == RenderGraphResourceSemantic::SceneHdrColor)
            .expect("LDR postFX still requires a sampleable offscreen scene color");
        assert_eq!(scene.format, Some(TextureFormat::Bgra8Unorm));
        assert!(plan.graph.resources.iter().any(|resource| {
            resource.semantic == RenderGraphResourceSemantic::ViewportDepth
                && resource.lifetime
                    == newengine_render_api::RenderGraphResourceLifetime::TransientFrame
        }));
        let forward = plan
            .graph
            .passes
            .iter()
            .find(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::ForwardOpaque)
            .expect("forward pass");
        assert!(forward
            .writes
            .iter()
            .any(|write| write.resource == crate::RG_SCENE_HDR_COLOR));
    }

    #[test]
    fn standard_pipeline_declares_every_graph_draw_list_route() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(3, Extent2D::new(1600, 900), Extent2D::new(1600, 900))
                .viewport_is_surface(true)
                .shadow(false, 1)
                .postfx(false)
                .ui(false)
                .debug_overlay(false)
                .draw_lists([DrawListDesc::standard(
                    newengine_render_api::RenderDrawListKind::OpaqueForward,
                )]),
        );
        let report = plan.validate_draw_list_routes();
        assert!(
            report.errors.is_empty(),
            "route errors: {:?}",
            report.errors
        );
        assert!(
            report
                .warnings
                .iter()
                .all(|issue| issue.code != "draw_list.route_without_declared_list"),
            "route warnings: {:?}",
            report.warnings
        );
        assert!(plan
            .draw_lists
            .iter()
            .any(|desc| { desc.kind == newengine_render_api::RenderDrawListKind::Transparent }));
    }

    #[test]
    fn hdr_standard_pipeline_keeps_float_scene_color() {
        let plan = standard_runtime_frame(StandardRuntimePipelineDesc::new(
            1,
            Extent2D::new(1600, 900),
            Extent2D::new(488, 236),
        ));
        let scene = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.semantic == RenderGraphResourceSemantic::SceneHdrColor)
            .expect("HDR scene color resource");
        assert_eq!(scene.format, Some(TextureFormat::Rgba16Float));
    }

    #[test]
    fn deferred_ssr_declares_gbuffer_dependencies_and_root_composite_input() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(
                41,
                Extent2D::new(1920, 1080),
                Extent2D::new(1920, 1080),
            )
            .deferred(true)
            .postfx(true)
            .screen_space_reflections(true)
            .ui(false)
            .debug_overlay(false),
        );

        let ssr_resource = plan
            .graph
            .resources
            .iter()
            .find(|resource| {
                resource.semantic == newengine_render_api::RenderGraphResourceSemantic::ScreenSpaceReflection
            })
            .expect("deferred SSR must allocate a reflection side-signal");
        assert_eq!(ssr_resource.format, Some(TextureFormat::Rgba16Float));

        let ssr_pass = plan
            .graph
            .passes
            .iter()
            .find(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::ScreenSpaceReflections)
            .expect("SSR pass");
        for id in [
            crate::RG_GBUFFER_DEPTH,
            crate::RG_GBUFFER_NORMAL,
            crate::RG_GBUFFER_MATERIAL,
        ] {
            assert!(
                ssr_pass.reads.iter().any(|read| read.resource == id),
                "SSR must read GBuffer resource {:?}",
                id
            );
        }

        let postfx = plan
            .graph
            .passes
            .iter()
            .find(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::PostFx)
            .expect("root PostFX pass");
        assert!(postfx.reads.iter().any(|read| read.resource == crate::RG_SSR_REFLECTION));

        let ssr_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::ScreenSpaceReflections)
            .unwrap();
        let postfx_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::PostFx)
            .unwrap();
        assert!(ssr_index < postfx_index);
    }

    #[test]
    fn forward_pipeline_never_materializes_ssr_even_if_requested() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(
                42,
                Extent2D::new(1280, 720),
                Extent2D::new(1280, 720),
            )
            .deferred(false)
            .postfx(true)
            .screen_space_reflections(true)
            .ui(false)
            .debug_overlay(false),
        );
        assert!(!plan.graph.passes.iter().any(|pass| {
            pass.kind == newengine_render_api::RenderGraphPassKind::ScreenSpaceReflections
        }));
        assert!(!plan.graph.resources.iter().any(|resource| {
            resource.semantic == newengine_render_api::RenderGraphResourceSemantic::ScreenSpaceReflection
        }));
    }

    #[test]
    fn enabled_bloom_is_a_linear_hdr_side_chain_before_root_postfx() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(
                43,
                Extent2D::new(1920, 1080),
                Extent2D::new(1920, 1080),
            )
            .postfx(true)
            .bloom(true)
            .ui(false)
            .debug_overlay(false),
        );
        let bloom = plan
            .graph
            .resources
            .iter()
            .find(|resource| {
                resource.semantic == newengine_render_api::RenderGraphResourceSemantic::BloomComposite
            })
            .expect("bloom side-chain resource");
        assert_eq!(bloom.format, Some(TextureFormat::Rgba16Float));
        let bloom_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::BloomExtract)
            .unwrap();
        let postfx_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::PostFx)
            .unwrap();
        assert!(bloom_index < postfx_index);
        let root = &plan.graph.passes[postfx_index];
        assert!(root.reads.iter().any(|read| read.resource == crate::RG_BLOOM_COMPOSITE));
    }

    #[test]
    fn froxel_fog_allocates_packed_volume_and_root_postfx_consumes_it() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(
                44,
                Extent2D::new(1920, 1080),
                Extent2D::new(1920, 1080),
            )
            .postfx(true)
            .bloom(true)
            .froxel_fog(true, 16, 64)
            .ui(false)
            .debug_overlay(false),
        );

        let volume = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.semantic == RenderGraphResourceSemantic::FroxelFog)
            .expect("froxel volume resource");
        assert_eq!(volume.format, Some(TextureFormat::Rgba16Float));
        assert_eq!(volume.extent, Some(Extent2D::new(960, 544)));

        let fog_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::FroxelFog)
            .expect("froxel fog pass");
        let bloom_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::BloomExtract)
            .expect("bloom pass");
        let postfx_index = plan
            .graph
            .passes
            .iter()
            .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::PostFx)
            .expect("root postfx pass");
        assert!(fog_index < bloom_index && bloom_index < postfx_index);

        let fog_pass = &plan.graph.passes[fog_index];
        assert!(fog_pass.reads.iter().any(|read| {
            plan.graph
                .resources
                .iter()
                .find(|resource| resource.id == read.resource)
                .is_some_and(|resource| {
                    matches!(
                        resource.semantic,
                        RenderGraphResourceSemantic::ViewportDepth
                            | RenderGraphResourceSemantic::GBufferDepth
                    )
                })
        }));
        assert!(fog_pass
            .writes
            .iter()
            .any(|write| write.resource == crate::RG_FROXEL_FOG_VOLUME));

        let root = &plan.graph.passes[postfx_index];
        assert!(root
            .reads
            .iter()
            .any(|read| read.resource == crate::RG_FROXEL_FOG_VOLUME));
    }

    #[test]
    fn froxel_fog_reads_cached_external_csm_atlas_without_shadow_writer() {
        let shadow_target = RenderTargetId(std::num::NonZeroU32::new(91).unwrap());
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(
                45,
                Extent2D::new(1920, 1080),
                Extent2D::new(1920, 1080),
            )
            .shadow(false, 2048)
            .shadow_cascades(4)
            .shadow_render_target(Some(shadow_target))
            .postfx(true)
            .froxel_fog(true, 16, 64)
            .ui(false)
            .debug_overlay(false),
        );

        assert!(!plan.graph.passes.iter().any(|pass| {
            matches!(
                pass.kind,
                newengine_render_api::RenderGraphPassKind::ShadowMap
                    | newengine_render_api::RenderGraphPassKind::ShadowCascadeMap
            )
        }));
        let shadow = plan
            .graph
            .resources
            .iter()
            .find(|resource| resource.id == crate::RG_SHADOW_MAP)
            .expect("cached CSM resource");
        assert_eq!(shadow.semantic, RenderGraphResourceSemantic::ShadowMap);
        assert_eq!(shadow.extent, Some(Extent2D::new(4096, 4096)));
        assert_eq!(
            shadow.external,
            Some(newengine_render_api::RenderGraphExternalResource::RenderTarget(shadow_target))
        );
        let froxel = plan
            .graph
            .passes
            .iter()
            .find(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::FroxelFog)
            .expect("froxel pass");
        assert!(froxel
            .reads
            .iter()
            .any(|read| read.resource == crate::RG_SHADOW_MAP));
    }

    #[test]
    fn layered_ui_expands_to_ordered_render_graph_composite_passes() {
        let plan = standard_runtime_frame(
            StandardRuntimePipelineDesc::new(
                12,
                Extent2D::new(1600, 900),
                Extent2D::new(1600, 900),
            )
            .viewport_is_surface(true)
            .shadow(false, 1)
            .postfx(false)
            .ui(true)
            .ui_layers([
                UiLayerDomain::Debug,
                UiLayerDomain::System,
                UiLayerDomain::GameViewport,
                UiLayerDomain::Editor,
            ])
            .debug_overlay(false),
        );

        let ui_passes = plan
            .graph
            .passes
            .iter()
            .filter(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::UiComposite)
            .collect::<Vec<_>>();
        assert_eq!(ui_passes.len(), 4);
        assert_eq!(
            ui_passes
                .iter()
                .map(|pass| pass.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "ui_composite.game_viewport",
                "ui_composite.editor",
                "ui_composite.system",
                "ui_composite.debug",
            ]
        );
        assert_eq!(
            ui_passes.iter().map(|pass| pass.id.0).collect::<Vec<_>>(),
            vec![900, 901, 902, 903]
        );
        assert!(ui_passes.iter().all(|pass| pass.draw_lists.is_empty()));
    }

    #[test]
    fn ui_layer_only_frame_has_no_scene_passes_and_keeps_domain_order() {
        let plan = ui_layer_only_frame(
            23,
            Extent2D::new(1280, 720),
            [UiLayerDomain::Debug, UiLayerDomain::System],
        );
        assert!(plan.graph.passes.iter().all(|pass| {
            matches!(
                pass.kind,
                newengine_render_api::RenderGraphPassKind::UiComposite
            )
        }));
        assert_eq!(
            plan.graph
                .passes
                .iter()
                .map(|pass| pass.label.as_str())
                .collect::<Vec<_>>(),
            vec!["ui_composite.system", "ui_composite.debug"]
        );
        assert!(!plan
            .graph
            .resources
            .iter()
            .any(|resource| { resource.semantic == RenderGraphResourceSemantic::SceneHdrColor }));
    }
}
