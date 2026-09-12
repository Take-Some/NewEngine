#![forbid(unsafe_op_in_unsafe_fn)]

/// Generic sky/environment dome rendering intent attached to an ordinary scene entity.
#[derive(Clone, Debug)]
pub struct EnvironmentDomeRenderState {
    pub definition_ref: Option<String>,
    pub asset_ref: Option<String>,
    pub uv_transform: [f32; 4],
    pub material_params: [f32; 4],
    pub emissive_params: [f32; 3],
}

/// World-authored clear-color projection consumed by generic render orchestration.
#[derive(Clone, Copy, Debug)]
pub struct WorldClearColor {
    pub color: [f32; 4],
}

/// Environment-owned atmospheric fog intent consumed by render orchestration.
/// This is deliberately separate from display grading: fog participates in scene
/// radiance before lens/tonemap and will later feed the graph-level froxel path.
#[derive(Clone, Copy, Debug)]
pub struct EnvironmentFogRenderState {
    pub enabled: bool,
    pub density: f32,
    pub height_falloff: f32,
    pub color_linear: [f32; 3],
    pub base_height_m: f32,
    pub start_distance_m: f32,
    pub max_opacity: f32,
}

impl Default for EnvironmentFogRenderState {
    fn default() -> Self {
        Self {
            enabled: false,
            density: 0.0,
            height_falloff: 0.12,
            color_linear: [0.45, 0.50, 0.58],
            base_height_m: 0.0,
            start_distance_m: 2.0,
            max_opacity: 0.94,
        }
    }
}

/// Environment-driven display/post-FX intent. The render backend still owns the
/// concrete implementation, adaptation history and display encoding.
#[derive(Clone, Copy, Debug)]
pub struct EnvironmentPostFxState {
    pub exposure: f32,
    pub gamma: f32,
    pub black_lift: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub temperature: f32,
    pub vignette_strength: f32,
    pub local_contrast_strength: f32,
    pub dither_strength: f32,
    pub bloom_threshold: f32,
    pub bloom_knee: f32,
    pub bloom_intensity: f32,
    pub bloom_radius: f32,
    pub sun_glare_scale: f32,
    pub sun_ray_scale: f32,
}

impl Default for EnvironmentPostFxState {
    fn default() -> Self {
        Self {
            exposure: 1.12,
            gamma: 2.2,
            black_lift: 0.0,
            saturation: 1.04,
            contrast: 1.02,
            temperature: 0.0,
            vignette_strength: 0.055,
            local_contrast_strength: 0.060,
            dither_strength: 1.0,
            bloom_threshold: 1.05,
            bloom_knee: 0.30,
            bloom_intensity: 0.060,
            bloom_radius: 1.0,
            sun_glare_scale: 1.0,
            sun_ray_scale: 1.0,
        }
    }
}

/// Physical cloud/atmosphere profile resolved by engine.world.environment and
/// consumed globally by sky-capable renderers. Kept separate from material and
/// cloud-shadow payloads so tuning does not overload unrelated ABI lanes.
/// profile0 = [low_base_m, low_thickness_m, low_density, high_coverage]
/// profile1 = [humidity, aerosol_density, precipitation, high_density]
#[derive(Clone, Copy, Debug)]
pub struct SkyCloudProfileRenderState {
    pub profile0: [f32; 4],
    pub profile1: [f32; 4],
}

impl Default for SkyCloudProfileRenderState {
    fn default() -> Self {
        Self {
            profile0: [1250.0, 1100.0, 0.16, 0.08],
            profile1: [0.45, 0.12, 0.0, 0.04],
        }
    }
}

/// Generic packed cloud-shadow projection consumed by light/render providers.
#[derive(Clone, Copy, Debug)]
pub struct CloudShadowRenderState {
    pub map0: [f32; 4],
    pub map1: [f32; 4],
    pub map2: [f32; 4],
    pub map3: [f32; 4],
    pub map4: [f32; 4],
    pub broad_ambient_scale: f32,
}

impl Default for CloudShadowRenderState {
    fn default() -> Self {
        Self {
            map0: [0.0, 0.0, 0.0, 0.5],
            map1: [0.0042, 1800.0, 0.0, 0.70],
            map2: [0.0, 0.0, 1.0, 0.0],
            map3: [0.0, 0.0, 0.0, 0.5],
            map4: [0.0, 0.032, 0.14, 96.0],
            broad_ambient_scale: 1.0,
        }
    }
}

/// Generic three-channel terrain material-layer intent.
#[derive(Clone, Debug)]
pub struct TerrainMaterialLayers {
    pub forest_base_texture: String,
    pub sand_base_texture: String,
    pub rock_base_texture: String,
    pub patch_scale: f32,
    pub blend_softness: f32,
}
