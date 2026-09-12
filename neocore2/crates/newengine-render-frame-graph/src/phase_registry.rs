use newengine_render_api::{RenderGraphPassId, RenderGraphPassKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StandardRenderPhase {
    BeginFrame,
    ShadowMap,
    ShadowCascadeMap,
    LocalShadowMap,
    TessellationPrepare,
    DepthPrepass,
    VisibilityCull,
    ViewportGBuffer,
    DeferredLighting,
    ViewportForward,
    ParticleSimulation,
    HairSimulation,
    ParticleGBuffer,
    ParticleComposite,
    Transparent,
    Water,
    FroxelFog,
    ScreenSpaceReflections,
    PostFx,
    BloomExtract,
    BloomBlur,
    TaaResolve,
    MsaaResolve,
    UiBackdropBlur,
    UiComposite,
    DebugOverlay,
    EndFrame,
}

impl StandardRenderPhase {
    #[inline]
    pub const fn pass_kind(self) -> Option<RenderGraphPassKind> {
        match self {
            Self::BeginFrame | Self::EndFrame => None,
            Self::ShadowMap => Some(RenderGraphPassKind::ShadowMap),
            Self::ShadowCascadeMap => Some(RenderGraphPassKind::ShadowCascadeMap),
            Self::LocalShadowMap => Some(RenderGraphPassKind::LocalShadowMap),
            Self::TessellationPrepare => Some(RenderGraphPassKind::TessellationPrepare),
            Self::DepthPrepass => Some(RenderGraphPassKind::DepthPrepass),
            Self::VisibilityCull => Some(RenderGraphPassKind::VisibilityCull),
            Self::ViewportGBuffer => Some(RenderGraphPassKind::GBuffer),
            Self::DeferredLighting => Some(RenderGraphPassKind::DeferredLighting),
            Self::ViewportForward => Some(RenderGraphPassKind::ForwardOpaque),
            Self::ParticleSimulation => Some(RenderGraphPassKind::ParticleSimulation),
            Self::HairSimulation => Some(RenderGraphPassKind::HairSimulation),
            Self::ParticleGBuffer => Some(RenderGraphPassKind::ParticleGBuffer),
            Self::ParticleComposite => Some(RenderGraphPassKind::ParticleComposite),
            Self::Transparent => Some(RenderGraphPassKind::Transparent),
            Self::Water => Some(RenderGraphPassKind::Water),
            Self::FroxelFog => Some(RenderGraphPassKind::FroxelFog),
            Self::ScreenSpaceReflections => Some(RenderGraphPassKind::ScreenSpaceReflections),
            Self::PostFx => Some(RenderGraphPassKind::PostFx),
            Self::BloomExtract => Some(RenderGraphPassKind::BloomExtract),
            Self::BloomBlur => Some(RenderGraphPassKind::BloomBlur),
            Self::TaaResolve => Some(RenderGraphPassKind::TaaResolve),
            Self::MsaaResolve => Some(RenderGraphPassKind::MsaaResolve),
            Self::UiBackdropBlur => Some(RenderGraphPassKind::UiBackdropBlur),
            Self::UiComposite => Some(RenderGraphPassKind::UiComposite),
            Self::DebugOverlay => Some(RenderGraphPassKind::DebugOverlay),
        }
    }

    #[inline]
    pub const fn stable_pass_id(self) -> Option<RenderGraphPassId> {
        let id = match self {
            Self::BeginFrame | Self::EndFrame => return None,
            Self::ShadowMap => 100,
            Self::ShadowCascadeMap => 120,
            Self::LocalShadowMap => 140,
            Self::TessellationPrepare => 180,
            Self::DepthPrepass => 200,
            Self::VisibilityCull => 250,
            Self::ViewportGBuffer => 300,
            Self::DeferredLighting => 400,
            Self::ViewportForward => 500,
            Self::ParticleSimulation => 560,
            Self::HairSimulation => 580,
            Self::ParticleGBuffer => 590,
            Self::ParticleComposite => 595,
            Self::Transparent => 600,
            Self::Water => 700,
            Self::FroxelFog => 760,
            Self::ScreenSpaceReflections => 790,
            Self::PostFx => 800,
            Self::BloomExtract => 820,
            Self::BloomBlur => 830,
            Self::TaaResolve => 840,
            Self::MsaaResolve => 850,
            Self::UiBackdropBlur => 880,
            Self::UiComposite => 900,
            Self::DebugOverlay => 1_000,
        };
        Some(RenderGraphPassId(id))
    }

    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BeginFrame => "begin_frame",
            Self::ShadowMap => "shadow_map",
            Self::ShadowCascadeMap => "shadow_cascade_map",
            Self::LocalShadowMap => "local_shadow_map",
            Self::TessellationPrepare => "tessellation_prepare",
            Self::DepthPrepass => "depth_prepass",
            Self::VisibilityCull => "visibility_cull",
            Self::ViewportGBuffer => "viewport_gbuffer",
            Self::DeferredLighting => "deferred_lighting",
            Self::ViewportForward => "viewport_forward",
            Self::ParticleSimulation => "particle_simulation",
            Self::HairSimulation => "hair_simulation",
            Self::ParticleGBuffer => "particle_gbuffer",
            Self::ParticleComposite => "particle_composite",
            Self::Transparent => "transparent",
            Self::Water => "water",
            Self::FroxelFog => "froxel_fog",
            Self::ScreenSpaceReflections => "screen_space_reflections",
            Self::PostFx => "postfx",
            Self::BloomExtract => "bloom_extract",
            Self::BloomBlur => "bloom_blur",
            Self::TaaResolve => "taa_resolve",
            Self::MsaaResolve => "msaa_resolve",
            Self::UiBackdropBlur => "ui_backdrop_blur",
            Self::UiComposite => "ui_composite",
            Self::DebugOverlay => "debug_overlay",
            Self::EndFrame => "end_frame",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPhaseDesc {
    pub phase: StandardRenderPhase,
    pub pass_id: Option<RenderGraphPassId>,
    pub label: String,
}

impl RenderPhaseDesc {
    #[inline]
    pub fn standard(phase: StandardRenderPhase) -> Self {
        Self {
            phase,
            pass_id: phase.stable_pass_id(),
            label: phase.label().to_string(),
        }
    }
}
