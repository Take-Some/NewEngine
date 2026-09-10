use newengine_render_api::{
    FrameCameraContext, RenderGraphDesc, RendererParitySettings, VisibilitySettings,
};

use crate::{
    DrawListDesc, FramePlanExecutionMode, RenderFrameRecipe, RenderPhaseDesc,
    RuntimeRecipeBuildParams, StandardRenderPhase,
};

#[path = "graph_builder/targets.rs"]
mod targets;

pub use self::targets::*;

#[path = "graph_builder/shadows.rs"]
mod shadows;

#[path = "graph_builder/scene_3d.rs"]
mod scene_3d;

#[path = "graph_builder/postfx.rs"]
mod postfx;

#[path = "graph_builder/ui.rs"]
mod ui;

#[path = "graph_builder/lighting.rs"]
mod lighting;

#[path = "graph_builder/core.rs"]
mod core;

#[derive(Debug, Clone)]
pub struct FrameGraphBuilder {
    graph: RenderGraphDesc,
    phases: Vec<RenderPhaseDesc>,
    draw_lists: Vec<DrawListDesc>,
    target: FrameGraphTargetDesc,
    execution_mode: FramePlanExecutionMode,
    next_custom_pass: u64,
}

impl FrameGraphBuilder {
    #[inline]
    pub fn new(label: impl Into<String>, frame_index: u64, target: FrameGraphTargetDesc) -> Self {
        let mut graph = RenderGraphDesc::new(label);
        graph.frame_index = frame_index;

        let mut this = Self {
            graph,
            phases: vec![RenderPhaseDesc::standard(StandardRenderPhase::BeginFrame)],
            draw_lists: Vec::new(),
            target,
            execution_mode: FramePlanExecutionMode::ImmediateCallbacks,
            next_custom_pass: 10_000,
        };
        this.add_standard_external_resources();
        this
    }

    #[inline]
    pub fn execution_mode(mut self, mode: FramePlanExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    #[inline]
    pub fn camera(mut self, camera: FrameCameraContext) -> Self {
        self.graph.camera = camera;
        self
    }

    #[inline]
    pub fn visibility_settings(mut self, visibility: VisibilitySettings) -> Self {
        self.graph.visibility = visibility;
        self
    }

    #[inline]
    pub fn parity_settings(mut self, parity: RendererParitySettings) -> Self {
        self.graph.parity = parity;
        self
    }

    #[inline]
    pub fn draw_list(mut self, list: DrawListDesc) -> Self {
        if let Some(existing) = self.draw_lists.iter_mut().find(|it| it.kind == list.kind) {
            *existing = list;
        } else {
            self.draw_lists.push(list);
        }
        self
    }

    #[inline]
    pub fn draw_lists(mut self, lists: impl IntoIterator<Item = DrawListDesc>) -> Self {
        for list in lists {
            self = self.draw_list(list);
        }
        self
    }

    pub fn apply_runtime_recipe(
        mut self,
        recipe: &RenderFrameRecipe,
        params: RuntimeRecipeBuildParams,
    ) -> Self {
        for phase in recipe.enabled_phases() {
            self = match phase {
                StandardRenderPhase::BeginFrame | StandardRenderPhase::EndFrame => self,
                StandardRenderPhase::ShadowMap => self.shadow_map(true, params.shadow_resolution),
                StandardRenderPhase::ShadowCascadeMap => self.shadow_cascade_map(
                    true,
                    params.shadow_resolution,
                    params.shadow_cascade_count,
                ),
                StandardRenderPhase::LocalShadowMap => self.local_shadow_atlas(true),
                StandardRenderPhase::TessellationPrepare => self.tessellation_prepare(),
                StandardRenderPhase::DepthPrepass => self.depth_prepass(),
                StandardRenderPhase::VisibilityCull => self.visibility_cull(),
                StandardRenderPhase::ViewportGBuffer => self.gbuffer(),
                StandardRenderPhase::DeferredLighting => self.deferred_lighting(),
                StandardRenderPhase::ViewportForward => self.forward_opaque(),
                StandardRenderPhase::ParticleSimulation => self.particle_simulation(),
                StandardRenderPhase::HairSimulation => self.hair_simulation(),
                StandardRenderPhase::ParticleGBuffer => self.particle_gbuffer(),
                StandardRenderPhase::ParticleComposite => self.particle_composite(),
                StandardRenderPhase::Transparent => self.transparent(),
                StandardRenderPhase::Water => self.water(),
                StandardRenderPhase::PostFx => self.postfx(true),
                StandardRenderPhase::BloomExtract => self.bloom_extract(),
                StandardRenderPhase::BloomBlur => self.bloom_blur(),
                StandardRenderPhase::TaaResolve => self.taa_resolve(),
                StandardRenderPhase::MsaaResolve => self.msaa_resolve(),
                StandardRenderPhase::UiBackdropBlur => self.ui_backdrop_blur(true),
                StandardRenderPhase::UiComposite => self.ui_composite(true),
                StandardRenderPhase::DebugOverlay => self.debug_overlay(true),
            };
        }
        self
    }
}

#[cfg(test)]
#[path = "graph_builder/tests.rs"]
mod tests;
