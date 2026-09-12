use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewDepthOfFieldFrameParams {
    #[serde(default)]
    pub near_start: f32,
    #[serde(default)]
    pub near_end: f32,
    #[serde(default = "default_dof_far_plane")]
    pub far_start: f32,
    #[serde(default = "default_dof_far_plane")]
    pub far_end: f32,
    #[serde(default)]
    pub blend_level: f32,
    #[serde(default)]
    pub high_quality: bool,
}

impl Default for ViewDepthOfFieldFrameParams {
    #[inline]
    fn default() -> Self {
        Self {
            near_start: 0.0,
            near_end: 0.0,
            far_start: default_dof_far_plane(),
            far_end: default_dof_far_plane(),
            blend_level: 0.0,
            high_quality: false,
        }
    }
}

#[inline]
pub(super) fn default_dof_far_plane() -> f32 {
    10_000.0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewMotionBlurFrameParams {
    #[serde(default)]
    pub strength: f32,
    #[serde(default = "default_motion_blur_decay_rate")]
    pub decay_rate: f32,
}

impl Default for ViewMotionBlurFrameParams {
    #[inline]
    fn default() -> Self {
        Self {
            strength: 0.0,
            decay_rate: default_motion_blur_decay_rate(),
        }
    }
}

#[inline]
fn default_motion_blur_decay_rate() -> f32 {
    0.5
}

/// Renderer-facing, source-agnostic frame post-process intent.
///
/// This is deliberately not tied to any view producer implementation. Cutscene, replay,
/// editor, gameplay or photo-mode systems can provide the same normalized
/// frame intent without coupling render API to producer-specific state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewPostFxFrameParams {
    #[serde(default)]
    pub dof: ViewDepthOfFieldFrameParams,
    #[serde(default)]
    pub motion_blur: ViewMotionBlurFrameParams,
    #[serde(default)]
    pub shake_amplitude: f32,
    #[serde(default)]
    pub exposure_bias: f32,
    #[serde(default)]
    pub jitter_px: [f32; 2],
}

impl Default for ViewPostFxFrameParams {
    #[inline]
    fn default() -> Self {
        Self {
            dof: ViewDepthOfFieldFrameParams::default(),
            motion_blur: ViewMotionBlurFrameParams::default(),
            shake_amplitude: 0.0,
            exposure_bias: 0.0,
            jitter_px: [0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UiBackdropPostFxParams {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub alpha: f32,
    #[serde(default)]
    pub dim_opacity: f32,
    #[serde(default)]
    pub blur_radius_px: f32,
}

impl Default for UiBackdropPostFxParams {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: false,
            alpha: 0.0,
            dim_opacity: 0.0,
            blur_radius_px: 0.0,
        }
    }
}

/// Analytic atmospheric medium used by the raster fallback until the graph-level
/// `FroxelFog` volume is available on every provider. Values are world-space and
/// source agnostic; backends decide how to integrate the medium.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AtmosphericFogFrameParams {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub density: f32,
    #[serde(default = "default_fog_height_falloff")]
    pub height_falloff: f32,
    #[serde(default = "default_fog_color")]
    pub color_linear: [f32; 3],
    #[serde(default)]
    pub base_height_m: f32,
    #[serde(default = "default_fog_start_distance")]
    pub start_distance_m: f32,
    #[serde(default = "default_fog_max_opacity")]
    pub max_opacity: f32,
}

impl Default for AtmosphericFogFrameParams {
    fn default() -> Self {
        Self {
            enabled: false,
            density: 0.0,
            height_falloff: default_fog_height_falloff(),
            color_linear: default_fog_color(),
            base_height_m: 0.0,
            start_distance_m: default_fog_start_distance(),
            max_opacity: default_fog_max_opacity(),
        }
    }
}

#[inline]
fn default_fog_height_falloff() -> f32 { 0.12 }
#[inline]
fn default_fog_color() -> [f32; 3] { [0.45, 0.50, 0.58] }
#[inline]
fn default_fog_start_distance() -> f32 { 2.0 }
#[inline]
fn default_fog_max_opacity() -> f32 { 0.94 }

pub const MAX_FROXEL_POINT_LIGHTS: usize = 4;
pub const MAX_FROXEL_SPOT_LIGHTS: usize = 4;
pub const MAX_FROXEL_CSM_CASCADES: usize = 4;

/// Bounded authoritative lighting payload consumed by volumetric fog. This is a
/// renderer-facing DTO only: no Vulkan handles or provider-native objects cross
/// the frame-envelope boundary. Arrays intentionally mirror the compact world
/// lighting cap used by `PackedLights`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FroxelLightingFrameParams {
    #[serde(default)]
    pub directional_dir_intensity: [f32; 4],
    #[serde(default)]
    pub directional_color: [f32; 4],
    #[serde(default)]
    pub point_pos_range: [[f32; 4]; MAX_FROXEL_POINT_LIGHTS],
    #[serde(default)]
    pub point_color_intensity: [[f32; 4]; MAX_FROXEL_POINT_LIGHTS],
    #[serde(default)]
    pub point_count: u32,
    #[serde(default)]
    pub spot_pos_range: [[f32; 4]; MAX_FROXEL_SPOT_LIGHTS],
    #[serde(default)]
    pub spot_dir_outer_cos: [[f32; 4]; MAX_FROXEL_SPOT_LIGHTS],
    #[serde(default)]
    pub spot_color_intensity: [[f32; 4]; MAX_FROXEL_SPOT_LIGHTS],
    #[serde(default)]
    pub spot_inner_cos: [f32; MAX_FROXEL_SPOT_LIGHTS],
    #[serde(default)]
    pub spot_count: u32,
    #[serde(default)]
    pub csm_enabled: bool,
    #[serde(default = "default_froxel_csm_cascade_count")]
    pub csm_cascade_count: u32,
    #[serde(default = "default_froxel_csm_matrices")]
    pub csm_light_mvp: [[f32; 16]; MAX_FROXEL_CSM_CASCADES],
    #[serde(default)]
    pub csm_splits: [f32; MAX_FROXEL_CSM_CASCADES],
    #[serde(default)]
    pub csm_shadow_params: [f32; 4],
    #[serde(default)]
    pub csm_shadow_extra: [f32; 4],
}

impl Default for FroxelLightingFrameParams {
    fn default() -> Self {
        Self {
            directional_dir_intensity: [0.0, -1.0, 0.0, 0.0],
            directional_color: [1.0, 1.0, 1.0, 0.0],
            point_pos_range: [[0.0; 4]; MAX_FROXEL_POINT_LIGHTS],
            point_color_intensity: [[0.0; 4]; MAX_FROXEL_POINT_LIGHTS],
            point_count: 0,
            spot_pos_range: [[0.0; 4]; MAX_FROXEL_SPOT_LIGHTS],
            spot_dir_outer_cos: [[0.0; 4]; MAX_FROXEL_SPOT_LIGHTS],
            spot_color_intensity: [[0.0; 4]; MAX_FROXEL_SPOT_LIGHTS],
            spot_inner_cos: [0.0; MAX_FROXEL_SPOT_LIGHTS],
            spot_count: 0,
            csm_enabled: false,
            csm_cascade_count: default_froxel_csm_cascade_count(),
            csm_light_mvp: default_froxel_csm_matrices(),
            csm_splits: [0.0; MAX_FROXEL_CSM_CASCADES],
            csm_shadow_params: [0.0; 4],
            csm_shadow_extra: [0.0; 4],
        }
    }
}

#[inline]
fn default_froxel_csm_cascade_count() -> u32 { 1 }

#[inline]
fn default_froxel_csm_matrices() -> [[f32; 16]; MAX_FROXEL_CSM_CASCADES] {
    let identity = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    [identity; MAX_FROXEL_CSM_CASCADES]
}

/// Provider-neutral volumetric fog frame contract. The logical data model is a
/// 3D froxel volume; backends may store it as a native 3D image or a packed 2D atlas.
/// Analytic `AtmosphericFogFrameParams` remains the mandatory bounded fallback.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FroxelFogFrameParams {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_froxel_tile_size_px")]
    pub tile_size_px: u32,
    #[serde(default = "default_froxel_depth_slices")]
    pub depth_slices: u32,
    #[serde(default = "default_froxel_max_distance_m")]
    pub max_distance_m: f32,
    #[serde(default = "default_froxel_temporal_feedback")]
    pub temporal_feedback: f32,
    #[serde(default = "default_froxel_anisotropy")]
    pub anisotropy: f32,
    #[serde(default)]
    pub lighting: FroxelLightingFrameParams,
}

impl Default for FroxelFogFrameParams {
    fn default() -> Self {
        Self {
            enabled: false,
            tile_size_px: default_froxel_tile_size_px(),
            depth_slices: default_froxel_depth_slices(),
            max_distance_m: default_froxel_max_distance_m(),
            temporal_feedback: default_froxel_temporal_feedback(),
            anisotropy: default_froxel_anisotropy(),
            lighting: FroxelLightingFrameParams::default(),
        }
    }
}

#[inline]
fn default_froxel_tile_size_px() -> u32 { 16 }
#[inline]
fn default_froxel_depth_slices() -> u32 { 64 }
#[inline]
fn default_froxel_max_distance_m() -> f32 { 180.0 }
#[inline]
fn default_froxel_temporal_feedback() -> f32 { 0.88 }
#[inline]
fn default_froxel_anisotropy() -> f32 { 0.20 }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostFxFrameParams {
    #[serde(default)]
    pub display: ToneMapDisplayParams,
    #[serde(default)]
    pub sun: SunPostFxParams,
    #[serde(default)]
    pub quality: PostFxQualityParams,
    #[serde(default)]
    pub fog: AtmosphericFogFrameParams,
    #[serde(default)]
    pub froxel_fog: FroxelFogFrameParams,
    #[serde(default)]
    pub view: ViewPostFxFrameParams,
    #[serde(default)]
    pub ui_backdrop: UiBackdropPostFxParams,
}

impl Default for PostFxFrameParams {
    #[inline]
    fn default() -> Self {
        Self {
            display: ToneMapDisplayParams::default(),
            sun: SunPostFxParams::default(),
            quality: PostFxQualityParams::default(),
            fog: AtmosphericFogFrameParams::default(),
            froxel_fog: FroxelFogFrameParams::default(),
            view: ViewPostFxFrameParams::default(),
            ui_backdrop: UiBackdropPostFxParams::default(),
        }
    }
}
