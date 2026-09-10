use newengine_assets::{
    AssetErrorKind, AssetServiceClient, RuntimeTextureAsset, RuntimeTextureFormat,
};
use newengine_core::render::{
    Extent2D, GpuResourceResidencyState, TextureDesc, TextureFormat, TextureId, TextureMipDataDesc,
    TextureUsage,
};
use newengine_core::{TaskLane, TaskPriority, TaskRequest, ThreadPoolHandle};
use newengine_plugin_host::default_host_api;
use newengine_task_api::{task_domain, task_pass};
use parking_lot::Mutex;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use super::super::controller::RuntimeRenderController;
use super::super::material_bindings::MaterialTextureGpuResidency;
use super::super::state::{
    MaterialTextureDecodeJob, MaterialTexturePriority, MaterialTextureQueueEntry,
    MaterialTextureStreamingClass, MaterialTextureUploadCandidate,
};

const MATERIAL_TEXTURE_ASSET_RETRY_FRAMES: u64 = 4;
const MATERIAL_TEXTURE_ALLOCATION_STALL_WARN_MS: f32 = 16.67;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::render_controller) enum MaterialTextureReadyState {
    Ready(TextureId),
    Waiting,
    Failed,
}

#[inline]
fn quantize_unit_f32(value: f32) -> u16 {
    if !value.is_finite() {
        return 0;
    }
    (value.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

#[inline]
fn proximity_score(distance_m: f32) -> u16 {
    if !distance_m.is_finite() || distance_m < 0.0 {
        return 0;
    }
    // Hyperbolic falloff keeps useful ordering from first-person distances through large rooms
    // without baking a game-specific far plane into the scheduler.
    quantize_unit_f32(1.0 / (1.0 + distance_m))
}

#[inline]
fn runtime_texture_payload_bytes(asset: &RuntimeTextureAsset) -> usize {
    asset.mips.iter().map(|mip| mip.bytes.len()).sum()
}

#[inline]
fn streaming_priority_from_hints(
    class: MaterialTextureStreamingClass,
    visible_now: bool,
    screen_coverage: f32,
    distance_m: f32,
    material_importance: u8,
    player_weapon_relevance: u8,
    mip_urgency: u8,
) -> MaterialTexturePriority {
    MaterialTexturePriority {
        class,
        visible_now,
        screen_coverage_q: quantize_unit_f32(screen_coverage),
        proximity_q: proximity_score(distance_m),
        material_importance,
        player_weapon_relevance,
        mip_urgency,
    }
}

#[inline]
fn render_texture_format_from_runtime(format: RuntimeTextureFormat) -> TextureFormat {
    match format {
        RuntimeTextureFormat::Rgba8Unorm => TextureFormat::Rgba8Unorm,
        RuntimeTextureFormat::Rgba8Srgb => TextureFormat::Rgba8Srgb,
        RuntimeTextureFormat::Bc1RgbaUnorm => TextureFormat::Bc1RgbaUnorm,
        RuntimeTextureFormat::Bc1RgbaSrgb => TextureFormat::Bc1RgbaSrgb,
        RuntimeTextureFormat::Bc3RgbaUnorm => TextureFormat::Bc3RgbaUnorm,
        RuntimeTextureFormat::Bc3RgbaSrgb => TextureFormat::Bc3RgbaSrgb,
        RuntimeTextureFormat::Bc5RgUnorm => TextureFormat::Bc5RgUnorm,
        RuntimeTextureFormat::Bc7RgbaUnorm => TextureFormat::Bc7RgbaUnorm,
        RuntimeTextureFormat::Bc7RgbaSrgb => TextureFormat::Bc7RgbaSrgb,
    }
}

fn sanitize_material_texture_task_id(path: &str) -> String {
    let mut out = String::with_capacity(path.len().min(96));
    for ch in path.chars().take(96) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("unknown");
    }
    out
}

#[inline]
fn material_texture_decode_task_priority(priority: MaterialTexturePriority) -> TaskPriority {
    match priority.class {
        MaterialTextureStreamingClass::LaunchCritical => TaskPriority::Critical,
        MaterialTextureStreamingClass::StreamingCritical => TaskPriority::Interactive,
        MaterialTextureStreamingClass::Secondary => TaskPriority::Background,
    }
}

#[inline]
fn material_texture_decode_request(
    path: &str,
    frame_index: u64,
    priority: MaterialTexturePriority,
) -> TaskRequest {
    let task_path = sanitize_material_texture_task_id(path);
    TaskRequest::new("material.texture.decode")
        .with_source("render.controller")
        .with_owner("engine.render")
        .with_category("asset-decode")
        .with_lane(TaskLane::AssetIo)
        // Texture semantic decode is required for residency, but it is not frame-critical CPU
        // work. Simulation/RenderPrep interactive jobs must remain ahead of it in the shared pool.
        .with_priority(material_texture_decode_task_priority(priority))
        .with_frame_id(frame_index)
        .with_dependency_group(format!("frame.{frame_index}.asset-io.texture-decode"))
        .with_task_domain(task_domain::ENGINE_ASSETS)
        .with_task_pass(task_pass::TEXTURE_DECODE)
        .with_task_id(format!("render.material.texture.decode.{task_path}"))
}

include!("material_textures/transfer.rs");

include!("material_textures/queue.rs");
include!("material_textures/pump.rs");
include!("material_textures/ready.rs");

#[cfg(test)]
#[path = "material_textures/tests.rs"]
mod tests;
