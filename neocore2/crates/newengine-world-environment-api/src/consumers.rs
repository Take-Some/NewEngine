use newengine_world_api::WorldCellCoord;
use serde::{Deserialize, Serialize};

use crate::{Color3Dto, EnvironmentResidencyIntentDto, ExposureIntentDto, TimeOfDayPhase, Vec3Dto};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderEnvironmentPacketDto {
    pub visual_group_id: String,
    pub texture_dictionary_ref: String,
    pub sky_texture_ref: String,
    pub starfield_texture_ref: String,
    pub cloud_field_ref: String,
    pub cloud_density_texture_ref: String,
    pub cloud_detail_texture_ref: String,
    pub cloud_dither_texture_ref: String,
    pub sun_disk_texture_ref: String,
    pub moon_disk_texture_ref: String,
    pub weather_visual_ref: String,
    pub sun_direction: Vec3Dto,
    pub sun_color_linear: Color3Dto,
    pub sun_intensity_hint: f32,
    pub moon_direction: Vec3Dto,
    pub moon_color_linear: Color3Dto,
    pub moon_intensity_hint: f32,
    pub fog_density: f32,
    pub fog_height_falloff: f32,
    pub fog_color_linear: Color3Dto,
    pub cloud_coverage: f32,
    pub cloud_shadow_strength: f32,
    pub exposure: ExposureIntentDto,
}

impl Default for RenderEnvironmentPacketDto {
    #[inline]
    fn default() -> Self {
        Self {
            visual_group_id: String::new(),
            texture_dictionary_ref: String::new(),
            sky_texture_ref: String::new(),
            starfield_texture_ref: String::new(),
            cloud_field_ref: String::new(),
            cloud_density_texture_ref: String::new(),
            cloud_detail_texture_ref: String::new(),
            cloud_dither_texture_ref: String::new(),
            sun_disk_texture_ref: String::new(),
            moon_disk_texture_ref: String::new(),
            weather_visual_ref: String::new(),
            sun_direction: Vec3Dto::up(),
            sun_color_linear: Color3Dto::white(),
            sun_intensity_hint: 0.0,
            moon_direction: Vec3Dto::up(),
            moon_color_linear: Color3Dto::new(0.58, 0.66, 0.86),
            moon_intensity_hint: 0.0,
            fog_density: 0.0,
            fog_height_falloff: 0.12,
            fog_color_linear: Color3Dto::new(0.45, 0.50, 0.58),
            cloud_coverage: 0.0,
            cloud_shadow_strength: 0.0,
            exposure: ExposureIntentDto::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiEnvironmentObservationDto {
    pub time_of_day_normalized: f32,
    pub phase: TimeOfDayPhase,
    pub is_night: bool,
    pub visibility_multiplier: f32,
    pub audio_masking_multiplier: f32,
    pub weather_hazard_level: f32,
    pub shelter_score: f32,
    pub tags: Vec<String>,
}

impl Default for AiEnvironmentObservationDto {
    #[inline]
    fn default() -> Self {
        Self {
            time_of_day_normalized: 0.0,
            phase: TimeOfDayPhase::Night,
            is_night: true,
            visibility_multiplier: 1.0,
            audio_masking_multiplier: 0.0,
            weather_hazard_level: 0.0,
            shelter_score: 0.0,
            tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicsEnvironmentIntentDto {
    pub wind_velocity: Vec3Dto,
    pub gust_strength: f32,
    pub precipitation_intensity: f32,
    pub wetness_accumulation_rate: f32,
    pub snow_accumulation_rate: f32,
    pub surface_slipperiness_hint: f32,
}

impl Default for PhysicsEnvironmentIntentDto {
    #[inline]
    fn default() -> Self {
        Self {
            wind_velocity: Vec3Dto::zero(),
            gust_strength: 0.0,
            precipitation_intensity: 0.0,
            wetness_accumulation_rate: 0.0,
            snow_accumulation_rate: 0.0,
            surface_slipperiness_hint: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioEnvironmentPacketDto {
    pub rain_intensity: f32,
    pub wind_speed: f32,
    pub thunder_probability: f32,
    pub storm_distance: f32,
    pub indoor_occlusion_hint: f32,
    pub ambience_tags: Vec<String>,
}

impl Default for AudioEnvironmentPacketDto {
    #[inline]
    fn default() -> Self {
        Self {
            rain_intensity: 0.0,
            wind_speed: 0.0,
            thunder_probability: 0.0,
            storm_distance: 0.0,
            indoor_occlusion_hint: 0.0,
            ambience_tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamingEnvironmentHintsDto {
    pub residency_intents: Vec<EnvironmentResidencyIntentDto>,
    pub affected_cells: Vec<WorldCellCoord>,
    pub tags: Vec<String>,
}

impl Default for StreamingEnvironmentHintsDto {
    #[inline]
    fn default() -> Self {
        Self {
            residency_intents: Vec::new(),
            affected_cells: Vec::new(),
            tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentConsumerPacketsDto {
    pub render: RenderEnvironmentPacketDto,
    pub ai: AiEnvironmentObservationDto,
    pub physics: PhysicsEnvironmentIntentDto,
    pub audio: AudioEnvironmentPacketDto,
    pub streaming: StreamingEnvironmentHintsDto,
}

impl Default for EnvironmentConsumerPacketsDto {
    #[inline]
    fn default() -> Self {
        Self {
            render: RenderEnvironmentPacketDto::default(),
            ai: AiEnvironmentObservationDto::default(),
            physics: PhysicsEnvironmentIntentDto::default(),
            audio: AudioEnvironmentPacketDto::default(),
            streaming: StreamingEnvironmentHintsDto::default(),
        }
    }
}
