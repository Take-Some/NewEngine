use super::*;

const EXPERIMENTAL_SPATIAL_CLOUD_SHADOWS_ENABLED: bool = false;

pub(crate) fn apply_sky_visuals(
    world: &mut newengine_ecs::World,
    frame: SkyFrameSample,
    atmosphere: Option<SkyAtmosphereRuntime>,
    dynamics: SkyDynamicsFrame,
) {
    let radius = atmosphere
        .as_ref()
        .map(|a| a.radius)
        .unwrap_or_else(|| newengine_game_data::default_game_data().world.sky.radius)
        .max(16.0);

    let entities = world
        .query::<SkyVisualRuntime>()
        .map(|(entity, visual)| (entity, visual.kind))
        .collect::<Vec<_>>();

    for (entity, kind) in entities {
        match kind {
            SkyVisualKind::Dome => {
                if let Some(primitive) = world.get_mut_tracked::<Primitive>(entity) {
                    let cloud_grade = (0.035 + dynamics.coverage * 0.075).clamp(0.035, 0.11);
                    let sky_grade = 1.0 - cloud_grade;
                    primitive.color = [
                        (frame.sky_tint[0] * sky_grade + frame.cloud_tint[0] * cloud_grade)
                            .clamp(0.0, 1.0),
                        (frame.sky_tint[1] * sky_grade + frame.cloud_tint[1] * cloud_grade)
                            .clamp(0.0, 1.0),
                        (frame.sky_tint[2] * sky_grade + frame.cloud_tint[2] * cloud_grade)
                            .clamp(0.0, 1.0),
                        1.0,
                    ];
                }
                if let Some(runtime) = world.get_mut_tracked::<EnvironmentDomeRenderState>(entity) {
                    // xy are temporal cloud phases; zw are the continuously
                    // integrated wind offset. The vertex shader keeps ordinary
                    // material UV scaling for non-sky instances.
                    runtime.uv_transform = [
                        dynamics.evolution_phase,
                        dynamics.lifecycle,
                        dynamics.cloud_offset.x,
                        dynamics.cloud_offset.y,
                    ];
                    runtime.material_params = [
                        dynamics.coverage,
                        dynamics.softness,
                        dynamics.haze,
                        dynamics.shadow_strength,
                    ];
                    runtime.emissive_params = [
                        1.0 + frame.rayleigh_strength.clamp(0.1, 2.5),
                        frame.mie_strength.clamp(0.05, 3.0),
                        frame.star_intensity.clamp(0.0, 1.5),
                    ];
                }
                if let Some(t) = world.get_mut_tracked::<Transform>(entity) {
                    t.position = Vec3::ZERO;
                    t.scale = Vec3::splat(radius);
                }
            }
        }
    }
}

pub fn tick_game_ready_sky_cycle(world: &mut newengine_ecs::World, dt: f32) {
    let sky_cycle_anchor = newengine_engine_runtime::gameplay::scene_entity_by_role(
        world,
        newengine_engine_runtime::gameplay::SceneEntityRole::SkyCycle,
    );
    let sun_anchor = newengine_engine_runtime::gameplay::scene_entity_by_role(
        world,
        newengine_engine_runtime::gameplay::SceneEntityRole::Sun,
    );
    let (frame, atmosphere, environment_frame, sun_entity) = {
        let atmosphere = world.resource::<SkyAtmosphereRuntime>().cloned();
        let Some(cycle) = world.resource_mut::<SkyCycleRuntime>() else {
            return;
        };

        cycle.anchor = sky_cycle_anchor.or(cycle.anchor);
        cycle.sun = sun_anchor.or(cycle.sun);
        let sun_entity = cycle.sun;
        let time_snapshot = time_snapshot_for_sky_cycle();
        if let Some(snapshot) = &time_snapshot {
            cycle.time_of_day_hours = (snapshot.game.normalized_day as f32 * 24.0).rem_euclid(24.0);
            cycle.day_index = snapshot.game.day_index;
        } else if dt > 0.0 {
            newengine_ulog_api::ulog::debug!(
                "game-ready sky cycle: engine.time route required for animated time; authored scene.day_night time remains fixed while degraded"
            );
        }

        let snapshot = time_snapshot.unwrap_or_else(|| authored_time_snapshot_for_sky_cycle(cycle));
        let environment_frame = environment_frame_for_sky_cycle(cycle, snapshot);
        let frame = if let Some(environment) = environment_frame.as_ref() {
            sample_sky_frame_from_environment(cycle, environment)
        } else {
            let to_sun = solar_direction_from_cycle(
                cycle.time_of_day_hours,
                cycle.latitude_degrees,
                cycle.axial_tilt_degrees,
                cycle.day_index,
            );
            sample_sky_frame(cycle, atmosphere.as_ref(), to_sun)
        };
        (frame, atmosphere, environment_frame, sun_entity)
    };

    let dynamics = update_sky_dynamics(world, &frame, dt);
    let physical_clouds = world
        .resource::<SkyDynamicsRuntime>()
        .copied()
        .unwrap_or_default();
    let sky_visual_ready = world.query::<EnvironmentDomeRenderState>().next().is_some();
    let spatial_shadow = if EXPERIMENTAL_SPATIAL_CLOUD_SHADOWS_ENABLED && sky_visual_ready {
        spatial_cloud_shadow_from_dynamics(&frame, &dynamics)
    } else {
        // Experimental projected cloud shadows are currently disabled. Keep the
        // cloud visual/atmosphere simulation, but send the neutral render state so
        // world lighting is not modulated by the spatial cloud-shadow field.
        CloudShadowRenderState::default()
    };
    let mut postfx = environment_frame
        .as_ref()
        .map(sky_postfx_from_environment)
        .unwrap_or_else(|| sky_postfx_from_authored_frame(&frame));
    let glare_visibility = dynamics.sun_occlusion.transmittance.powf(0.62);
    postfx.sun_glare_scale *= glare_visibility;
    postfx.sun_ray_scale *= dynamics.sun_occlusion.transmittance.powf(0.72);
    postfx.bloom_intensity *= 0.72 + glare_visibility * 0.28;
    // Real auto-exposure responds slower than a passing cloud. Apply only a
    // small compensation so the world visibly darkens when direct sun vanishes.
    postfx.exposure *= 1.0 + dynamics.sun_occlusion.smoothed_density * 0.025;
    let fog = environment_frame
        .as_ref()
        .map(|environment| {
            let visibility_m = environment
                .atmosphere
                .visibility_distance_meters
                .max(25.0);
            // Koschmieder extinction for ~2% contrast at the reported meteorological
            // visibility distance. This turns the weather model into a metric optical
            // coefficient instead of treating normalized fog_density as 1/metre.
            let visibility_extinction = 3.912 / visibility_m;
            let weather_extinction = environment.atmosphere.fog_density.clamp(0.0, 1.0)
                * 0.0035;
            newengine_engine_runtime::gameplay::EnvironmentFogRenderState {
                enabled: visibility_m < 80_000.0 || environment.atmosphere.fog_density > 0.002,
                density: visibility_extinction.max(weather_extinction).clamp(0.0, 0.08),
                // Environment authoring expresses falloff as a normalized weather
                // parameter. Convert it to a gentle inverse-metre coefficient.
                height_falloff: (environment.atmosphere.fog_height_falloff * 0.01)
                    .clamp(0.00005, 0.02),
                color_linear: [
                    environment.atmosphere.fog_color_linear.r.max(0.0),
                    environment.atmosphere.fog_color_linear.g.max(0.0),
                    environment.atmosphere.fog_color_linear.b.max(0.0),
                ],
                base_height_m: 0.0,
                start_distance_m: 2.0,
                max_opacity: (0.78 + environment.atmosphere.fog_density.clamp(0.0, 1.0) * 0.18)
                    .clamp(0.0, 0.96),
            }
        })
        .unwrap_or_default();
    world.insert_resource(postfx);
    world.insert_resource(fog);
    world.insert_resource(dynamics.sun_occlusion);
    world.insert_resource(spatial_shadow);
    world.insert_resource(
        newengine_engine_runtime::gameplay::SkyCloudProfileRenderState {
            profile0: [
                physical_clouds.smoothed_cloud_base_altitude_m,
                physical_clouds.smoothed_cloud_thickness_m,
                physical_clouds.smoothed_cloud_layer_density,
                physical_clouds.smoothed_high_cloud_coverage,
            ],
            profile1: [
                physical_clouds.smoothed_humidity,
                physical_clouds.smoothed_aerosol_density,
                physical_clouds.smoothed_precipitation_intensity,
                physical_clouds.smoothed_high_cloud_density,
            ],
        },
    );

    if let Some(environment) = environment_frame.as_ref() {
        if environment.frame_id <= 2 || environment.frame_id.is_multiple_of(600) {
            if !EXPERIMENTAL_SPATIAL_CLOUD_SHADOWS_ENABLED {
                newengine_ulog_api::ulog::debug!(
                    "game-ready live sky: experimental spatial cloud shadows disabled"
                );
            } else if !sky_visual_ready {
                newengine_ulog_api::ulog::warn!(
                    "game-ready live sky: visual cloud layer unavailable; spatial cloud shadows suppressed policy='visible-clouds-and-cloud-shadows-share-one-admission-state'"
                );
            }
            let (shadow_samples, shadow_min, shadow_max) =
                spatial_cloud_shadow_probe(&spatial_shadow, frame.to_sun);
            newengine_ulog_api::ulog::info!(
                "game-ready atmosphere: profile='{}' p={:.1}hPa T={:.2}C Td={:.2}C RH={:.3} q={:.2}g/kg rho={:.3}kg/m3 LCL={:.0}m PW={:.2}mm CWP={:.3}kg/m2 condensation={:.3} CAPE={:.0}J/kg CIN={:.0}J/kg cloud_top={:.0}m aerosol={:.3} visibility={:.0}m",
                environment.global.atmosphere_profile_ref,
                environment.atmosphere.surface_pressure_hpa,
                environment.atmosphere.temperature_celsius,
                environment.atmosphere.dew_point_celsius,
                environment.atmosphere.humidity,
                environment.atmosphere.specific_humidity_g_per_kg,
                environment.atmosphere.air_density_kg_m3,
                environment.atmosphere.lifting_condensation_level_meters,
                environment.atmosphere.precipitable_water_mm,
                environment.atmosphere.cloud_water_path_kg_m2,
                environment.atmosphere.condensation_potential,
                environment.atmosphere.cape_j_per_kg,
                environment.atmosphere.cin_j_per_kg,
                environment.atmosphere.convective_cloud_top_meters,
                environment.atmosphere.aerosol_density,
                environment.atmosphere.visibility_distance_meters,
            );
            newengine_ulog_api::ulog::info!(
                "game-ready live sky: frame={} profile='{}' weather='{}' tod={:.3} sun_y={:.3} sun_lux={:.1} day={:.3} overcast={:.3} target_clouds={:.3} live_clouds={:.3} absorption={:.3} sky_light={:.3} haze={:.3} -> clear_sun={:.3} world_sun={:.3} ambient={:.3} world_ambient={:.3} sun_cloud_raw={:.3} sun_cloud={:.3} optical_depth={:.3} transmittance={:.3} world_shadow={:.3} map=[{:.3},{:.3},{:.3},{:.3},{:.3}] spread={:.3} history={:.3} erosion=[freq:{:.4},strength:{:.3},fade:{:.1}] deck=[base:{:.0},thickness:{:.0},density:{:.3},high:{:.3}/{:.3}] air=[humidity:{:.3},aerosol:{:.3},precip:{:.3}] wind_offset=[{:.4},{:.4}] evolution={:.4} lifecycle={:.4} gust={:.3}",
                environment.frame_id,
                environment.global.active_environment_profile,
                environment.global.active_weather_profile,
                environment.time_of_day_normalized,
                environment.celestial.sun.direction_world.y,
                environment.celestial.sun.intensity_lux_hint,
                environment.time_of_day_state.day_blend,
                environment.sky.overcast_blend,
                environment.clouds.coverage,
                dynamics.coverage,
                environment.clouds.light_absorption,
                environment.lighting_intent.sky_light_intensity,
                environment.atmosphere.haze_amount,
                frame.sun_intensity,
                frame.sun_intensity
                    * spatial_shadow.map2[2]
                    * dynamics.sun_occlusion.direct_light_scale,
                frame.ambient_intensity,
                frame.ambient_intensity * spatial_shadow.broad_ambient_scale,
                dynamics.sun_occlusion.raw_density,
                dynamics.sun_occlusion.smoothed_density,
                dynamics.sun_occlusion.optical_depth,
                dynamics.sun_occlusion.transmittance,
                dynamics.sun_occlusion.world_shadow_strength,
                shadow_samples[0],
                shadow_samples[1],
                shadow_samples[2],
                shadow_samples[3],
                shadow_samples[4],
                shadow_max - shadow_min,
                spatial_shadow.map4[0],
                spatial_shadow.map4[1],
                spatial_shadow.map4[2],
                spatial_shadow.map4[3],
                physical_clouds.smoothed_cloud_base_altitude_m,
                physical_clouds.smoothed_cloud_thickness_m,
                physical_clouds.smoothed_cloud_layer_density,
                physical_clouds.smoothed_high_cloud_coverage,
                physical_clouds.smoothed_high_cloud_density,
                physical_clouds.smoothed_humidity,
                physical_clouds.smoothed_aerosol_density,
                physical_clouds.smoothed_precipitation_intensity,
                dynamics.cloud_offset.x,
                dynamics.cloud_offset.y,
                dynamics.evolution_phase,
                dynamics.lifecycle,
                dynamics.gust_factor,
            );
        }
    }

    if let Some(environment_frame) = environment_frame {
        let visual_assets = environment_frame.visual_assets.clone();
        let changed = world
            .resource::<GameReadyEnvironmentVisualAssetsRuntime>()
            .map(|current| current.visual_assets != visual_assets)
            .unwrap_or(true);
        if changed {
            newengine_ulog_api::ulog::debug!(
                "game-ready environment bridge: visual asset group='{}' dictionary='{}' sky='{}' sun='{}' moon='{}' cloud_density='{}' weather='{}'",
                visual_assets.visual_group_id,
                visual_assets.texture_dictionary_ref,
                visual_assets.sky_texture_ref,
                visual_assets.sun_disk_texture_ref,
                visual_assets.moon_disk_texture_ref,
                visual_assets.cloud_density_texture_ref,
                visual_assets.weather_visual_ref
            );
        }
        world.insert_resource(GameReadyEnvironmentVisualAssetsRuntime { visual_assets });
    }

    if let Some(ambient) = world.resource_mut::<AmbientLight>() {
        ambient.color = frame.ambient_color;
        ambient.intensity = frame.ambient_intensity * spatial_shadow.broad_ambient_scale;
    }

    let direction = -frame.to_sun;
    if let Some(sun_entity) = sun_entity {
        if let Some(light) = world.get_mut_tracked::<DirectionalLight>(sun_entity) {
            light.direction_ws = [direction.x, direction.y, direction.z];
            let cloud_neutral = [0.78, 0.84, 0.94];
            let cloud_mix = dynamics.sun_occlusion.smoothed_density * 0.10;
            light.color = sky_lerp3(frame.sun_color, cloud_neutral, cloud_mix);
            // Keep scene illumination synchronized with the same cloud optical
            // column that attenuates the solar disc/glare. Previously the log
            // reported this factor but the actual DirectionalLight ignored it.
            light.intensity = frame.sun_intensity
                * spatial_shadow.map2[2]
                * dynamics.sun_occlusion.direct_light_scale;
        }
    } else {
        newengine_ulog_api::ulog::warn!(
            "game-ready sky cycle: no SceneEntityRole::Sun anchor found; directional light update skipped policy='entity anchor is source of scene semantics'"
        );
    }

    world.insert_resource(WorldClearColor {
        color: frame.sky_tint,
    });
    apply_sky_visuals(world, frame, atmosphere, dynamics);
}
