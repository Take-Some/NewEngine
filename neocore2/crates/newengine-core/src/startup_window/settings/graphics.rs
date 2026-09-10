#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupGraphicsSettings {
    pub preset: GraphicsPreset,
    pub msaa_samples: u8,
    pub fxaa_enabled: bool,
    pub fxaa_edge_threshold: f32,
    pub fxaa_edge_threshold_min: f32,
    pub fxaa_subpixel_quality: f32,
    pub taa_enabled: bool,
    pub taa_feedback: f32,
    pub taa_neighborhood_clamping: f32,
    pub taa_jitter_scale: f32,
    pub ssao_enabled: bool,
    pub ssao_radius_ws: f32,
    pub ssao_intensity: f32,
    pub ssao_quality_steps: u32,
    pub ssao_half_resolution: bool,
    pub bloom_enabled: bool,
    pub bloom_threshold: f32,
    pub bloom_knee: f32,
    pub bloom_intensity: f32,
    pub bloom_radius: f32,
    pub depth_of_field_enabled: bool,
    pub motion_blur_enabled: bool,
    pub sun_rays_enabled: bool,
    pub shadows_enabled: bool,
    pub shadow_quality: ShadowQuality,
    /// 0 keeps the scene-authored cascade count; 1..=4 overrides it for this launch.
    pub shadow_cascade_count: u32,
    /// 0 keeps the scene-authored map size; otherwise one of the supported 256..=16284 launch overrides.
    pub shadow_map_resolution: u32,
    pub shadow_advanced_override: bool,
    pub shadow_filter: ShadowFilterMode,
    pub shadow_max_distance: f32,
    pub shadow_softness: f32,
    pub shadow_bias: f32,
    pub shadow_normal_bias: f32,
    pub shadow_contact_strength: f32,
    pub shadow_pcss_light_radius_degrees: f32,
    pub shadow_pcss_blocker_radius_texels: f32,
    pub shadow_pcss_max_filter_radius_texels: f32,
    pub shadow_pcss_blocker_samples: u32,
    pub shadow_pcss_filter_samples: u32,
    pub shadow_pcss_min_filter_radius_texels: f32,
    pub shadow_pcss_stable_kernel_texels: f32,
    /// Effective world visibility radius in meters. Authored-world streaming converts this
    /// to a map-cell render radius while camera projection keeps a farther safety clip.
    pub view_distance_meters: f32,
    pub lod_quality: LodQuality,
    /// Global distance multiplier used by runtime visibility/LOD policy. 1.0 preserves authored/default distances.
    pub lod_distance_scale: f32,
    pub texture_quality: TextureQuality,
    pub anisotropy: u8,
}

impl Default for StartupGraphicsSettings {
    fn default() -> Self {
        let value = Self {
            preset: GraphicsPreset::Balanced,
            msaa_samples: 0,
            fxaa_enabled: true,
            fxaa_edge_threshold: 0.125,
            fxaa_edge_threshold_min: 0.0312,
            fxaa_subpixel_quality: 0.75,
            taa_enabled: false,
            taa_feedback: 0.92,
            taa_neighborhood_clamping: 1.0,
            taa_jitter_scale: 1.0,
            ssao_enabled: false,
            ssao_radius_ws: 0.75,
            ssao_intensity: 0.82,
            ssao_quality_steps: 16,
            ssao_half_resolution: true,
            bloom_enabled: true,
            bloom_threshold: 0.85,
            bloom_knee: 0.35,
            bloom_intensity: 0.085,
            bloom_radius: 1.0,
            depth_of_field_enabled: false,
            motion_blur_enabled: false,
            sun_rays_enabled: true,
            shadows_enabled: true,
            shadow_quality: ShadowQuality::Balanced,
            shadow_cascade_count: 0,
            shadow_map_resolution: 0,
            shadow_advanced_override: false,
            shadow_filter: ShadowFilterMode::Pcss,
            shadow_max_distance: 80.0,
            shadow_softness: 1.0,
            shadow_bias: 0.0025,
            shadow_normal_bias: 0.015,
            shadow_contact_strength: 0.25,
            shadow_pcss_light_radius_degrees: 0.266,
            shadow_pcss_blocker_radius_texels: 3.0,
            shadow_pcss_max_filter_radius_texels: 5.0,
            shadow_pcss_blocker_samples: 10,
            shadow_pcss_filter_samples: 12,
            shadow_pcss_min_filter_radius_texels: 0.18,
            shadow_pcss_stable_kernel_texels: 8.0,
            view_distance_meters: 1000.0,
            lod_quality: LodQuality::High,
            lod_distance_scale: 1.0,
            texture_quality: TextureQuality::High,
            anisotropy: 8,
        };
        // Default launch settings preserve scene-authored cascade/map topology. Quality
        // presets become authoritative only when the user explicitly selects one.
        value
    }
}

impl StartupGraphicsSettings {
    pub fn apply_preset(&mut self, preset: GraphicsPreset) {
        self.preset = preset;
        match preset {
            GraphicsPreset::Low => {
                self.shadow_advanced_override = false;
                self.msaa_samples = 0;
                self.fxaa_enabled = true;
                self.taa_enabled = false;
                self.ssao_enabled = false;
                self.ssao_quality_steps = 8;
                self.ssao_half_resolution = true;
                self.bloom_enabled = false;
                self.depth_of_field_enabled = false;
                self.motion_blur_enabled = false;
                self.sun_rays_enabled = false;
                self.shadows_enabled = true;
                self.shadow_quality = ShadowQuality::Performance;
                self.shadow_cascade_count = 2;
                self.shadow_map_resolution = 512;
                self.view_distance_meters = 500.0;
                self.lod_distance_scale = 0.65;
                self.texture_quality = TextureQuality::Low;
                self.anisotropy = 2;
                self.shadow_filter = ShadowFilterMode::Pcf;
                self.shadow_max_distance = 48.0;
                self.shadow_softness = 0.7;
                self.shadow_bias = 0.0025;
                self.shadow_normal_bias = 0.015;
                self.shadow_contact_strength = 0.10;
                self.shadow_pcss_light_radius_degrees = 0.266;
                self.shadow_pcss_blocker_radius_texels = 2.0;
                self.shadow_pcss_max_filter_radius_texels = 3.0;
                self.shadow_pcss_blocker_samples = 6;
                self.shadow_pcss_filter_samples = 8;
                self.shadow_pcss_min_filter_radius_texels = 0.18;
                self.shadow_pcss_stable_kernel_texels = 8.0;
                self.lod_quality = LodQuality::Low;
            }
            GraphicsPreset::Balanced => {
                self.shadow_advanced_override = false;
                self.msaa_samples = 0;
                self.fxaa_enabled = true;
                self.taa_enabled = false;
                self.ssao_enabled = false;
                self.ssao_quality_steps = 16;
                self.ssao_half_resolution = true;
                self.bloom_enabled = true;
                self.depth_of_field_enabled = false;
                self.motion_blur_enabled = false;
                self.sun_rays_enabled = true;
                self.shadows_enabled = true;
                self.shadow_quality = ShadowQuality::Balanced;
                self.shadow_cascade_count = 3;
                self.shadow_map_resolution = 1024;
                self.view_distance_meters = 1000.0;
                self.lod_distance_scale = 0.85;
                self.texture_quality = TextureQuality::High;
                self.anisotropy = 8;
                self.shadow_filter = ShadowFilterMode::Pcss;
                self.shadow_max_distance = 80.0;
                self.shadow_softness = 1.0;
                self.shadow_bias = 0.0025;
                self.shadow_normal_bias = 0.015;
                self.shadow_contact_strength = 0.25;
                self.shadow_pcss_light_radius_degrees = 0.266;
                self.shadow_pcss_blocker_radius_texels = 3.0;
                self.shadow_pcss_max_filter_radius_texels = 5.0;
                self.shadow_pcss_blocker_samples = 8;
                self.shadow_pcss_filter_samples = 12;
                self.shadow_pcss_min_filter_radius_texels = 0.18;
                self.shadow_pcss_stable_kernel_texels = 8.0;
                self.lod_quality = LodQuality::Medium;
            }
            GraphicsPreset::High => {
                self.shadow_advanced_override = false;
                self.msaa_samples = 2;
                self.fxaa_enabled = true;
                self.taa_enabled = false;
                self.ssao_enabled = true;
                self.ssao_quality_steps = 24;
                self.ssao_half_resolution = true;
                self.bloom_enabled = true;
                self.depth_of_field_enabled = false;
                self.motion_blur_enabled = false;
                self.sun_rays_enabled = true;
                self.shadows_enabled = true;
                self.shadow_quality = ShadowQuality::Quality;
                self.shadow_cascade_count = 4;
                self.shadow_map_resolution = 2048;
                self.view_distance_meters = 1500.0;
                self.lod_distance_scale = 1.0;
                self.texture_quality = TextureQuality::High;
                self.anisotropy = 8;
                self.shadow_filter = ShadowFilterMode::Pcss;
                self.shadow_max_distance = 140.0;
                self.shadow_softness = 1.0;
                self.shadow_bias = 0.0025;
                self.shadow_normal_bias = 0.015;
                self.shadow_contact_strength = 0.25;
                self.shadow_pcss_light_radius_degrees = 0.266;
                self.shadow_pcss_blocker_radius_texels = 3.0;
                self.shadow_pcss_max_filter_radius_texels = 5.0;
                self.shadow_pcss_blocker_samples = 12;
                self.shadow_pcss_filter_samples = 16;
                self.shadow_pcss_min_filter_radius_texels = 0.18;
                self.shadow_pcss_stable_kernel_texels = 8.0;
                self.lod_quality = LodQuality::High;
            }
            GraphicsPreset::Ultra => {
                self.shadow_advanced_override = false;
                self.msaa_samples = 4;
                self.fxaa_enabled = true;
                self.taa_enabled = true;
                self.ssao_enabled = true;
                self.ssao_quality_steps = 32;
                self.ssao_half_resolution = false;
                self.bloom_enabled = true;
                self.depth_of_field_enabled = true;
                self.motion_blur_enabled = true;
                self.sun_rays_enabled = true;
                self.shadows_enabled = true;
                self.shadow_quality = ShadowQuality::Cinematic;
                self.shadow_cascade_count = 4;
                self.shadow_map_resolution = 4096;
                self.view_distance_meters = 2500.0;
                self.lod_distance_scale = 1.35;
                self.texture_quality = TextureQuality::Ultra;
                self.anisotropy = 16;
                self.shadow_filter = ShadowFilterMode::Pcss;
                self.shadow_max_distance = 240.0;
                self.shadow_softness = 1.0;
                self.shadow_bias = 0.0025;
                self.shadow_normal_bias = 0.015;
                self.shadow_contact_strength = 0.25;
                self.shadow_pcss_light_radius_degrees = 0.266;
                self.shadow_pcss_blocker_radius_texels = 3.0;
                self.shadow_pcss_max_filter_radius_texels = 8.0;
                self.shadow_pcss_blocker_samples = 16;
                self.shadow_pcss_filter_samples = 16;
                self.shadow_pcss_min_filter_radius_texels = 0.18;
                self.shadow_pcss_stable_kernel_texels = 8.0;
                self.lod_quality = LodQuality::Ultra;
            }
            GraphicsPreset::Custom => {}
        }
    }

    pub fn normalize(&mut self) {
        self.msaa_samples = match self.msaa_samples {
            2 | 4 | 8 => self.msaa_samples,
            _ => 0,
        };
        self.anisotropy = match self.anisotropy {
            2 | 4 | 8 | 16 => self.anisotropy,
            _ => 0,
        };
        self.fxaa_edge_threshold = self.fxaa_edge_threshold.clamp(0.01, 1.0);
        self.fxaa_edge_threshold_min = self.fxaa_edge_threshold_min.clamp(0.001, 1.0);
        self.fxaa_subpixel_quality = self.fxaa_subpixel_quality.clamp(0.0, 1.0);
        self.taa_feedback = self.taa_feedback.clamp(0.0, 0.99);
        self.taa_neighborhood_clamping = self.taa_neighborhood_clamping.clamp(0.0, 4.0);
        self.taa_jitter_scale = self.taa_jitter_scale.clamp(0.0, 2.0);
        self.ssao_radius_ws = self.ssao_radius_ws.clamp(0.05, 10.0);
        self.ssao_intensity = self.ssao_intensity.clamp(0.0, 4.0);
        self.ssao_quality_steps = self.ssao_quality_steps.clamp(4, 64);
        self.bloom_threshold = self.bloom_threshold.clamp(0.0, 20.0);
        self.bloom_knee = self.bloom_knee.clamp(0.0, 5.0);
        self.bloom_intensity = self.bloom_intensity.clamp(0.0, 5.0);
        self.bloom_radius = self.bloom_radius.clamp(0.1, 5.0);
        self.view_distance_meters = self.view_distance_meters.clamp(100.0, 10_000.0);
        self.lod_distance_scale = self.lod_distance_scale.clamp(0.5, 2.0);
        self.shadow_max_distance = self.shadow_max_distance.clamp(4.0, 2048.0);
        self.shadow_softness = self.shadow_softness.clamp(0.0, 8.0);
        self.shadow_bias = self.shadow_bias.clamp(0.0, 0.1);
        self.shadow_normal_bias = self.shadow_normal_bias.clamp(0.0, 0.5);
        self.shadow_contact_strength = self.shadow_contact_strength.clamp(0.0, 1.0);
        self.shadow_pcss_light_radius_degrees =
            self.shadow_pcss_light_radius_degrees.clamp(0.001, 5.0);
        self.shadow_pcss_blocker_radius_texels =
            self.shadow_pcss_blocker_radius_texels.clamp(0.5, 32.0);
        self.shadow_pcss_max_filter_radius_texels =
            self.shadow_pcss_max_filter_radius_texels.clamp(0.5, 64.0);
        self.shadow_pcss_blocker_samples = self.shadow_pcss_blocker_samples.clamp(4, 16);
        self.shadow_pcss_filter_samples = self.shadow_pcss_filter_samples.clamp(4, 16);
        self.shadow_pcss_min_filter_radius_texels = self
            .shadow_pcss_min_filter_radius_texels
            .clamp(0.0, self.shadow_pcss_max_filter_radius_texels);
        self.shadow_pcss_stable_kernel_texels =
            self.shadow_pcss_stable_kernel_texels.clamp(1.0, 32.0);
        self.shadow_cascade_count = match self.shadow_cascade_count {
            0 => 0,
            value => value.clamp(1, 4),
        };
        self.shadow_map_resolution = normalize_shadow_map_resolution(self.shadow_map_resolution);
        if !self.shadows_enabled {
            self.shadow_quality = ShadowQuality::Off;
        } else if matches!(self.shadow_quality, ShadowQuality::Off) {
            self.shadow_quality = ShadowQuality::Balanced;
        }
    }

    pub fn apply_lod_quality(&mut self, quality: LodQuality) {
        self.lod_quality = quality;
        if let Some(scale) = quality.distance_scale() {
            self.lod_distance_scale = scale;
        }
        self.mark_custom();
    }

    #[inline]
    pub fn mark_custom(&mut self) {
        self.preset = GraphicsPreset::Custom;
    }
}
