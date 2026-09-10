#![forbid(unsafe_op_in_unsafe_fn)]

use std::sync::atomic::{AtomicBool, Ordering};

use newengine_core::render::{
    Extent2D, RectI32, RenderApi, RenderFrameDebugSnapshot, RenderTargetId, TextureFormat, Viewport,
};
use newengine_core::{EngineResult, ThreadPoolHandle};
use newengine_gameplay_world_runtime::gameplay::GameRunMode;
use newengine_render_feature_api::SceneExtractionCtx;
use newengine_render_frame_graph::{standard_runtime_frame, StandardRuntimePipelineDesc};
use newengine_scene::Scene;
use newengine_ui_api::{UiDrawList, UiLayerDrawPacketSet, UiPaintCommand};

use super::super::controller::RuntimeRenderController;
use super::super::error_policy::{
    is_backend_device_lost_error, is_transient_shader_pipeline_error,
};
use super::draw_lists::DrawListBuildCtx;
use super::feature_extraction::FeatureExtractionFrame;
use super::frame_envelope_builder::build_runtime_frame_envelope;
use super::frame_snapshots::SceneRenderSnapshot;
use super::frame_submit::submit_frame_envelope;
use super::frame_types::{PlayableFrameOutcome, RenderFrameScope, WorldFrameState};
use super::profiling::{emit_timed_profile_deferred, timed_profile_due, FrameCpuProfile};
use super::{lights, passes, picking, postfx, shadows};
use newengine_scene_bridge_runtime::scene_bridge::{
    apply_engine_view_postfx, EngineViewTransitionPhase,
};

mod fallback;
mod profile_trace;
mod shadow_trace;
mod submit;
mod task_events;

use shadow_trace::log_gpu_safe_profile_once;

pub(super) struct RenderFrameOrchestrator;
