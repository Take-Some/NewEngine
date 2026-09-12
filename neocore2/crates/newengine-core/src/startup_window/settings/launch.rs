#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupLaunchSettings {
    pub schema_version: u32,
    pub display: StartupDisplaySettings,
    pub graphics: StartupGraphicsSettings,
}

impl Default for StartupLaunchSettings {
    fn default() -> Self {
        Self {
            schema_version: STARTUP_SETTINGS_SCHEMA_VERSION,
            display: StartupDisplaySettings::default(),
            graphics: StartupGraphicsSettings::default(),
        }
    }
}

impl StartupLaunchSettings {
    pub fn normalize(&mut self) {
        self.schema_version = STARTUP_SETTINGS_SCHEMA_VERSION;
        self.display.monitor_index = self.display.monitor_index.max(-1);
        self.display.resolution = match self.display.resolution {
            [0, 0] => [0, 0],
            [width, height] if width == 0 || height == 0 => [0, 0],
            [width, height] => [width.clamp(64, 16_384), height.clamp(64, 16_384)],
        };
        self.display.render_scale = self.display.render_scale.clamp(0.25, 2.0);
        self.display.refresh_rate_millihz = self.display.refresh_rate_millihz.min(1_000_000);
        self.display.frame_limit = match self.display.frame_limit {
            0 => 0,
            value => value.clamp(15, 1_000),
        };
        if !matches!(self.display.window_mode, StartupWindowMode::Windowed) {
            self.display.center_window = false;
        }
        self.graphics.normalize();
    }

    pub fn publish_environment_snapshot(&self) {
        let mut value = self.clone();
        value.normalize();
        set_env(ENV_GRAPHICS_PRESET, value.graphics.preset.as_str());
        set_env(ENV_RENDER_SCALE, value.display.render_scale.to_string());
        set_env(ENV_MSAA_SAMPLES, value.graphics.msaa_samples.to_string());
        set_env(ENV_FXAA_ENABLED, bool_text(value.graphics.fxaa_enabled));
        set_env(
            ENV_FXAA_EDGE_THRESHOLD,
            value.graphics.fxaa_edge_threshold.to_string(),
        );
        set_env(
            ENV_FXAA_EDGE_THRESHOLD_MIN,
            value.graphics.fxaa_edge_threshold_min.to_string(),
        );
        set_env(
            ENV_FXAA_SUBPIXEL_QUALITY,
            value.graphics.fxaa_subpixel_quality.to_string(),
        );
        set_env(ENV_TAA_ENABLED, bool_text(value.graphics.taa_enabled));
        set_env(ENV_TAA_FEEDBACK, value.graphics.taa_feedback.to_string());
        set_env(
            ENV_TAA_NEIGHBORHOOD_CLAMPING,
            value.graphics.taa_neighborhood_clamping.to_string(),
        );
        set_env(
            ENV_TAA_JITTER_SCALE,
            value.graphics.taa_jitter_scale.to_string(),
        );
        set_env(ENV_SSAO_ENABLED, bool_text(value.graphics.ssao_enabled));
        set_env(
            ENV_SSAO_RADIUS_WS,
            value.graphics.ssao_radius_ws.to_string(),
        );
        set_env(
            ENV_SSAO_INTENSITY,
            value.graphics.ssao_intensity.to_string(),
        );
        set_env(
            ENV_SSAO_QUALITY_STEPS,
            value.graphics.ssao_quality_steps.to_string(),
        );
        set_env(
            ENV_SSAO_HALF_RESOLUTION,
            bool_text(value.graphics.ssao_half_resolution),
        );
        set_env(ENV_SSR_ENABLED, bool_text(value.graphics.ssr_enabled));
        set_env(ENV_SSR_INTENSITY, value.graphics.ssr_intensity.to_string());
        set_env(ENV_SSR_MAX_DISTANCE_METERS, value.graphics.ssr_max_distance_m.to_string());
        set_env(ENV_SSR_THICKNESS_METERS, value.graphics.ssr_thickness_m.to_string());
        set_env(ENV_SSR_STRIDE_METERS, value.graphics.ssr_stride_m.to_string());
        set_env(ENV_SSR_ROUGHNESS_CUTOFF, value.graphics.ssr_roughness_cutoff.to_string());
        set_env(ENV_SSR_MAX_STEPS, value.graphics.ssr_max_steps.to_string());
        set_env(ENV_VOLUMETRIC_FOG_ENABLED, bool_text(value.graphics.volumetric_fog_enabled));
        set_env(ENV_FROXEL_TILE_SIZE_PX, value.graphics.froxel_tile_size_px.to_string());
        set_env(ENV_FROXEL_DEPTH_SLICES, value.graphics.froxel_depth_slices.to_string());
        set_env(ENV_FROXEL_MAX_DISTANCE_METERS, value.graphics.froxel_max_distance_m.to_string());
        set_env(ENV_FROXEL_TEMPORAL_FEEDBACK, value.graphics.froxel_temporal_feedback.to_string());
        set_env(ENV_FROXEL_ANISOTROPY, value.graphics.froxel_anisotropy.to_string());
        set_env(ENV_BLOOM_ENABLED, bool_text(value.graphics.bloom_enabled));
        set_env(
            ENV_BLOOM_THRESHOLD,
            value.graphics.bloom_threshold.to_string(),
        );
        set_env(ENV_BLOOM_KNEE, value.graphics.bloom_knee.to_string());
        set_env(
            ENV_BLOOM_INTENSITY,
            value.graphics.bloom_intensity.to_string(),
        );
        set_env(ENV_BLOOM_RADIUS, value.graphics.bloom_radius.to_string());
        set_env(
            ENV_DOF_ENABLED,
            bool_text(value.graphics.depth_of_field_enabled),
        );
        set_env(
            ENV_MOTION_BLUR_ENABLED,
            bool_text(value.graphics.motion_blur_enabled),
        );
        set_env(
            ENV_SUN_RAYS_ENABLED,
            bool_text(value.graphics.sun_rays_enabled),
        );
        set_env(
            ENV_SHADOWS_ENABLED,
            bool_text(value.graphics.shadows_enabled),
        );
        set_env(ENV_SHADOW_QUALITY, value.graphics.shadow_quality.as_str());
        set_env(
            ENV_SHADOW_CASCADE_COUNT,
            value.graphics.shadow_cascade_count.to_string(),
        );
        set_env(
            ENV_SHADOW_MAP_RESOLUTION,
            value.graphics.shadow_map_resolution.to_string(),
        );
        set_env(ENV_SHADOW_FILTER, value.graphics.shadow_filter.as_str());
        set_env(
            ENV_SHADOW_MAX_DISTANCE,
            value.graphics.shadow_max_distance.to_string(),
        );
        set_env(
            ENV_SHADOW_SOFTNESS,
            value.graphics.shadow_softness.to_string(),
        );
        set_env(ENV_SHADOW_BIAS, value.graphics.shadow_bias.to_string());
        set_env(
            ENV_SHADOW_NORMAL_BIAS,
            value.graphics.shadow_normal_bias.to_string(),
        );
        set_env(
            ENV_SHADOW_CONTACT_STRENGTH,
            value.graphics.shadow_contact_strength.to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_LIGHT_RADIUS_DEGREES,
            value.graphics.shadow_pcss_light_radius_degrees.to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_BLOCKER_RADIUS_TEXELS,
            value.graphics.shadow_pcss_blocker_radius_texels.to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_MAX_FILTER_RADIUS_TEXELS,
            value
                .graphics
                .shadow_pcss_max_filter_radius_texels
                .to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_BLOCKER_SAMPLES,
            value.graphics.shadow_pcss_blocker_samples.to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_FILTER_SAMPLES,
            value.graphics.shadow_pcss_filter_samples.to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_MIN_FILTER_RADIUS_TEXELS,
            value
                .graphics
                .shadow_pcss_min_filter_radius_texels
                .to_string(),
        );
        set_env(
            ENV_SHADOW_PCSS_STABLE_KERNEL_TEXELS,
            value.graphics.shadow_pcss_stable_kernel_texels.to_string(),
        );
        set_env(
            ENV_VIEW_DISTANCE_METERS,
            value.graphics.view_distance_meters.to_string(),
        );
        set_env(ENV_LOD_QUALITY, value.graphics.lod_quality.as_str());
        set_env(
            ENV_LOD_DISTANCE_SCALE,
            value.graphics.lod_distance_scale.to_string(),
        );
        set_env(ENV_TEXTURE_QUALITY, value.graphics.texture_quality.as_str());
        set_env(ENV_ANISOTROPY, value.graphics.anisotropy.to_string());
        set_env(
            ENV_GPU_SCENE_TABLES_ENABLE,
            bool_text(value.graphics.gpu_scene_tables_enabled),
        );
        set_env(
            ENV_GPU_DRIVEN_INDIRECT_ENABLE,
            bool_text(value.graphics.gpu_driven_indirect_enabled),
        );
        set_env(
            ENV_GPU_DRIVEN_SHADOW_INDIRECT_ENABLE,
            bool_text(value.graphics.gpu_driven_shadow_indirect_enabled),
        );
        set_env(
            ENV_GEOMETRY_ARENA_VERTEX_PAGE_MIB,
            value.graphics.geometry_arena_vertex_page_mib.to_string(),
        );
        set_env(
            ENV_GEOMETRY_ARENA_INDEX_PAGE_MIB,
            value.graphics.geometry_arena_index_page_mib.to_string(),
        );
        set_env(
            ENV_GEOMETRY_ARENA_MAX_PAGES,
            value.graphics.geometry_arena_max_pages.to_string(),
        );
        set_env(ENV_WINDOW_MODE, value.display.window_mode.as_str());
        set_env(ENV_VSYNC, bool_text(value.display.vsync));
        set_env(
            ENV_REFRESH_RATE_MILLIHZ,
            value.display.refresh_rate_millihz.to_string(),
        );
        set_env(ENV_HDR_MODE, value.display.hdr.as_str());
        set_env(ENV_FRAME_LIMIT, value.display.frame_limit.to_string());
    }
}
