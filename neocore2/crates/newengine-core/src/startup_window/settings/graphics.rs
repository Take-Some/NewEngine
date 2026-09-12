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
    pub ssr_enabled: bool,
    pub ssr_intensity: f32,
    pub ssr_max_distance_m: f32,
    pub ssr_thickness_m: f32,
    pub ssr_stride_m: f32,
    pub ssr_roughness_cutoff: f32,
    pub ssr_max_steps: u32,
    pub volumetric_fog_enabled: bool,
    pub froxel_tile_size_px: u32,
    pub froxel_depth_slices: u32,
    pub froxel_max_distance_m: f32,
    pub froxel_temporal_feedback: f32,
    pub froxel_anisotropy: f32,
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
    /// Builds stable geometry/material/object tables for the experimental GPU-driven data plane.
    pub gpu_scene_tables_enabled: bool,
    /// Enables VisibilityCull + compacted indirect submission only when backend capability negotiation passes.
    pub gpu_driven_indirect_enabled: bool,
    /// Enables the conservative opaque shadow indirect subset; requires the parent GPU-driven path.
    pub gpu_driven_shadow_indirect_enabled: bool,
    pub geometry_arena_vertex_page_mib: u32,
    pub geometry_arena_index_page_mib: u32,
    pub geometry_arena_max_pages: u32,
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
            ssr_enabled: true,
            ssr_intensity: 0.55,
            ssr_max_distance_m: 48.0,
            ssr_thickness_m: 0.22,
            ssr_stride_m: 0.55,
            ssr_roughness_cutoff: 0.68,
            ssr_max_steps: 56,
            volumetric_fog_enabled: true,
            froxel_tile_size_px: 24,
            froxel_depth_slices: 48,
            froxel_max_distance_m: 140.0,
            froxel_temporal_feedback: 0.82,
            froxel_anisotropy: 0.15,
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
            gpu_scene_tables_enabled: false,
            gpu_driven_indirect_enabled: false,
            gpu_driven_shadow_indirect_enabled: false,
            geometry_arena_vertex_page_mib: 32,
            geometry_arena_index_page_mib: 16,
            geometry_arena_max_pages: 64,
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
                self.ssr_enabled = false;
                self.ssr_intensity = 0.40;
                self.ssr_max_distance_m = 24.0;
                self.ssr_thickness_m = 0.30;
                self.ssr_stride_m = 0.90;
                self.ssr_roughness_cutoff = 0.50;
                self.ssr_max_steps = 24;
                self.volumetric_fog_enabled = false;
                self.froxel_tile_size_px = 32;
                self.froxel_depth_slices = 32;
                self.froxel_max_distance_m = 90.0;
                self.froxel_temporal_feedback = 0.72;
                self.froxel_anisotropy = 0.05;
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
                self.ssr_enabled = true;
                self.ssr_intensity = 0.52;
                self.ssr_max_distance_m = 40.0;
                self.ssr_thickness_m = 0.25;
                self.ssr_stride_m = 0.65;
                self.ssr_roughness_cutoff = 0.62;
                self.ssr_max_steps = 48;
                self.volumetric_fog_enabled = true;
                self.froxel_tile_size_px = 24;
                self.froxel_depth_slices = 48;
                self.froxel_max_distance_m = 140.0;
                self.froxel_temporal_feedback = 0.82;
                self.froxel_anisotropy = 0.15;
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
                self.ssr_enabled = true;
                self.ssr_intensity = 0.58;
                self.ssr_max_distance_m = 56.0;
                self.ssr_thickness_m = 0.20;
                self.ssr_stride_m = 0.48;
                self.ssr_roughness_cutoff = 0.72;
                self.ssr_max_steps = 72;
                self.volumetric_fog_enabled = true;
                self.froxel_tile_size_px = 16;
                self.froxel_depth_slices = 64;
                self.froxel_max_distance_m = 200.0;
                self.froxel_temporal_feedback = 0.88;
                self.froxel_anisotropy = 0.20;
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
                self.ssr_enabled = true;
                self.ssr_intensity = 0.62;
                self.ssr_max_distance_m = 72.0;
                self.ssr_thickness_m = 0.16;
                self.ssr_stride_m = 0.36;
                self.ssr_roughness_cutoff = 0.80;
                self.ssr_max_steps = 96;
                self.volumetric_fog_enabled = true;
                self.froxel_tile_size_px = 16;
                self.froxel_depth_slices = 96;
                self.froxel_max_distance_m = 260.0;
                self.froxel_temporal_feedback = 0.92;
                self.froxel_anisotropy = 0.30;
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
        if !self.gpu_scene_tables_enabled {
            self.gpu_driven_indirect_enabled = false;
        }
        if !self.gpu_scene_tables_enabled || !self.gpu_driven_indirect_enabled {
            self.gpu_driven_shadow_indirect_enabled = false;
        }
        self.geometry_arena_vertex_page_mib = self.geometry_arena_vertex_page_mib.clamp(4, 256);
        self.geometry_arena_index_page_mib = self.geometry_arena_index_page_mib.clamp(2, 128);
        self.geometry_arena_max_pages = self.geometry_arena_max_pages.clamp(1, 256);
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
        self.ssr_intensity = self.ssr_intensity.clamp(0.0, 2.0);
        self.ssr_max_distance_m = self.ssr_max_distance_m.clamp(2.0, 200.0);
        self.ssr_thickness_m = self.ssr_thickness_m.clamp(0.02, 2.0);
        self.ssr_stride_m = self.ssr_stride_m.clamp(0.05, 4.0);
        self.ssr_roughness_cutoff = self.ssr_roughness_cutoff.clamp(0.05, 1.0);
        self.ssr_max_steps = self.ssr_max_steps.clamp(8, 128);
        self.froxel_tile_size_px = self.froxel_tile_size_px.clamp(8, 64);
        self.froxel_depth_slices = self.froxel_depth_slices.clamp(16, 128);
        self.froxel_max_distance_m = self.froxel_max_distance_m.clamp(16.0, 1_000.0);
        self.froxel_temporal_feedback = self.froxel_temporal_feedback.clamp(0.0, 0.98);
        self.froxel_anisotropy = self.froxel_anisotropy.clamp(-0.85, 0.85);
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
