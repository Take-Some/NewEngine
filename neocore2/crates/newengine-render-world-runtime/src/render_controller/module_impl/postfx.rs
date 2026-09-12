#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_core::render::{AntiAliasingMode, PostFxFrameParams, SunPostFxParams};
use newengine_math::{Mat4, Vec3, Vec4};

use super::lights;

/// Apparent angular half-radius of the Sun as seen from Earth (~0.2666 degrees).
const SOLAR_ANGULAR_RADIUS_RAD: f32 = 0.004_653;
const SUN_PROJECTION_DISTANCE: f32 = 2_048.0;

pub(super) fn game_sun_postfx_params(
    world: &newengine_ecs::World,
    viewproj: Mat4,
    camera_position: Vec3,
) -> PostFxFrameParams {
    let mut params = PostFxFrameParams::default();
    let launch_graphics = newengine_core::startup_launch_settings().graphics;
    params.quality.anti_aliasing = match launch_graphics.msaa_samples {
        8 => AntiAliasingMode::Msaa8x,
        4 => AntiAliasingMode::Msaa4x,
        2 => AntiAliasingMode::Msaa2x,
        _ if launch_graphics.taa_enabled => AntiAliasingMode::Taa,
        _ if launch_graphics.fxaa_enabled => AntiAliasingMode::Fxaa,
        _ => AntiAliasingMode::None,
    };
    params.quality.fxaa.enabled = launch_graphics.fxaa_enabled;
    params.quality.fxaa.edge_threshold = launch_graphics.fxaa_edge_threshold;
    params.quality.fxaa.edge_threshold_min = launch_graphics.fxaa_edge_threshold_min;
    params.quality.fxaa.subpixel_quality = launch_graphics.fxaa_subpixel_quality;
    params.quality.taa.enabled = launch_graphics.taa_enabled;
    params.quality.taa.feedback = launch_graphics.taa_feedback;
    params.quality.taa.neighborhood_clamping = launch_graphics.taa_neighborhood_clamping;
    params.quality.taa.jitter_scale = launch_graphics.taa_jitter_scale;
    params.quality.ssao.enabled = launch_graphics.ssao_enabled;
    params.quality.ssao.radius_ws = launch_graphics.ssao_radius_ws;
    params.quality.ssao.intensity = launch_graphics.ssao_intensity;
    params.quality.ssao.quality_steps = launch_graphics.ssao_quality_steps;
    params.quality.ssao.half_resolution = launch_graphics.ssao_half_resolution;
    params.quality.ssr.enabled = launch_graphics.ssr_enabled;
    params.quality.ssr.intensity = launch_graphics.ssr_intensity;
    params.quality.ssr.max_distance_m = launch_graphics.ssr_max_distance_m;
    params.quality.ssr.thickness_m = launch_graphics.ssr_thickness_m;
    params.quality.ssr.stride_m = launch_graphics.ssr_stride_m;
    params.quality.ssr.roughness_cutoff = launch_graphics.ssr_roughness_cutoff;
    params.quality.ssr.max_steps = launch_graphics.ssr_max_steps;

    // Contact shadows belong to the shadow authoring policy, not to the backend
    // backend. Bridge the scene-level ShadowSettings into the renderer-facing
    // postfx DTO so the screen-space contact layer tracks the same authored
    // strength as CSM/PCSS instead of using a backend hard-coded constant.
    let shadow_settings = world
        .resource::<newengine_lighting::ShadowSettings>()
        .copied()
        .unwrap_or_default()
        .sanitized();
    params.quality.contact_shadows.enabled =
        shadow_settings.enabled && shadow_settings.contact_strength > 0.0;
    params.quality.contact_shadows.strength = shadow_settings.contact_strength;

    let sky_postfx = world
        .resource::<newengine_gameplay_world_runtime::gameplay::EnvironmentPostFxState>()
        .copied()
        .unwrap_or_default();
    params.display.exposure = sky_postfx.exposure;
    params.display.gamma = sky_postfx.gamma;
    params.display.black_lift = sky_postfx.black_lift;
    params.quality.color.saturation = sky_postfx.saturation;
    params.quality.color.contrast = sky_postfx.contrast;
    params.quality.color.temperature = sky_postfx.temperature;
    params.quality.color.vignette_strength = sky_postfx.vignette_strength;
    params.quality.color.local_contrast_strength = sky_postfx.local_contrast_strength;
    params.quality.color.dither_strength = sky_postfx.dither_strength;
    params.quality.bloom.enabled = launch_graphics.bloom_enabled;
    params.quality.bloom.threshold = launch_graphics.bloom_threshold;
    params.quality.bloom.knee = launch_graphics.bloom_knee;
    params.quality.bloom.intensity = launch_graphics.bloom_intensity;
    params.quality.bloom.radius = launch_graphics.bloom_radius;

    let fog = world
        .resource::<newengine_gameplay_world_runtime::gameplay::EnvironmentFogRenderState>()
        .copied()
        .unwrap_or_default();
    params.fog.enabled = fog.enabled && fog.density > 1.0e-7;
    params.fog.density = fog.density.clamp(0.0, 0.08);
    params.fog.height_falloff = fog.height_falloff.clamp(0.00005, 0.02);
    params.fog.color_linear = fog.color_linear.map(|component| component.max(0.0));
    params.fog.base_height_m = fog.base_height_m;
    params.fog.start_distance_m = fog.start_distance_m.max(0.0);
    params.fog.max_opacity = fog.max_opacity.clamp(0.0, 0.98);
    params.froxel_fog.enabled =
        params.fog.enabled && launch_graphics.volumetric_fog_enabled;
    params.froxel_fog.tile_size_px = launch_graphics.froxel_tile_size_px;
    params.froxel_fog.depth_slices = launch_graphics.froxel_depth_slices;
    params.froxel_fog.max_distance_m = launch_graphics.froxel_max_distance_m;
    params.froxel_fog.temporal_feedback = launch_graphics.froxel_temporal_feedback;
    params.froxel_fog.anisotropy = launch_graphics.froxel_anisotropy;

    let Some(sun) = lights::primary_directional_light(world) else {
        return params;
    };

    let incoming = Vec3::new(
        sun.direction_ws[0],
        sun.direction_ws[1],
        sun.direction_ws[2],
    )
    .normalize_or_zero();
    if incoming.length_squared() <= 1.0e-8 || sun.intensity <= 0.0 {
        return params;
    }

    // DirectionalLight.direction_ws points from the Sun into the scene. The
    // visible solar disc lies in the opposite direction from the camera.
    let to_sun = -incoming;
    let Some(screen) = project_direction_to_screen(viewproj, camera_position, to_sun) else {
        return params;
    };
    let screen_x = screen[0];
    let screen_y = screen[1];

    // Fade only at the viewport boundary. Do not use center alignment as an
    // artificial visibility term: a real lens still flares near the frame edge.
    let edge_distance = screen_x
        .min(1.0 - screen_x)
        .min(screen_y)
        .min(1.0 - screen_y);
    let edge_visibility = ((edge_distance + 0.06) / 0.10).clamp(0.0, 1.0);
    let on_screen = (-0.06..=1.06).contains(&screen_x) && (-0.06..=1.06).contains(&screen_y);
    let daylight = ((to_sun.y + 0.035) / 0.16).clamp(0.0, 1.0);
    let horizon_grazing = (1.0 - to_sun.y.abs()).clamp(0.0, 1.0);
    // Optical source energy is intentionally bounded. The sky HDR disc remains
    // physically bright, while raster lens/shaft effects react smoothly to a dim
    // sunrise, cloud attenuation, and a full daylight source without exploding.
    let optical_source_energy = (sun.intensity / (sun.intensity + 1.0)).clamp(0.0, 1.0);
    let visibility = if on_screen {
        daylight * edge_visibility
    } else {
        0.0
    };

    // Derive the disc radius from the active projection rather than hard-coding
    // a screen-space size. This keeps the visual Sun stable across FOV/aspect.
    let disk_radius = projected_solar_radius(viewproj, camera_position, to_sun)
        .unwrap_or(0.0045)
        .clamp(0.0015, 0.018);

    params.sun = SunPostFxParams {
        screen_position: [screen_x, screen_y],
        color: sun.color,
        direction: [incoming.x, incoming.y, incoming.z],
        intensity: sun.intensity,
        visibility,
        disk_radius,
        // Flare stays compact; the stronger low-angle response is carried by the
        // depth-aware raster shaft field. This mirrors the reference renderer's
        // separation between a tiny solar disc and a much broader scatter radius.
        flare_strength: (0.18 + 0.18 * horizon_grazing)
            * sky_postfx.sun_glare_scale
            * (0.60 + 0.40 * optical_source_energy),
        ray_strength: if launch_graphics.sun_rays_enabled {
            (0.18 + 0.34 * horizon_grazing) * sky_postfx.sun_ray_scale * optical_source_energy
        } else {
            0.0
        },
    };
    params
}

pub(super) fn apply_froxel_lighting(
    mut params: PostFxFrameParams,
    lights: newengine_render_feature_api::PackedLights,
) -> PostFxFrameParams {
    let dst = &mut params.froxel_fog.lighting;
    dst.directional_dir_intensity = lights.dir_dir_intensity;
    dst.directional_color = lights.dir_color;
    dst.point_pos_range = lights.point_pos_range;
    dst.point_color_intensity = lights.point_color_intensity;
    dst.point_count = lights.point_count_pad[0]
        .round()
        .clamp(0.0, newengine_core::render::MAX_FROXEL_POINT_LIGHTS as f32) as u32;
    dst.spot_pos_range = lights.spot_pos_range;
    dst.spot_dir_outer_cos = lights.spot_dir_outer_cos;
    dst.spot_color_intensity = lights.spot_color_intensity;
    dst.spot_inner_cos = lights.spot_inner_cos;
    dst.spot_count = lights.spot_count_pad[0]
        .round()
        .clamp(0.0, newengine_core::render::MAX_FROXEL_SPOT_LIGHTS as f32) as u32;
    dst.csm_enabled = lights.shadow_params[0] > 0.5 && lights.shadow_extra[1] >= 1.0;
    dst.csm_cascade_count = lights.shadow_extra[1]
        .round()
        .clamp(1.0, newengine_core::render::MAX_FROXEL_CSM_CASCADES as f32) as u32;
    for (dst_mvp, src_mvp) in dst
        .csm_light_mvp
        .iter_mut()
        .zip(lights.shadow_cascade_light_mvp.iter())
    {
        *dst_mvp = src_mvp.to_cols_array();
    }
    dst.csm_splits = lights.shadow_cascade_splits;
    dst.csm_shadow_params = lights.shadow_params;
    dst.csm_shadow_extra = lights.shadow_extra;
    params
}

fn project_direction_to_screen(
    viewproj: Mat4,
    camera_position: Vec3,
    direction: Vec3,
) -> Option<[f32; 2]> {
    let direction = direction.normalize_or_zero();
    if direction.length_squared() <= 1.0e-8 {
        return None;
    }
    let world = camera_position + direction * SUN_PROJECTION_DISTANCE;
    let clip = viewproj * Vec4::new(world.x, world.y, world.z, 1.0);
    if !clip.is_finite() || clip.w <= 1.0e-5 {
        return None;
    }
    let inv_w = 1.0 / clip.w;
    Some([clip.x * inv_w * 0.5 + 0.5, clip.y * inv_w * 0.5 + 0.5])
}

fn projected_solar_radius(viewproj: Mat4, camera_position: Vec3, to_sun: Vec3) -> Option<f32> {
    let to_sun = to_sun.normalize_or_zero();
    let center = project_direction_to_screen(viewproj, camera_position, to_sun)?;

    let mut tangent = to_sun.cross(Vec3::Y).normalize_or_zero();
    if tangent.length_squared() <= 1.0e-8 {
        tangent = to_sun.cross(Vec3::X).normalize_or_zero();
    }
    if tangent.length_squared() <= 1.0e-8 {
        return None;
    }

    let edge_direction = (to_sun * SOLAR_ANGULAR_RADIUS_RAD.cos()
        + tangent * SOLAR_ANGULAR_RADIUS_RAD.sin())
    .normalize_or_zero();
    let edge = project_direction_to_screen(viewproj, camera_position, edge_direction)?;
    let dx = edge[0] - center[0];
    let dy = edge[1] - center[1];
    Some(dx.hypot(dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn froxel_lighting_bridge_preserves_authoritative_world_light_packet() {
        let mut lights = newengine_render_feature_api::PackedLights::default();
        lights.point_count_pad[0] = 1.0;
        lights.point_pos_range[0] = [1.0, 2.0, 3.0, 8.0];
        lights.point_color_intensity[0] = [0.8, 0.6, 0.4, 12.0];
        lights.spot_count_pad[0] = 1.0;
        lights.spot_pos_range[0] = [4.0, 5.0, 6.0, 18.0];
        lights.spot_dir_outer_cos[0] = [0.0, -1.0, 0.0, 0.7];
        lights.spot_color_intensity[0] = [0.2, 0.4, 1.0, 7.0];
        lights.spot_inner_cos[0] = 0.9;
        lights.shadow_params = [1.0, 0.001, 0.75, 1.0];
        lights.shadow_extra = [0.0, 4.0, 0.0, 180.0];
        lights.shadow_cascade_splits = [12.0, 36.0, 84.0, 180.0];

        let params = apply_froxel_lighting(PostFxFrameParams::default(), lights);
        let fog = params.froxel_fog.lighting;
        assert_eq!(fog.point_count, 1);
        assert_eq!(fog.point_pos_range[0], [1.0, 2.0, 3.0, 8.0]);
        assert_eq!(fog.spot_count, 1);
        assert_eq!(fog.spot_inner_cos[0], 0.9);
        assert!(fog.csm_enabled);
        assert_eq!(fog.csm_cascade_count, 4);
        assert_eq!(fog.csm_splits, [12.0, 36.0, 84.0, 180.0]);
    }

    #[test]
    fn projected_solar_radius_is_positive_and_small() {
        let camera = Vec3::ZERO;
        let view = Mat4::IDENTITY;
        let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 16.0 / 9.0, 0.1, 5_000.0);
        let radius = projected_solar_radius(proj * view, camera, Vec3::new(0.0, 0.1, -1.0))
            .expect("sun must project");
        assert!(radius > 0.001, "radius={radius}");
        assert!(radius < 0.02, "radius={radius}");
    }

    #[test]
    fn projected_sun_center_tracks_view_projection() {
        let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 5_000.0);
        let center = project_direction_to_screen(proj, Vec3::ZERO, -Vec3::Z)
            .expect("forward sun must project");
        assert!((center[0] - 0.5).abs() < 1.0e-4, "x={}", center[0]);
        assert!((center[1] - 0.5).abs() < 1.0e-4, "y={}", center[1]);
    }
}
