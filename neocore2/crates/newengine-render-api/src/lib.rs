#![forbid(unsafe_op_in_unsafe_fn)]

pub use newengine_ui_draw::{
    TextureRef, UiBorderPaintCommand, UiClipPaintCommand, UiDrawCmd, UiDrawList,
    UiIconPaintCommand, UiImagePaintCommand, UiImageRef, UiLayerDomain, UiLayerDrawPacket,
    UiLayerDrawPacketSet, UiLayerPaintCommand, UiMaterialParamValue, UiMaterialQuadPaintCommand,
    UiMesh, UiPaintCommand, UiPaintList, UiPaintNodeRef, UiPaintPhase, UiRect, UiRectPaintCommand,
    UiRoundedRectPaintCommand, UiScopePaintCommand, UiSurfaceEffectPaintCommand, UiTexId,
    UiTextPaintCommand, UiTexture, UiTextureDelta, UiTextureMode, UiTexturePatch,
    UiVectorPaintCommand, UiVertex, VectorRef,
};

pub mod reserved_textures {
    pub use newengine_ui_draw::reserved::*;
}

mod bindings;
mod capabilities;
mod constants;
mod diagnostics;
mod effects;
mod events;
mod feature_registry;
mod frame;
mod gpu_visibility;
mod hair;
mod ids;
mod material_graph;
mod maturity;
mod pipeline;
mod postfx;
mod protocol;
mod provider_bridge;
mod render_graph;
mod residency;
mod resources;
mod shader_cache;
mod shader_variants;
mod shadows;
mod uploads;

pub use bindings::*;
pub use capabilities::*;
pub use constants::*;
pub use diagnostics::*;
pub use effects::*;
pub use events::*;
pub use feature_registry::*;
pub use frame::*;
pub use gpu_visibility::*;
pub use hair::*;
pub use ids::*;
pub use material_graph::*;
pub use maturity::*;
pub use pipeline::*;
pub use postfx::*;
pub use protocol::*;
pub use provider_bridge::*;
pub use render_graph::*;
pub use residency::*;
pub use resources::*;
pub use shader_cache::*;
pub use shader_variants::*;
pub use shadows::*;
pub use uploads::*;
