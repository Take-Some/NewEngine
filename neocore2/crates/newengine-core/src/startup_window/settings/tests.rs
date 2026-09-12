#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_independent_aa_and_expensive_effects() {
        let mut settings = StartupLaunchSettings::default();
        settings.graphics.preset = GraphicsPreset::Custom;
        settings.graphics.msaa_samples = 8;
        settings.graphics.fxaa_enabled = true;
        settings.graphics.taa_enabled = true;
        settings.graphics.ssao_enabled = true;
        settings.display.render_scale = 4.0;
        settings.normalize();

        assert_eq!(settings.graphics.msaa_samples, 8);
        assert!(settings.graphics.fxaa_enabled);
        assert!(settings.graphics.taa_enabled);
        assert!(settings.graphics.ssao_enabled);
        assert_eq!(settings.display.render_scale, 2.0);
    }

    #[test]
    fn display_resolution_uses_auto_sentinel_and_clamps_authored_size() {
        let mut settings = StartupLaunchSettings::default();
        assert_eq!(settings.display.resolution, [0, 0]);

        settings.display.resolution = [32, 50_000];
        settings.normalize();
        assert_eq!(settings.display.resolution, [64, 16_384]);

        settings.display.resolution = [0, 1080];
        settings.normalize();
        assert_eq!(settings.display.resolution, [0, 0]);
    }

    #[test]
    fn disabled_shadows_force_off_quality() {
        let mut settings = StartupLaunchSettings::default();
        settings.graphics.shadows_enabled = false;
        settings.graphics.shadow_quality = ShadowQuality::Cinematic;
        settings.normalize();
        assert_eq!(settings.graphics.shadow_quality, ShadowQuality::Off);
    }

    #[test]
    fn normalizes_lod_and_shadow_overrides_without_forcing_scene_defaults() {
        let mut settings = StartupLaunchSettings::default();
        assert_eq!(settings.graphics.shadow_cascade_count, 0);
        assert_eq!(settings.graphics.shadow_map_resolution, 0);
        settings.graphics.view_distance_meters = 99_999.0;
        settings.graphics.lod_distance_scale = 9.0;
        settings.graphics.shadow_cascade_count = 99;
        settings.graphics.shadow_map_resolution = 3000;
        settings.normalize();
        assert_eq!(settings.graphics.view_distance_meters, 10_000.0);
        assert_eq!(settings.graphics.lod_distance_scale, 2.0);
        assert_eq!(settings.graphics.shadow_cascade_count, 4);
        assert_eq!(settings.graphics.shadow_map_resolution, 4096);

        settings.graphics.shadow_map_resolution = 16284;
        settings.normalize();
        assert_eq!(settings.graphics.shadow_map_resolution, 16284);
    }

    #[test]
    fn preset_is_only_a_starting_point_and_controls_remain_independent() {
        let mut graphics = StartupGraphicsSettings::default();
        graphics.apply_preset(GraphicsPreset::High);
        graphics.fxaa_enabled = false;
        graphics.msaa_samples = 8;
        graphics.mark_custom();
        graphics.normalize();

        assert_eq!(graphics.preset, GraphicsPreset::Custom);
        assert_eq!(graphics.view_distance_meters, 1500.0);
        assert_eq!(graphics.msaa_samples, 8);
        assert!(!graphics.fxaa_enabled);
        assert!(graphics.ssao_enabled);
    }

    #[test]
    fn gpu_driven_settings_are_fail_closed_and_bounded() {
        let mut settings = StartupLaunchSettings::default();
        assert!(!settings.graphics.gpu_scene_tables_enabled);
        assert!(!settings.graphics.gpu_driven_indirect_enabled);
        assert!(!settings.graphics.gpu_driven_shadow_indirect_enabled);
        assert_eq!(settings.graphics.geometry_arena_vertex_page_mib, 32);
        assert_eq!(settings.graphics.geometry_arena_index_page_mib, 16);
        assert_eq!(settings.graphics.geometry_arena_max_pages, 64);

        settings.graphics.gpu_driven_indirect_enabled = true;
        settings.graphics.gpu_driven_shadow_indirect_enabled = true;
        settings.graphics.geometry_arena_vertex_page_mib = 999;
        settings.graphics.geometry_arena_index_page_mib = 0;
        settings.graphics.geometry_arena_max_pages = 999;
        settings.normalize();
        assert!(!settings.graphics.gpu_driven_indirect_enabled);
        assert!(!settings.graphics.gpu_driven_shadow_indirect_enabled);
        assert_eq!(settings.graphics.geometry_arena_vertex_page_mib, 256);
        assert_eq!(settings.graphics.geometry_arena_index_page_mib, 2);
        assert_eq!(settings.graphics.geometry_arena_max_pages, 256);

        settings.graphics.gpu_scene_tables_enabled = true;
        settings.graphics.gpu_driven_indirect_enabled = true;
        settings.graphics.gpu_driven_shadow_indirect_enabled = true;
        settings.normalize();
        assert!(settings.graphics.gpu_driven_indirect_enabled);
        assert!(settings.graphics.gpu_driven_shadow_indirect_enabled);
    }
}
