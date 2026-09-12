#![allow(clippy::too_many_arguments)]
use crate::profile_catalog::{EnvironmentProfileDescriptor, WeatherPresentationDescriptor};
use newengine_world_api::WorldCellCoord;
use newengine_world_environment_api::{
    AiEnvironmentObservationDto, AtmosphereStateDto, AudioEnvironmentPacketDto, CelestialBodyDto,
    CloudStateDto, EnvironmentConsumerPacketsDto, EnvironmentGameplayModifiersDto,
    EnvironmentResidencyIntentDto, ExposureIntentDto, PhysicsEnvironmentIntentDto,
    PrecipitationKind, RenderEnvironmentPacketDto, StreamingEnvironmentHintsDto, TimeOfDayPhase,
    TimeOfDayStateDto, Vec3Dto, WeatherStateDto, WindStateDto,
};

pub(crate) fn build_consumer_packets(
    profile: &EnvironmentProfileDescriptor,
    pattern: &WeatherPresentationDescriptor,
    time: &TimeOfDayStateDto,
    sun: &CelestialBodyDto,
    moon: &CelestialBodyDto,
    atmosphere: &AtmosphereStateDto,
    weather: &WeatherStateDto,
    clouds: &CloudStateDto,
    wind: &WindStateDto,
    lighting: &newengine_world_environment_api::EnvironmentLightingIntentDto,
    gameplay: &EnvironmentGameplayModifiersDto,
    exposure: &ExposureIntentDto,
    affected_cells: Vec<WorldCellCoord>,
) -> EnvironmentConsumerPacketsDto {
    let wind_velocity = Vec3Dto::new(
        wind.global_direction.x * wind.global_speed_mps,
        wind.global_direction.y * wind.global_speed_mps,
        wind.global_direction.z * wind.global_speed_mps,
    );
    let visual_assets = profile.visual_assets;
    let mut tags = weather.tags.clone();
    if wind.gust_strength > 0.45 {
        tags.push("wind.strong".to_owned());
    }
    tags.sort();
    tags.dedup();
    EnvironmentConsumerPacketsDto {
        render: RenderEnvironmentPacketDto {
            visual_group_id: visual_assets.id.to_owned(),
            texture_dictionary_ref: visual_assets.texture_dictionary_ref.to_owned(),
            sky_texture_ref: visual_assets.sky_texture_ref.to_owned(),
            starfield_texture_ref: visual_assets.starfield_texture_ref.to_owned(),
            cloud_field_ref: visual_assets.cloud_density_texture_ref.to_owned(),
            cloud_density_texture_ref: visual_assets.cloud_density_texture_ref.to_owned(),
            cloud_detail_texture_ref: visual_assets.cloud_detail_texture_ref.to_owned(),
            cloud_dither_texture_ref: visual_assets.cloud_dither_texture_ref.to_owned(),
            sun_disk_texture_ref: visual_assets.sun_disk_texture_ref.to_owned(),
            moon_disk_texture_ref: visual_assets.moon_disk_texture_ref.to_owned(),
            weather_visual_ref: pattern.weather_visual_ref.to_owned(),
            sun_direction: sun.direction_world,
            sun_color_linear: sun.color_linear,
            sun_intensity_hint: lighting.sun_lux_hint,
            moon_direction: moon.direction_world,
            moon_color_linear: moon.color_linear,
            moon_intensity_hint: lighting.moon_lux_hint,
            fog_density: atmosphere.fog_density,
            fog_height_falloff: atmosphere.fog_height_falloff,
            fog_color_linear: atmosphere.fog_color_linear,
            cloud_coverage: clouds.coverage,
            cloud_shadow_strength: clouds.shadow_strength,
            exposure: *exposure,
        },
        ai: AiEnvironmentObservationDto {
            time_of_day_normalized: time.normalized_day,
            phase: time.phase,
            is_night: matches!(time.phase, TimeOfDayPhase::Night),
            visibility_multiplier: gameplay.visibility_multiplier,
            audio_masking_multiplier: gameplay.audio_masking_multiplier,
            weather_hazard_level: gameplay.weather_hazard_level,
            shelter_score: gameplay.shelter_score,
            tags: tags.clone(),
        },
        physics: PhysicsEnvironmentIntentDto {
            wind_velocity,
            gust_strength: wind.gust_strength,
            precipitation_intensity: weather.precipitation.intensity,
            wetness_accumulation_rate: weather.wetness.accumulation_rate,
            snow_accumulation_rate: weather.snow.accumulation_rate,
            surface_slipperiness_hint: gameplay.surface_slipperiness_hint,
        },
        audio: AudioEnvironmentPacketDto {
            rain_intensity: rain_audio_intensity(weather),
            wind_speed: wind.global_speed_mps,
            thunder_probability: weather.thunder.probability,
            storm_distance: weather.thunder.distance_meters,
            indoor_occlusion_hint: 0.0,
            ambience_tags: tags.clone(),
        },
        streaming: StreamingEnvironmentHintsDto {
            residency_intents: clouds
                .volumes
                .iter()
                .chain(clouds.storm_cells.iter())
                .map(|object| EnvironmentResidencyIntentDto {
                    object_id: object.id,
                    owning_cells: object.owning_cells.clone(),
                    required_assets: required_assets_for_spatial_object(pattern, profile),
                    priority: object
                        .state_json
                        .get("priority")
                        .and_then(|v| v.as_str())
                        .unwrap_or("normal")
                        .to_owned(),
                    reason: object
                        .state_json
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("environment")
                        .to_owned(),
                })
                .collect(),
            affected_cells,
            tags,
        },
    }
}

fn rain_audio_intensity(weather: &WeatherStateDto) -> f32 {
    match weather.precipitation.kind {
        PrecipitationKind::Rain => weather.precipitation.intensity,
        _ => 0.0,
    }
}

fn required_assets_for_spatial_object(
    pattern: &WeatherPresentationDescriptor,
    profile: &EnvironmentProfileDescriptor,
) -> Vec<String> {
    let mut assets = vec![
        profile.visual_assets.cloud_density_texture_ref.to_owned(),
        profile.visual_assets.cloud_detail_texture_ref.to_owned(),
        profile.visual_assets.cloud_dither_texture_ref.to_owned(),
    ];
    assets.extend(
        pattern
            .required_assets
            .iter()
            .map(|asset| (*asset).to_owned()),
    );
    assets.sort();
    assets.dedup();
    assets
}
