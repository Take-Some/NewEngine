#![forbid(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;

use serde::Deserialize;

use newengine_plugin_manager_bridge::PluginManagerBridge;
use newengine_scene_bridge_runtime::scene_bridge::SceneBridge;
use newengine_viewport_bridge::ViewportBridge;

use super::error_policy::RenderBackendFailureState;
use super::gpu::{MaterialGpuPipelineKey, MaterialGpuPipelineProvider};
use super::state::{
    RenderBridgeState, RenderDiagnosticsRuntimeState, RenderFeatureProviderState,
    RenderFrameRuntimeState, RenderGpuSceneState, RenderRuntimeProfileState,
    RenderShadowRuntimeState, RenderUiSurfaceRuntimeState, RenderViewportState,
};
use newengine_core::render::{
    RenderBackendCapabilities, RenderBackendStatus, RenderExecutionCapabilities, RenderFeature,
};
use newengine_render_feature_api::{LightExtractionProvider, RenderDrawListProvider};

/// Engine-side render composition root.
///
/// This type is intentionally a composition root, not a renderer backend. It owns
/// runtime-facing state, delegates frame orchestration to module_impl, and submits
/// a typed RenderFrameEnvelope into the backend RenderApi adapter.

#[derive(Clone, Debug, Default)]
pub(super) struct RenderRuntimeAppPolicy {
    pub(super) ui_only: bool,
    pub(super) preview_only: bool,
    pub(super) viewport_pass: Option<bool>,
}

impl RenderRuntimeAppPolicy {
    pub(super) fn from_startup_config() -> Self {
        let mut policy = Self::default();
        let Some(startup) = newengine_core::startup::last_startup_config() else {
            return policy;
        };
        if let Some(value) = startup.plugins.get("engine.render") {
            if let Ok(config) = serde_json::from_value::<RenderPolicyConfig>(value.clone()) {
                policy.merge(config);
            }
        }
        if let Some(value) = startup
            .plugins
            .get("engine.runtime")
            .and_then(|value| value.get("render"))
        {
            if let Ok(config) = serde_json::from_value::<RenderPolicyConfig>(value.clone()) {
                policy.merge(config);
            }
        }
        if policy.ui_only {
            policy.preview_only = false;
            policy.viewport_pass = Some(false);
        }
        policy
    }

    pub(super) fn idle_preview_uses_ui_only(
        &self,
        external_extent_owned: bool,
        external_redraw_requested: bool,
    ) -> bool {
        self.preview_only && (!external_extent_owned || !external_redraw_requested)
    }

    fn merge(&mut self, config: RenderPolicyConfig) {
        if let Some(mode) = config.mode.as_deref() {
            if mode.eq_ignore_ascii_case("ui_only") || mode.eq_ignore_ascii_case("ui-only") {
                self.ui_only = true;
            } else if mode.eq_ignore_ascii_case("preview_only")
                || mode.eq_ignore_ascii_case("preview-only")
            {
                self.preview_only = true;
            }
        }
        if let Some(ui_only) = config.ui_only {
            self.ui_only = ui_only;
        }
        if let Some(preview_only) = config.preview_only {
            self.preview_only = preview_only;
        }
        if let Some(viewport_pass) = config.viewport_pass {
            self.viewport_pass = Some(viewport_pass);
        }
    }
}

#[derive(Debug, Deserialize)]
struct RenderPolicyConfig {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    ui_only: Option<bool>,
    #[serde(default)]
    preview_only: Option<bool>,
    #[serde(default)]
    viewport_pass: Option<bool>,
}

pub struct RuntimeRenderController {
    pub(super) bridges: RenderBridgeState,
    pub(super) viewport: RenderViewportState,
    pub(super) shadows: RenderShadowRuntimeState,
    pub(super) features: RenderFeatureProviderState,
    pub(super) gpu: RenderGpuSceneState,
    pub(super) frame: RenderFrameRuntimeState,
    pub(super) diagnostics: RenderDiagnosticsRuntimeState,
    pub(super) ui: RenderUiSurfaceRuntimeState,
    pub(super) runtime_profile: RenderRuntimeProfileState,
    pub(super) backend_failure: RenderBackendFailureState,
    pub(super) backend_execution: RenderExecutionCapabilities,
    pub(super) backend_gpu_driven_ready: bool,
    pub(super) app_policy: RenderRuntimeAppPolicy,
    pub(super) editor_viewport: newengine_editor_viewport_runtime::EditorViewportController,
    pub(super) editor_viewport_scene:
        newengine_scene_bridge_runtime::editor_viewport_adapter::EditorViewportSceneAdapter,
}

impl RuntimeRenderController {
    #[inline]
    pub(crate) fn runtime_profile(&self) -> &super::runtime_profile::RenderRuntimeProfile {
        &self.runtime_profile.profile
    }

    /// True only while a standalone preview-only tool owns an offscreen viewport.
    /// Such targets are sampled directly by retained UI, so they must remain LDR
    /// and must not depend on the gameplay HDR/postFX chain to become displayable.
    #[inline]
    pub(super) fn external_preview_target_active(&self) -> bool {
        self.app_policy.preview_only && self.bridges.viewport.external_extent_owned()
    }

    pub(crate) fn apply_backend_capability_profile(
        &mut self,
        capabilities: &RenderBackendCapabilities,
    ) {
        self.shadows.max_texture_dimension_2d = capabilities
            .limits
            .max_texture_dimension_2d
            .max(super::render_quality::SHADOW_RESOLUTION_MIN);
        self.runtime_profile
            .apply_hardware_tier_once(capabilities.hardware_tier);
        self.gpu.hair.apply_backend_capabilities(capabilities);
        self.backend_execution = capabilities.execution;
        self.backend_gpu_driven_ready = capabilities.supports(RenderFeature::StorageBuffers)
            && capabilities.supports(RenderFeature::HiZOcclusion)
            && capabilities.supports(RenderFeature::IndirectDraws)
            && capabilities.supports(RenderFeature::MultiDrawIndirect)
            && capabilities.supports(RenderFeature::IndirectDrawCount)
            && capabilities.supports(RenderFeature::VisibilityIndirectCompaction);
    }

    #[inline]
    pub(in crate::render_controller) fn gpu_driven_backend_ready(&self) -> bool {
        self.backend_gpu_driven_ready
    }

        #[inline]
    pub(in crate::render_controller) fn gpu_indirect_gbuffer_ready(&self) -> bool {
        newengine_runtime_env::var_bool("NEWENGINE_GPU_DRIVEN_INDIRECT_ENABLE", false)
            && self.backend_gpu_driven_ready
            && self
                .gpu
                .table_buffers
                .is_some_and(|buffers| buffers.frame_index == self.frame.frame_index)
            && self
                .gpu
                .indirect_stream
                .current()
                .is_some_and(|stream| {
                    stream.frame_index == self.frame.frame_index && stream.migration_ready()
                })
    }
pub(crate) fn backend_status_snapshot(&self) -> RenderBackendStatus {
        self.backend_failure.snapshot()
    }

    pub(super) fn restore_playable_view_after_ui_close(&mut self) {
        let restore_viewport = self.runtime_profile().ui.restore_viewport_pass_on_close;
        let invalidate_shadow_cache = self.runtime_profile().ui.invalidate_shadow_cache_on_close;
        let restore_input = self.runtime_profile().ui.restore_gameplay_input_on_close;
        if restore_viewport && self.viewport.pass_disabled && !self.backend_render_disabled() {
            newengine_ulog_api::ulog::warn!(
                "render controller: UI restore reopened viewport GPU pass after UI close"
            );
            self.viewport.pass_disabled = false;
        }
        if invalidate_shadow_cache {
            self.shadows.cache_valid = false;
        }
        if restore_input {
            self.frame.input_systems.set_enabled(
                newengine_input_systems_runtime::InputRuntimeSystem::Actions,
                true,
                "UI restore contract",
                self.frame.frame_index,
            );
            self.frame.input_systems.set_enabled(
                newengine_input_systems_runtime::InputRuntimeSystem::GameplayMovement,
                true,
                "UI restore contract",
                self.frame.frame_index,
            );
            self.frame.input_systems.set_enabled(
                newengine_input_systems_runtime::InputRuntimeSystem::CameraLook,
                true,
                "UI restore contract",
                self.frame.frame_index,
            );
        }
        newengine_core::crash::record_breadcrumb("render controller: UI restore contract applied");
    }

    /// Registers a profile-owned draw-list provider.
    ///
    /// Engine-runtime has no built-in draw-list defaults: the active profile owns
    /// terrain/mesh/UI extraction policy and must register it explicitly.
    #[inline]
    pub fn with_draw_list_provider(mut self, provider: Arc<dyn RenderDrawListProvider>) -> Self {
        self.features
            .draw_list_providers
            .register_provider(provider);
        self
    }

    /// Registers a profile-owned light extraction provider.
    ///
    /// Shadow planning policy belongs to the profile/render feature pack, not to
    /// the renderer backend adapter or reusable engine runtime.
    #[inline]
    pub fn with_light_extraction_provider(
        mut self,
        provider: Arc<dyn LightExtractionProvider>,
    ) -> Self {
        self.features
            .light_extraction_providers
            .register_provider(provider);
        self
    }

    /// Registers a host-side material-domain provider used by the reusable render
    /// controller. Profiles/features own these providers; engine-runtime only
    /// stores the trait object.
    #[inline]
    pub fn with_material_pipeline_provider(
        mut self,
        provider: Box<dyn MaterialGpuPipelineProvider>,
    ) -> Self {
        self.gpu.material.registry.register_provider(provider);
        self
    }

    /// Selects the material domain used by the current lit mesh/shadow passes.
    ///
    /// This keeps the reusable render controller free from profile-owned shader path
    /// knowledge: the profile chooses a domain key and registers the provider.
    #[inline]
    pub fn with_primary_lit_material_domain(mut self, key: MaterialGpuPipelineKey) -> Self {
        self.gpu.material.primary_lit_pipeline_key = Some(key);
        self
    }

    /// Registers a profile-owned gameplay execution provider.
    ///
    /// The reusable runtime owns only phase ordering. Concrete FPS/gameplay
    /// behavior is selected explicitly by the active application/profile.
    #[inline]
    pub fn with_gameplay_system_provider(
        mut self,
        provider: Arc<dyn newengine_gameplay_world_runtime::gameplay::GameplaySystemProvider>,
    ) -> Self {
        self.frame.gameplay_systems.register_provider(provider);
        self
    }

    /// Registers a profile-owned gameplay content provider.
    #[inline]
    pub fn with_gameplay_content_provider(
        mut self,
        provider: Arc<dyn newengine_gameplay_world_runtime::gameplay::GameplayContentProvider>,
    ) -> Self {
        self.frame.gameplay_content.register_provider(provider);
        self
    }

    /// Registers a profile-owned gameplay UI provider.
    #[inline]
    pub fn with_gameplay_ui_provider(
        mut self,
        provider: Arc<dyn newengine_gameplay_world_runtime::gameplay::GameplayUiProvider>,
    ) -> Self {
        self.frame.gameplay_ui.register_provider(provider);
        self
    }

    /// Registers a profile-owned gameplay physics-query provider.
    #[inline]
    pub fn with_gameplay_physics_query_provider(
        mut self,
        provider: Arc<dyn newengine_gameplay_world_runtime::gameplay::GameplayPhysicsQueryProvider>,
    ) -> Self {
        self.frame
            .gameplay_physics_queries
            .register_provider(provider);
        self
    }

    /// Registers a profile-owned world runtime provider.
    ///
    /// World assembly, streaming and environment policy are application/profile
    /// contributions. The reusable render loop only schedules this contract.
    #[inline]
    pub fn with_world_runtime_provider(
        mut self,
        provider: Arc<dyn newengine_world_runtime_api::WorldRuntimeProvider>,
    ) -> Self {
        self.frame.world_runtime.register_provider(provider);
        self
    }

    /// Enables or disables one semantic input system at runtime.
    ///
    /// This is the in-process control point used by scripted states such as cutscenes,
    /// dialogue or loading gates. Disabling a higher-level system does not stop raw
    /// device polling; it only suppresses the matching semantic effects.
    #[inline]
    pub fn set_input_system_enabled(
        &mut self,
        system: newengine_input_systems_runtime::InputRuntimeSystem,
        enabled: bool,
        reason: impl Into<String>,
    ) {
        self.frame
            .input_systems
            .set_enabled(system, enabled, reason, self.frame.frame_index);
    }

    #[inline]
    pub fn input_systems_snapshot(
        &self,
    ) -> newengine_input_systems_runtime::InputRuntimeSystemsSnapshot {
        self.frame.input_systems.snapshot(self.frame.frame_index)
    }

    #[inline]
    pub fn log_input_systems_snapshot(&self, reason: &str) {
        self.frame
            .input_systems
            .log_explicit_snapshot(self.frame.frame_index, reason);
    }

    pub(super) fn disable_viewport_pass(
        &mut self,
        phase: &'static str,
        error: impl std::fmt::Display,
    ) {
        let message = error.to_string();
        if !self.viewport.pass_disabled {
            newengine_ulog_api::ulog::error!(
                "render controller: viewport GPU pass disabled phase='{}' err='{}'",
                phase,
                message
            );
            newengine_core::crash::record_breadcrumb(format!(
                "render controller: viewport GPU pass disabled phase='{}' err='{}'",
                phase, message
            ));
        }
        self.viewport.pass_disabled = true;
    }

    #[inline]
    pub fn new(
        viewport_bridge: Arc<ViewportBridge>,
        plugins_bridge: Arc<PluginManagerBridge>,
        scene_bridge: Arc<SceneBridge>,
    ) -> Self {
        Self {
            bridges: RenderBridgeState::new(viewport_bridge, plugins_bridge, scene_bridge),
            viewport: RenderViewportState::new(),
            shadows: RenderShadowRuntimeState::new(),
            features: RenderFeatureProviderState::new(),
            gpu: RenderGpuSceneState::new(),
            frame: RenderFrameRuntimeState::new(),
            diagnostics: RenderDiagnosticsRuntimeState::new(),
            ui: RenderUiSurfaceRuntimeState::new(),
            runtime_profile: RenderRuntimeProfileState::new(),
            backend_failure: RenderBackendFailureState::new(),
            backend_execution: RenderExecutionCapabilities::default(),
            backend_gpu_driven_ready: false,
            app_policy: RenderRuntimeAppPolicy::from_startup_config(),
            editor_viewport: newengine_editor_viewport_runtime::EditorViewportController::default(),
            editor_viewport_scene:
                newengine_scene_bridge_runtime::editor_viewport_adapter::EditorViewportSceneAdapter::default(),
        }
    }
}

#[cfg(test)]
mod app_policy_tests {
    use super::*;

    #[test]
    fn preview_only_policy_skips_world_until_preview_extent_is_owned() {
        let policy = RenderRuntimeAppPolicy {
            preview_only: true,
            ..RenderRuntimeAppPolicy::default()
        };
        assert!(policy.idle_preview_uses_ui_only(false, false));
        assert!(policy.idle_preview_uses_ui_only(true, false));
        assert!(!policy.idle_preview_uses_ui_only(true, true));
    }

    #[test]
    fn ui_only_policy_does_not_activate_preview_world_path() {
        let policy = RenderRuntimeAppPolicy {
            ui_only: true,
            preview_only: false,
            viewport_pass: Some(false),
        };
        assert!(!policy.idle_preview_uses_ui_only(false, false));
    }
}
