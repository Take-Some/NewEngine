#![forbid(unsafe_op_in_unsafe_fn)]

//! Canonical launcher binding for the Game Ready FPS vertical slice.
//!
//! The binary entrypoint delegates here instead of manually assembling runtime
//! host services, boot options, engine modules, plugin preload, asset roots and
//! platform runtime execution.

use newengine_core::{Engine, EngineResult, StartupConfig};
use newengine_game_data::{
    GAME_READY_FPS_APP_NAME, GAME_READY_FPS_EARLY_LOG_FILE, GAME_READY_FPS_WINDOW_TITLE,
};
use newengine_runtime_host::app_launcher::{
    RuntimeHostAppProfile, RuntimeHostBootOption, RuntimeHostLaunchSpec, RuntimeHostLauncher,
};
use newengine_ui::{UiBuildFn, UiProviderKind};
use newengine_windowed_host_runtime::{WindowedHostFrontend, WindowedRuntimeHostProfile};

use crate::{
    GameReadyRuntimeProfile, GAME_APP_ASSETS_DIR_ENV, GAME_FIXED_DT_MS, GAME_READY_APP_DIR_NAME,
};

pub const GAME_READY_UI_SCREEN_PROFILE_ENV: &str =
    "NEWENGINE_PLUGIN_ENGINE_RUNTIME__ui__screen_profile__profile";
pub const GAME_READY_UI_PUBLISH_EDITOR_SHELL_ENV: &str =
    "NEWENGINE_PLUGIN_ENGINE_RUNTIME__ui__screen_profile__publish_editor_shell";

pub const GAME_READY_UI_PROFILE_GAME: &str = "game";

pub const GAME_READY_FPS_BOOT_OPTIONS: &[RuntimeHostBootOption] = &[
    RuntimeHostBootOption::RuntimeBootstrapOverlay,
    RuntimeHostBootOption::RuntimePlugins,
    RuntimeHostBootOption::PlatformWindow,
    RuntimeHostBootOption::RenderBackend,
    RuntimeHostBootOption::UiBackend,
];

/// Core GameReady runtime policy shared by editor-oriented and standalone game
/// launchers. Keep provider requirements here so app binaries never duplicate
/// engine capability policy.
pub const GAME_READY_CORE_ENV_POLICY: &[(&str, &str)] = &[
    ("NEWENGINE_GAME_READY_DEMO", "1"),
    ("NEWENGINE_REQUIRE_RENDER_BACKEND", "1"),
    ("NEWENGINE_REQUIRE_ASSET_MANAGER", "1"),
    ("NEWENGINE_REQUIRE_MATERIALS_BACKEND", "1"),
    ("NEWENGINE_PLUGIN_TARGET", "runtime"),
    // Game-ready launch must not dlopen bootstrap DLLs before platform/runtime
    // diagnostics are visible. Bootstrap plugins are loaded together with the
    // engine phase; stale DLLs can otherwise terminate the process with SEH
    // STATUS_ACCESS_VIOLATION before Rust can report an error.
    ("NEWENGINE_BOOTSTRAP_PLUGIN_PRELOAD", "deferred"),
    // Filesystem hot reload is an editor/authoring facility. A standalone GameReady launch must
    // never spend background CPU recursively stat'ing project content once per second.
    ("NEWENGINE_DISABLE_ASSET_HOT_RELOAD", "1"),
    // Profile-owned render startup policy: keep the viewport alive while the
    // real authored GLSL is being compiled asynchronously by engine.threading.
    // Users can override this env value before launch; it is still reported in
    // shader diagnostics as an explicit degraded policy.
    ("NEWENGINE_SHADER_ASYNC_PREBAKED_UNTIL_READY", "1"),
    // Keep the loading projection visible until every non-optional scene
    // material texture is GPU-resident. Partial material residency is an explicit
    // opt-in override, never the shipping GameReady default.
    ("NEWENGINE_SCENE_TEXTURE_LAUNCH_MIN_READY", "0"),
    ("NEWENGINE_SCENE_TEXTURE_LAUNCH_MIN_RATIO", "1.00"),
    ("NEWENGINE_SCENE_TEXTURE_GATE_SOFT_TIMEOUT_FRAMES", "1800"),
    ("NEWENGINE_SCENE_TEXTURE_GATE_SOFT_TIMEOUT_MS", "90000"),
];

/// Canonical env policy for the shipping FPS vertical slice.
pub const GAME_READY_FPS_ENV_POLICY: &[(&str, &str)] = &[
    ("NEWENGINE_GAME_FPS_DEMO", "1"),
    ("NEWENGINE_GAME_READY_DEMO", "1"),
    ("NEWENGINE_REQUIRE_RENDER_BACKEND", "1"),
    ("NEWENGINE_REQUIRE_ASSET_MANAGER", "1"),
    ("NEWENGINE_REQUIRE_MATERIALS_BACKEND", "1"),
    ("NEWENGINE_REQUIRE_UI_BACKEND", "1"),
    // Shipping gameplay presents only project-authored UI. Render diagnostics stay
    // available through telemetry/logging and are never mounted as a game HUD layer.
    ("NEWENGINE_PLUGIN_TARGET", "runtime"),
    ("NEWENGINE_BOOTSTRAP_PLUGIN_PRELOAD", "deferred"),
    ("NEWENGINE_DISABLE_ASSET_HOT_RELOAD", "1"),
    ("NEWENGINE_SHADER_ASYNC_PREBAKED_UNTIL_READY", "1"),
    ("NEWENGINE_SCENE_TEXTURE_LAUNCH_MIN_READY", "0"),
    ("NEWENGINE_SCENE_TEXTURE_LAUNCH_MIN_RATIO", "1.00"),
    ("NEWENGINE_SCENE_TEXTURE_GATE_SOFT_TIMEOUT_FRAMES", "1800"),
    ("NEWENGINE_SCENE_TEXTURE_GATE_SOFT_TIMEOUT_MS", "90000"),
    (GAME_READY_UI_SCREEN_PROFILE_ENV, GAME_READY_UI_PROFILE_GAME),
];

/// Applies the shipping GameReady policy before a registered runtime-profile handoff.
/// Explicit caller/project environment values win; profile defaults fill only missing keys.
#[inline]
pub fn apply_game_ready_fps_env_policy() {
    for &(key, value) in GAME_READY_FPS_ENV_POLICY {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
}

#[derive(Clone)]
pub struct GameReadyFpsApp {
    profile: GameReadyRuntimeProfile,
}

impl Default for GameReadyFpsApp {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl GameReadyFpsApp {
    #[inline]
    pub fn new() -> Self {
        Self {
            profile: GameReadyRuntimeProfile::standalone_game(),
        }
    }

    #[inline]
    pub fn with_profile(profile: GameReadyRuntimeProfile) -> Self {
        Self { profile }
    }

    #[inline]
    pub fn launch_spec() -> RuntimeHostLaunchSpec {
        RuntimeHostLaunchSpec {
            product_name: "North Star",
            app_name: GAME_READY_FPS_APP_NAME,
            app_version: env!("CARGO_PKG_VERSION"),
            startup_config_path: "config.json",
            fixed_dt_ms: GAME_FIXED_DT_MS,
            app_dir_name: GAME_READY_APP_DIR_NAME,
            app_assets_env: GAME_APP_ASSETS_DIR_ENV,
            early_log_file_name: GAME_READY_FPS_EARLY_LOG_FILE,
            default_profile_env: None,
            env_defaults: GAME_READY_FPS_ENV_POLICY,
        }
    }

    #[inline]
    pub fn run_process(self) -> ! {
        RuntimeHostLauncher::new(Self::launch_spec(), self)
            .run_process_with_frontend(WindowedHostFrontend::new(GAME_READY_FPS_WINDOW_TITLE))
    }

    #[inline]
    pub fn run_process_with_resolved_project(
        self,
        project: newengine_project_runtime::ProjectRuntimeContext,
    ) -> ! {
        RuntimeHostLauncher::new(Self::launch_spec(), self)
            .with_resolved_project(project)
            .run_process_with_frontend(WindowedHostFrontend::new(GAME_READY_FPS_WINDOW_TITLE))
    }
}

impl RuntimeHostAppProfile for GameReadyFpsApp {
    #[inline]
    fn composition_spec(&self) -> Option<newengine_service_api::EngineCompositionSpec> {
        Some(crate::provider_routes::GAME_READY_COMPOSITION_SPEC)
    }

    #[inline]
    fn distribution_runtime_unit_registrations(
        &self,
    ) -> &'static [newengine_runtime_host::app_launcher::RuntimeHostRuntimeUnitRegistration] {
        newengine_runtime_units::STATIC_RUNTIME_UNIT_REGISTRATIONS
    }

    #[inline]
    fn runtime_unit_registrations(
        &self,
    ) -> &'static [newengine_runtime_host::app_launcher::RuntimeHostRuntimeUnitRegistration] {
        crate::runtime_units::GAME_READY_RUNTIME_UNIT_REGISTRATIONS
    }

    #[inline]
    fn runtime_unit_requirements_for_runtime(
        &self,
        runtime: Option<&newengine_project_runtime::RuntimeCompositionContext>,
    ) -> Result<Vec<newengine_service_api::RuntimeUnitRequirementDescriptor>, String> {
        self.profile.runtime_unit_requirements_for_runtime(runtime)
    }

    #[inline]
    fn register_modules(
        &self,
        engine: &mut Engine<()>,
        startup: &StartupConfig,
    ) -> EngineResult<()> {
        self.profile.register_modules(engine, startup)
    }

    #[inline]
    fn boot_options(&self) -> Option<&'static [RuntimeHostBootOption]> {
        Some(GAME_READY_FPS_BOOT_OPTIONS)
    }

    #[inline]
    fn initialize_composition_services(
        &self,
        engine: &mut Engine<()>,
        host_preinit: &newengine_runtime_host::HostPreInitSnapshot,
        runtime: Option<&newengine_project_runtime::RuntimeCompositionContext>,
    ) -> EngineResult<()> {
        self.profile
            .initialize_composition_services(engine, host_preinit, runtime)
    }

    fn register_engine_provider_routes_best_effort(&self) {
        self.profile.register_engine_provider_routes_best_effort();
    }

    #[inline]
    fn bootstrap_content_best_effort(&self) {
        self.profile.bootstrap_content_best_effort();
    }
}

impl WindowedRuntimeHostProfile for GameReadyFpsApp {
    #[inline]
    fn ui_build_from_startup(&self, startup: &StartupConfig) -> Option<Box<dyn UiBuildFn>> {
        self.profile.ui_build_from_startup(startup)
    }

    #[inline]
    fn ui_provider_kind_from_startup(&self, startup: &StartupConfig) -> UiProviderKind {
        self.profile.ui_provider_kind_from_startup(startup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generic_game_ready_app_does_not_preinstall_a_concrete_game_module() {
        let app = GameReadyFpsApp::new();
        assert_eq!(app.profile.game_module_factory_count(), 0);
    }

    #[test]
    fn shipping_fps_requires_every_visual_runtime_backend() {
        for required in [
            RuntimeHostBootOption::RuntimePlugins,
            RuntimeHostBootOption::PlatformWindow,
            RuntimeHostBootOption::RenderBackend,
            RuntimeHostBootOption::UiBackend,
        ] {
            assert!(
                GAME_READY_FPS_BOOT_OPTIONS.contains(&required),
                "missing required Game Ready boot option: {required:?}"
            );
        }
    }

    #[test]
    fn shipping_fps_policy_mounts_authored_game_hud() {
        let value = |key: &str| {
            GAME_READY_FPS_ENV_POLICY
                .iter()
                .find_map(|(candidate, value)| (*candidate == key).then_some(*value))
        };
        assert_eq!(value("NEWENGINE_REQUIRE_UI_BACKEND"), Some("1"));
        assert_eq!(value("NEWENGINE_RUNTIME_DEBUG_OVERLAY"), None);
        assert_eq!(
            value(GAME_READY_UI_SCREEN_PROFILE_ENV),
            Some(GAME_READY_UI_PROFILE_GAME)
        );
        assert_eq!(value(GAME_READY_UI_PUBLISH_EDITOR_SHELL_ENV), None);
    }

    #[test]
    fn shipping_fps_disables_authoring_hot_reload_by_default() {
        let value = GAME_READY_FPS_ENV_POLICY.iter().find_map(|(key, value)| {
            (*key == "NEWENGINE_DISABLE_ASSET_HOT_RELOAD").then_some(*value)
        });
        assert_eq!(value, Some("1"));
        assert!(GAME_READY_CORE_ENV_POLICY
            .iter()
            .any(|(key, value)| { *key == "NEWENGINE_DISABLE_ASSET_HOT_RELOAD" && *value == "1" }));
    }

    #[test]
    fn shipping_fps_environment_policy_has_unique_keys() {
        let mut keys = HashSet::new();
        for (key, _) in GAME_READY_FPS_ENV_POLICY {
            assert!(
                keys.insert(*key),
                "duplicate Game Ready env policy key: {key}"
            );
        }
    }

    #[test]
    fn launch_spec_uses_authored_game_ready_scene_by_default() {
        let spec = GameReadyFpsApp::launch_spec();
        assert_eq!(spec.default_profile_env, None);
        assert_eq!(spec.env_defaults, GAME_READY_FPS_ENV_POLICY);
        assert!(!GAME_READY_FPS_WINDOW_TITLE.is_empty());
    }
}
