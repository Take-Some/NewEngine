use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SsaoParams {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_ssao_radius_ws")]
    pub radius_ws: f32,
    #[serde(default = "default_ssao_intensity")]
    pub intensity: f32,
    #[serde(default = "default_ssao_quality_steps")]
    pub quality_steps: u32,
    #[serde(default = "default_true")]
    pub half_resolution: bool,
}

impl Default for SsaoParams {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: false,
            radius_ws: default_ssao_radius_ws(),
            intensity: default_ssao_intensity(),
            quality_steps: default_ssao_quality_steps(),
            half_resolution: true,
        }
    }
}

/// Screen-space directional contact-shadow controls.
///
/// This is intentionally independent from SSAO. Contact shadows are a short-range
/// directional visibility layer that augments raster shadow maps near receivers;
/// SSAO remains an ambient cavity term.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContactShadowParams {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_contact_shadow_strength")]
    pub strength: f32,
    #[serde(default = "default_contact_shadow_ray_length_px")]
    pub max_ray_length_px: f32,
    #[serde(default = "default_contact_shadow_receiver_bias_scale")]
    pub receiver_bias_scale: f32,
}

impl Default for ContactShadowParams {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: true,
            strength: default_contact_shadow_strength(),
            max_ray_length_px: default_contact_shadow_ray_length_px(),
            receiver_bias_scale: default_contact_shadow_receiver_bias_scale(),
        }
    }
}

#[inline]
fn default_contact_shadow_strength() -> f32 {
    0.25
}

#[inline]
fn default_contact_shadow_ray_length_px() -> f32 {
    22.0
}

#[inline]
fn default_contact_shadow_receiver_bias_scale() -> f32 {
    1.75
}

#[inline]
fn default_ssao_radius_ws() -> f32 {
    0.75
}
#[inline]
fn default_ssao_intensity() -> f32 {
    0.82
}
#[inline]
fn default_ssao_quality_steps() -> u32 {
    16
}

/// Screen-space reflection controls for the raster/deferred reflection tier.
/// RT reflections are a separate capability and must not silently alias this contract.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenSpaceReflectionParams {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_ssr_intensity")]
    pub intensity: f32,
    #[serde(default = "default_ssr_max_distance")]
    pub max_distance_m: f32,
    #[serde(default = "default_ssr_thickness")]
    pub thickness_m: f32,
    #[serde(default = "default_ssr_stride")]
    pub stride_m: f32,
    #[serde(default = "default_ssr_roughness_cutoff")]
    pub roughness_cutoff: f32,
    #[serde(default = "default_ssr_max_steps")]
    pub max_steps: u32,
}

impl Default for ScreenSpaceReflectionParams {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: default_ssr_intensity(),
            max_distance_m: default_ssr_max_distance(),
            thickness_m: default_ssr_thickness(),
            stride_m: default_ssr_stride(),
            roughness_cutoff: default_ssr_roughness_cutoff(),
            max_steps: default_ssr_max_steps(),
        }
    }
}

#[inline]
fn default_ssr_intensity() -> f32 { 0.55 }
#[inline]
fn default_ssr_max_distance() -> f32 { 48.0 }
#[inline]
fn default_ssr_thickness() -> f32 { 0.22 }
#[inline]
fn default_ssr_stride() -> f32 { 0.55 }
#[inline]
fn default_ssr_roughness_cutoff() -> f32 { 0.68 }
#[inline]
fn default_ssr_max_steps() -> u32 { 56 }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostFxQualityParams {
    #[serde(default)]
    pub bloom: BloomParams,
    #[serde(default)]
    pub fxaa: FxaaParams,
    #[serde(default)]
    pub taa: TaaParams,
    #[serde(default)]
    pub ssao: SsaoParams,
    #[serde(default)]
    pub contact_shadows: ContactShadowParams,
    #[serde(default)]
    pub ssr: ScreenSpaceReflectionParams,
    #[serde(default)]
    pub color: ColorGradeParams,
    #[serde(default)]
    pub anti_aliasing: AntiAliasingMode,
}

impl Default for PostFxQualityParams {
    #[inline]
    fn default() -> Self {
        Self {
            bloom: BloomParams::default(),
            fxaa: FxaaParams::default(),
            taa: TaaParams::default(),
            ssao: SsaoParams::default(),
            contact_shadows: ContactShadowParams::default(),
            ssr: ScreenSpaceReflectionParams::default(),
            color: ColorGradeParams::default(),
            anti_aliasing: AntiAliasingMode::Fxaa,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SunPostFxParams {
    /// Sun screen position in normalized viewport coordinates. [0,0] is lower-left.
    #[serde(default = "default_sun_screen_position")]
    pub screen_position: [f32; 2],
    /// Linear RGB sun color from the active gameplay directional light.
    #[serde(default = "default_sun_color")]
    pub color: [f32; 3],
    /// World-space light direction used by lighting/deferred passes. Must be normalized by the provider.
    #[serde(default = "default_sun_direction")]
    pub direction: [f32; 3],
    /// Scalar intensity from the active gameplay directional light.
    #[serde(default)]
    pub intensity: f32,
    /// 0..1 visibility. The runtime computes this from sun direction and the active view projection.
    #[serde(default)]
    pub visibility: f32,
    /// Normalized screen radius of the visible solar disk.
    #[serde(default = "default_sun_disk_radius")]
    pub disk_radius: f32,
    /// Screen-space flare strength. No ray tracing; pure post-process optics.
    #[serde(default = "default_sun_flare_strength")]
    pub flare_strength: f32,
    /// Screen-space god-ray/radial streak strength. No ray tracing.
    #[serde(default = "default_sun_ray_strength")]
    pub ray_strength: f32,
}

impl Default for SunPostFxParams {
    #[inline]
    fn default() -> Self {
        Self {
            screen_position: default_sun_screen_position(),
            color: default_sun_color(),
            direction: default_sun_direction(),
            intensity: 0.0,
            visibility: 0.0,
            disk_radius: default_sun_disk_radius(),
            flare_strength: default_sun_flare_strength(),
            ray_strength: default_sun_ray_strength(),
        }
    }
}

#[inline]
fn default_true() -> bool {
    true
}
#[inline]
fn default_sun_screen_position() -> [f32; 2] {
    [0.5, 0.5]
}
#[inline]
fn default_sun_color() -> [f32; 3] {
    [1.0, 0.94, 0.82]
}
#[inline]
fn default_sun_direction() -> [f32; 3] {
    [-0.53590363, -0.7989835, -0.27282366]
}
#[inline]
fn default_sun_disk_radius() -> f32 {
    0.0045
}
#[inline]
fn default_sun_flare_strength() -> f32 {
    0.18
}
#[inline]
fn default_sun_ray_strength() -> f32 {
    0.16
}
