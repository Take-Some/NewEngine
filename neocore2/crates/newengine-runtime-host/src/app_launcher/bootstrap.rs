use newengine_asset_bootstrap_runtime::{collect_app_asset_roots, mount_asset_roots_best_effort};
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use newengine_assets::AssetServiceClient;
use newengine_core::{
    ConfigPaths, Engine, EngineError, EngineResult, PluginDiscoveryRoot, StartupConfig,
    StartupLoader,
};
use newengine_project_api::{
    ContentMountRegistry, ProjectContentMountState, RuntimeLaunchProfile, GAME_ROOT_ENV,
    PROJECT_DIR_ENV, PROJECT_MANIFEST_ENV, PROJECT_ROOT_ENV, PROJECT_STARTUP_SCENE_ENV,
    ROOT_DIR_ENV,
};
use newengine_project_runtime::{
    game_manifest_request_from_environment, load_project_from_request_with_launch,
    project_launch_request_from_environment, project_request_from_environment,
    register_engine_bootstrap_asset_roots, ProjectRuntimeContext, RuntimeCompositionContext,
};

use crate::engine_factory::build_engine_from_startup_with_host;

use super::boot_options::{apply_declared_boot_options_env, boot_option_enabled};
use super::types::{
    RuntimeHostAppProfile, RuntimeHostFrontend, RuntimeHostFrontendContext, RuntimeHostLauncher,
};

const HEADLESS_PREFERRED_TAGS: &[newengine_service_api::SystemTag] =
    &[newengine_service_api::SystemTag::new(
        newengine_service_api::system_tag::HEADLESS,
    )];
const HEADLESS_FORBIDDEN_TAGS: &[newengine_service_api::SystemTag] =
    &[newengine_service_api::SystemTag::new(
        newengine_service_api::system_tag::HEADFUL,
    )];

fn composition_for_launch(
    composition: newengine_service_api::EngineCompositionSpec,
    runtime_context: Option<&RuntimeCompositionContext>,
) -> newengine_service_api::EngineCompositionSpec {
    let headless = runtime_context.is_some_and(|context| {
        matches!(
            context.launch_profile,
            RuntimeLaunchProfile::Server | RuntimeLaunchProfile::Test
        )
    });
    if headless {
        composition
            .with_preferred_tags(HEADLESS_PREFERRED_TAGS)
            .with_forbidden_tags(HEADLESS_FORBIDDEN_TAGS)
    } else {
        composition
    }
}

impl<P> RuntimeHostLauncher<P>
where
    P: RuntimeHostAppProfile,
{
    pub fn run_with_frontend<F>(&self, frontend: &F) -> EngineResult<()>
    where
        F: RuntimeHostFrontend<P>,
    {
        self.early_log(format_args!("run.begin app={}", self.spec.app_name));
        let run_id = newengine_core::init_run_id().to_owned();
        self.bind_early_log_to_run(&run_id);
        self.early_log(format_args!("run_id.init.ok run_id={}", run_id));
        newengine_ulog_api::ulog::info_event!(
            "engine.startup.run_id",
            "Run ID initialized",
            {
                "app_name": self.spec.app_name,
                "run_id": run_id.as_str()
            }
        );
        newengine_core::crash::record_breadcrumb(format!(
            "{} launcher: run start run_id={}",
            self.spec.app_name, run_id
        ));

        // Capture process environment exactly once. From this point forward launcher-derived
        // policy is written only into this Engine instance's HostContext snapshot.
        let host_context = newengine_plugin_host::create_host_context_with_environment_snapshot(
            std::env::vars_os(),
        );
        host_context.set_environment_var("NEWENGINE_RUN_ID", run_id.as_str());
        self.spec.apply_env_defaults(&host_context);
        let root_dir = install_root_dir_authority(&host_context)?;
        self.early_log(format_args!(
            "path.authority.root root_dir={}",
            root_dir.display()
        ));
        let boot_options = self.profile.boot_options();
        apply_declared_boot_options_env(self.spec.app_name, boot_options, &host_context);

        self.install_error_reporter(&host_context);

        // PreInit providers are composition-owned. A profile may register an
        // alternative engine.host.capabilities route before the native fallback
        // is considered.
        self.profile.register_preinit_provider_routes_best_effort();

        // Void Engine Host PreInit is deliberately before project/runtime composition.
        // OS/hardware discovery produces immutable DTOs and installs only generic
        // gateway-selection policy; no game module or concrete runtime exists yet.
        let host_preinit = crate::preinit::run_host_preinit();

        let editor_project_request =
            project_request_from_environment(|name| host_context.environment_var_os(name));
        let game_request =
            game_manifest_request_from_environment(|name| host_context.environment_var_os(name));
        if game_request.is_none()
            && boot_option_enabled(
                boot_options,
                super::boot_options::RuntimeHostBootOption::ProjectBrowser,
            )
            && !super::env_bool("NEWENGINE_HEADLESS", false)
            && !super::env_bool("NEWENGINE_PROJECT_BROWSER_DISABLED", false)
        {
            return Err(EngineError::Other(
                "runtime host does not discover projects; editor must resolve game.toml before runtime launch"
                    .to_owned(),
            ));
        }
        let project_launch_request =
            project_launch_request_from_environment(|name| host_context.environment_var(name));
        let game_context = if let Some(context) = self.resolved_project.as_ref() {
            let context = context.clone();
            let editor_owned = editor_project_request.is_some();
            apply_project_environment(&host_context, &context, editor_owned);
            self.early_log(format_args!(
                "game.manifest.resolved_payload id={} root={} manifest={} mounts={} launch={} launch_profile={} runtime_profile={} editor_owned={}",
                context.manifest.id,
                context.project_root.display(),
                context.manifest_path.display(),
                context.mounts.mounts().len(),
                context.launch.preset_id,
                context.launch.profile.id(),
                context.launch.runtime_profile.as_deref().unwrap_or("app-default"),
                editor_owned,
            ));
            Some(context)
        } else {
            match game_request {
                Some(request) => {
                    let context = load_project_from_request_with_launch(
                        &request,
                        project_launch_request.as_deref(),
                    )
                    .map_err(|error| {
                        EngineError::Other(format!(
                            "game manifest load failed request='{}': {error}",
                            request.display()
                        ))
                    })?;
                    let editor_owned = editor_project_request.is_some();
                    apply_project_environment(&host_context, &context, editor_owned);
                    self.early_log(format_args!(
                        "game.manifest.loaded id={} root={} manifest={} mounts={} launch={} launch_profile={} runtime_profile={} editor_owned={}",
                        context.manifest.id,
                        context.project_root.display(),
                        context.manifest_path.display(),
                        context.mounts.mounts().len(),
                        context.launch.preset_id,
                        context.launch.profile.id(),
                        context.launch.runtime_profile.as_deref().unwrap_or("app-default"),
                        editor_owned,
                    ));
                    Some(context)
                }
                None => None,
            }
        };
        let runtime_context = game_context
            .as_ref()
            .map(RuntimeCompositionContext::from_project);
        if let Some(project) = game_context.as_ref() {
            crate::project_service::register_selected_project_service(&host_context, project)
                .map_err(|error| {
                    EngineError::Other(format!(
                        "selected project service registration failed project='{}': {error}",
                        project.manifest.id
                    ))
                })?;
        }
        install_project_scripting_provider_policy(&host_context, game_context.as_ref())?;
        frontend.prepare_startup(&self.profile, &self.spec)?;
        let mut startup = self.load_startup_config(&host_context)?;
        newengine_core::crash::record_breadcrumb(format!(
            "{} launcher: startup config loaded",
            self.spec.app_name
        ));
        self.configure_sharded_log_files(Arc::make_mut(&mut startup), &run_id, &host_context);

        let asset_roots = collect_app_asset_roots(self.spec.app_dir_name, self.spec.app_assets_env);
        if let Some(shared_content_root) = asset_roots.iter().find(|root| {
            root.file_name().and_then(|name| name.to_str()) == Some("Content")
                && root
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    == Some("Engine")
        }) {
            host_context.set_environment_var(
                "NEWENGINE_SHARED_CONTENT_ROOT",
                shared_content_root.as_os_str().to_os_string(),
            );
        }
        self.early_log(format_args!(
            "asset_roots.collected count={}",
            asset_roots.len()
        ));

        let mut content_mount_registry = ContentMountRegistry::default();
        register_engine_bootstrap_asset_roots(&mut content_mount_registry, &asset_roots)
            .map_err(EngineError::Other)?;
        if let Some(runtime) = runtime_context.as_ref() {
            for mount in runtime.mounts.mounts() {
                content_mount_registry
                    .register(mount.clone())
                    .map_err(EngineError::Other)?;
            }
        }

        // Environment ingress closes here. Everything below this boundary belongs to the
        // Engine instance and resolves compatibility knobs through this HostContext snapshot.
        let mut engine = self.build_engine(&startup, host_context.clone())?;
        // Runtime consumers receive an immutable snapshot. Provider selection has
        // already consumed the derived generic policy; systems never re-probe OS hardware.
        engine.resources_mut().insert(Arc::clone(&host_preinit));
        newengine_runtime_session_runtime::init_runtime_session_command_service();

        if let Some(game) = game_context.as_ref() {
            configure_game_plugin_roots(&mut engine, game)?;
        }
        engine
            .resources_mut()
            .insert(content_mount_registry.clone());
        if let Some(runtime) = runtime_context.clone() {
            engine.resources_mut().insert(runtime.scripts.clone());
            engine
                .resources_mut()
                .insert::<RuntimeCompositionContext>(runtime);
            if editor_project_request.is_some() {
                if let Some(project) = game_context.clone() {
                    engine
                        .resources_mut()
                        .insert::<ProjectRuntimeContext>(project);
                    let hot_reload_config = if runtime_context
                        .as_ref()
                        .is_some_and(|context| context.launch_profile == RuntimeLaunchProfile::Game)
                    {
                        newengine_asset_hot_reload_runtime::AssetFileWatcherConfig::interactive_play(
                        )
                    } else {
                        newengine_asset_hot_reload_runtime::AssetFileWatcherConfig::default()
                    };
                    newengine_asset_hot_reload_runtime::install_asset_file_watcher_with_config(
                        engine.resources_mut(),
                        hot_reload_config,
                    );
                }
            }
            engine
                .resources_mut()
                .insert(ProjectContentMountState::pending());
            engine.register_module(Box::new(
                super::project_content::DeferredProjectContentMountModule::new(asset_roots.clone()),
            ))?;
            self.early_log(format_args!(
                "runtime.content.bootstrap deferred-module=true mounts={} project_hot_reload={}",
                content_mount_registry.mounts().len(),
                editor_project_request.is_some(),
            ));
        } else {
            engine
                .resources_mut()
                .insert(ProjectContentMountState::default());
        }
        if let Some(composition) = self.profile.composition_spec() {
            let composition = composition_for_launch(composition, runtime_context.as_ref());
            if composition.schema_version
                != newengine_service_api::EngineCompositionSpec::SCHEMA_VERSION
            {
                return Err(EngineError::Other(format!(
                    "unsupported engine composition schema={} id='{}' expected={}",
                    composition.schema_version,
                    composition.id,
                    newengine_service_api::EngineCompositionSpec::SCHEMA_VERSION,
                )));
            }
            newengine_plugin_host::declare_engine_composition(composition)
                .map_err(EngineError::Other)?;
            self.early_log(format_args!(
                "composition.declared id='{}' schema={} requirements={} preferred_tags={} forbidden_tags={}",
                composition.id,
                composition.schema_version,
                composition.requirements.len(),
                composition.preferred_tags.len(),
                composition.forbidden_tags.len(),
            ));
        }
        self.profile.initialize_composition_services(
            &mut engine,
            host_preinit.as_ref(),
            runtime_context.as_ref(),
        )?;
        self.initialize_profile_and_plugins(&mut engine, &startup, boot_options)?;

        let asset_host = newengine_plugin_host::default_host_api();
        let assets_available =
            newengine_core::has_engine_gateway_route(newengine_assets_api::ENGINE_ASSET_SERVICE_ID);
        let assets = AssetServiceClient::new(asset_host.clone());
        self.early_log(format_args!(
            "asset_service.availability available={} gateway={} gateway_ready={}",
            assets_available,
            newengine_assets_api::ENGINE_ASSET_SERVICE_ID,
            assets_available,
        ));

        if assets_available {
            // Compatibility roots remain mounted at the legacy empty prefix so existing
            // asset refs continue to work. The new registry adds namespaced project/game/plugin/user
            // mounts alongside them instead of silently changing path semantics.
            mount_asset_roots_best_effort(&assets, &asset_roots);
            self.early_log(format_args!(
                "asset_roots.mount.requested count={} registry_mounts={} project={} project_mounts=deferred-module",
                asset_roots.len(),
                content_mount_registry.mounts().len(),
                editor_project_request.is_some(),
            ));
        } else {
            newengine_ulog_api::ulog::warn!(
                "{} launcher: engine.assets route unavailable after profile registration; asset root mount skipped until provider readiness",
                self.spec.app_name
            );
            newengine_core::crash::record_breadcrumb(format!(
                "{} launcher: engine.assets unavailable during initial asset mount",
                self.spec.app_name
            ));
        }

        frontend.launch(
            &self.profile,
            engine,
            RuntimeHostFrontendContext {
                launch_spec: &self.spec,
                startup: &startup,
                assets_available,
                assets: &assets,
                asset_roots: &asset_roots,
            },
        )
    }

    /// Generic runtime-host default: platformless/headless control plane. Windowed
    /// products must explicitly provide `newengine-windowed-host-runtime`.
    pub fn run(&self) -> EngineResult<()> {
        self.run_with_frontend(&crate::HeadlessRuntimeFrontend)
    }

    fn install_error_reporter(&self, host: &newengine_plugin_host::HostContextHandle) {
        self.early_log(format_args!("error_reporter.install.begin"));
        newengine_core::EngineErrorReporter::install(newengine_core::EngineErrorReporterConfig {
            crash: newengine_core::crash::CrashReporterConfig {
                product_name: self.spec.product_name.to_owned(),
                app_name: self.spec.app_name.to_owned(),
                app_version: self.spec.app_version.to_owned(),
                spawn_reporter: host
                    .environment_var_os("NEWENGINE_CRASH_REPORTER_PATH")
                    .is_some(),
                ..Default::default()
            },
            ..Default::default()
        });
        self.early_log(format_args!("error_reporter.install.ok"));
    }

    fn load_startup_config(
        &self,
        host: &newengine_plugin_host::HostContextHandle,
    ) -> EngineResult<Arc<StartupConfig>> {
        let startup_path = host
            .environment_var(crate::runtime_config::ENGINE_STARTUP_CONFIG_ENV)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.spec.startup_config_path.to_owned());
        let startup_path = resolve_startup_config_path(host, &startup_path)?;
        let startup_path_text = startup_path.to_string_lossy().into_owned();
        let paths = ConfigPaths::from_startup_str(&startup_path_text);
        self.early_log(format_args!(
            "startup.load.begin path={}",
            startup_path.display()
        ));
        let (mut startup, _report) = StartupLoader::load_json(&paths)?;
        expand_startup_path_authorities(host, &mut startup)?;
        publish_expanded_storage_roots(host, &startup);
        // Runtime-data readers use the globally published startup snapshot.
        // Republish it only after ROOT-DIR/PROJECT-DIR authority expansion.
        newengine_core::startup::set_last_startup_config(startup.clone());
        self.early_log(format_args!(
            "startup.load.ok modules_dir={} cache_files={} config={}",
            startup.modules_dir.display(),
            startup.resolved_cache_files_dir().display(),
            startup.resolved_config_dir().display()
        ));
        Ok(Arc::new(startup))
    }

    fn build_engine(
        &self,
        startup: &StartupConfig,
        host: newengine_plugin_host::HostContextHandle,
    ) -> EngineResult<Engine<()>> {
        self.early_log(format_args!("engine.build.begin"));
        let engine = build_engine_from_startup_with_host(startup, self.spec.fixed_dt_ms, host)?;
        self.early_log(format_args!("engine.build.ok"));
        newengine_core::crash::record_breadcrumb(format!(
            "{} launcher: host engine constructed",
            self.spec.app_name
        ));
        Ok(engine)
    }
}

include!("bootstrap/authority.rs");
include!("bootstrap/scripting.rs");
include!("bootstrap/environment.rs");
include!("bootstrap/tests.rs");
