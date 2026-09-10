#![forbid(unsafe_op_in_unsafe_fn)]

mod project_browser_settings;
mod standalone_package;

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::atomic::{AtomicBool, Ordering},
};

use newengine_plugin_api::Blob;
use newengine_plugin_host::{
    call_service_v1, default_host_api, has_service, init_host_context, PluginLoadOrigin,
    PluginManager,
};
use newengine_project_api::{
    runtime_profile_service_id, GAME_MANIFEST_ENV, PROJECT_BROWSER_PRESENT_METHOD_V1,
    PROJECT_BROWSER_SERVICE_ID, PROJECT_DIR_ENV, ROOT_DIR_ENV, RUNTIME_PROFILE_LAUNCH_METHOD_V1,
};
use newengine_project_runtime::{
    default_projects_root, game_manifest_request_from_process,
    load_project_from_request_with_launch, project_launch_request_from_process,
    project_request_from_cli, ProjectBrowserSelection,
};
use newengine_runtime_host::runtime_config::{load_engine_runtime_config, ENGINE_RUNTIME_MODE_ENV};
use project_browser_settings::{
    apply_project_browser_settings_patch, prepare_project_browser_config_document,
};

const DEFAULT_PROJECT_BROWSER_PLUGIN_ID: &str = "newengine.project-browser.egui";
const PROJECT_BROWSER_PLUGIN_ID_ENV: &str = "NEWENGINE_PROJECT_BROWSER_PLUGIN_ID";
static PROJECT_BROWSER_INVOCATION_ACTIVE: AtomicBool = AtomicBool::new(false);

struct ProjectBrowserInvocationLease;

impl ProjectBrowserInvocationLease {
    fn acquire() -> Result<Self, String> {
        PROJECT_BROWSER_INVOCATION_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| {
                "Project Browser invocation rejected because a selection window is already active"
                    .to_owned()
            })
    }
}

impl Drop for ProjectBrowserInvocationLease {
    fn drop(&mut self) {
        PROJECT_BROWSER_INVOCATION_ACTIVE.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectBrowserAction {
    Launch,
    BuildStandalone,
}

struct ProjectBrowserResult {
    selection: ProjectBrowserSelection,
    action: ProjectBrowserAction,
    build_options: standalone_package::StandaloneBuildOptions,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("NewEngine: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let (runtime_config_path, runtime_config) = load_engine_runtime_config()?;
    install_process_root_dir_authority(&runtime_config_path)?;
    runtime_config.apply_process_env(&runtime_config_path)?;

    if env::args().any(|arg| arg == "--build-standalone") {
        let manifest_path = project_request_from_cli().ok_or_else(|| {
            "--build-standalone requires --project <game.toml|project-directory>".to_owned()
        })?;
        let launch_request =
            project_launch_request_from_process().or_else(|| Some("game".to_owned()));
        let project =
            load_project_from_request_with_launch(&manifest_path, launch_request.as_deref())?;
        let build_options = standalone_build_options_from_cli()?;
        let output = standalone_package::build_game_ready_standalone_with_options(
            &runtime_config_path,
            &project,
            &build_options,
        )?;
        println!(
            "North Star: standalone Game Ready package built at '{}'",
            output.display()
        );
        return Ok(());
    }

    let project_browser_requested =
        env::args().any(|arg| arg == "--list-projects" || arg == "--project-browser");
    if project_browser_requested {
        return run_project_selection(&runtime_config_path);
    }

    let manifest_path = project_request_from_cli().or_else(game_manifest_request_from_process);
    let Some(manifest_path) = manifest_path else {
        if runtime_config.runtime.startup_window {
            return run_project_selection(&runtime_config_path);
        }
        return Err("runtime package is incomplete: game.toml was not found; pass --project <game.toml> or enable the startup project browser".to_owned());
    };
    let launch_request = project_launch_request_from_process()
        .or_else(|| Some(runtime_config.runtime.mode.launch_id().to_owned()));
    let project = load_project_from_request_with_launch(&manifest_path, launch_request.as_deref())?;
    dispatch_project_launch(&runtime_config_path, project)
}

fn dispatch_project_launch(
    runtime_config_path: &Path,
    project: newengine_project_runtime::ProjectRuntimeContext,
) -> Result<(), String> {
    // Project Browser is a startup chooser only. Editing-tools plugins remain in
    // normal runtime composition and are discovered as optional capabilities.
    exclude_project_browser_from_runtime();

    env::set_var(PROJECT_DIR_ENV, &project.project_root);
    env::set_var(GAME_MANIFEST_ENV, &project.manifest_path);
    env::remove_var("NEWENGINE_PROJECT");
    env::set_var(
        newengine_project_api::PROJECT_LAUNCH_PRESET_ENV,
        &project.launch.preset_id,
    );

    // The resolved Game/Server/Test launch owns the effective runtime mode.
    // Optional editing tools do not alter runtime mode or world ownership.
    env::set_var(ENGINE_RUNTIME_MODE_ENV, project.launch.profile.id());
    env::set_var("NEWENGINE_LAUNCH_PROFILE", project.launch.profile.id());
    env::set_var(
        "NEWENGINE_HEADLESS",
        if matches!(
            project.launch.profile,
            newengine_project_api::RuntimeLaunchProfile::Server
                | newengine_project_api::RuntimeLaunchProfile::Test
        ) {
            "1"
        } else {
            "0"
        },
    );
    // Publish the complete resolved project contract before loading a runtime
    // composition DLL. The composition may read screen/profile policy during
    // plugin initialization, before its own runtime host reloads game.toml.
    newengine_project_runtime::apply_resolved_project_launch_env(&project.launch);
    newengine_project_runtime::apply_project_ui_env(&project.manifest);

    if let Some(runtime_profile) = project
        .launch
        .runtime_profile
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        println!(
            "North Star: launching project '{}' profile='{}' runtime_profile='{}' launch='{}' game_manifest='{}' runtime_config='{}'",
            project.manifest.name,
            project.launch.profile.id(),
            runtime_profile,
            project.launch.preset_id,
            project.manifest_path.display(),
            runtime_config_path.display(),
        );
        if env_bool("NEWENGINE_LAUNCHER_DRY_RUN", false) {
            return Ok(());
        }
        return launch_runtime_profile_via_plugins(runtime_profile, &project);
    }

    let launcher = project
        .manifest
        .launcher
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "project '{}' launch '{}' does not declare runtime_profile or launcher in {}",
                project.manifest.id,
                project.launch.preset_id,
                project.manifest_path.display()
            )
        })?;
    if launcher.eq_ignore_ascii_case("newengine-launcher")
        || launcher.eq_ignore_ascii_case("NewEngineLauncher")
        || launcher.eq_ignore_ascii_case("newengine")
        || launcher.eq_ignore_ascii_case("NewEngine")
    {
        return Err("game launcher cannot recursively target NewEngine".to_owned());
    }

    let target = resolve_launcher_target(launcher)?;
    println!(
        "North Star: launching project '{}' profile='{}' via '{}' game_manifest='{}'",
        project.manifest.name,
        project.launch.profile.id(),
        launcher,
        project.manifest_path.display()
    );

    if env_bool("NEWENGINE_LAUNCHER_DRY_RUN", false) {
        println!("North Star: dry-run target={target:?}");
        return Ok(());
    }

    match target {
        LauncherTarget::Executable(path) => {
            let mut command = Command::new(&path);
            command
                .arg("--game-manifest")
                .arg(&project.manifest_path)
                .arg("--launch")
                .arg(&project.launch.preset_id);
            command.env(GAME_MANIFEST_ENV, &project.manifest_path).env(
                newengine_project_api::PROJECT_LAUNCH_PRESET_ENV,
                &project.launch.preset_id,
            );
            if env_bool("NEWENGINE_LAUNCHER_WAIT_CHILD", false) {
                let status = command
                    .status()
                    .map_err(|error| format!("launch '{}' failed: {error}", path.display()))?;
                if !status.success() {
                    return Err(format!("game launcher exited with {status}"));
                }
            } else {
                command
                    .spawn()
                    .map_err(|error| format!("launch '{}' failed: {error}", path.display()))?;
            }
        }
        LauncherTarget::Cargo { workspace, package } => {
            let mut command = Command::new("cargo");
            command
                .current_dir(&workspace)
                .arg("run")
                .arg("-p")
                .arg(&package)
                .arg("--")
                .arg("--game-manifest")
                .arg(&project.manifest_path)
                .arg("--launch")
                .arg(&project.launch.preset_id)
                .env(GAME_MANIFEST_ENV, &project.manifest_path)
                .env(
                    newengine_project_api::PROJECT_LAUNCH_PRESET_ENV,
                    &project.launch.preset_id,
                );
            if env_bool("NEWENGINE_LAUNCHER_WAIT_CHILD", false) {
                let status = command
                    .status()
                    .map_err(|error| format!("cargo launch package '{package}' failed: {error}"))?;
                if !status.success() {
                    return Err(format!("cargo game launcher exited with {status}"));
                }
            } else {
                command
                    .spawn()
                    .map_err(|error| format!("cargo launch package '{package}' failed: {error}"))?;
            }
        }
    }
    Ok(())
}

fn cli_option_value(name: &str) -> Option<String> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == name {
            return args.next().filter(|value| !value.trim().is_empty());
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            if !value.trim().is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn standalone_build_options_from_cli() -> Result<standalone_package::StandaloneBuildOptions, String>
{
    let target_os = cli_option_value("--standalone-target")
        .as_deref()
        .map(|value| {
            standalone_package::StandaloneTargetOs::from_id(value)
                .ok_or_else(|| format!("unknown standalone target OS '{value}'"))
        })
        .transpose()?
        .unwrap_or(standalone_package::StandaloneTargetOs::Windows);
    Ok(standalone_package::StandaloneBuildOptions {
        output_dir: cli_option_value("--standalone-output").map(PathBuf::from),
        package_name: cli_option_value("--standalone-name"),
        target_os,
        rebuild_plugins: !env::args().any(|arg| arg == "--standalone-no-rebuild-plugins"),
        include_source: !env::args().any(|arg| arg == "--standalone-no-source"),
    })
}

fn run_project_selection(runtime_config_path: &Path) -> Result<(), String> {
    env::set_var("NEWENGINE_HEADLESS", "0");

    let (manifest_path, launch_request) = if let Some(request) = project_request_from_cli() {
        // Explicit CLI project selection is the only chooser bypass. Inherited
        // NEWENGINE_PROJECT is deliberately ignored in project-selection mode
        (request, project_launch_request_from_process())
    } else {
        let root = default_projects_root().ok_or_else(|| {
            "project browser cannot discover Projects root; set NEWENGINE_PROJECTS_ROOT or pass --project <game.toml>"
                .to_owned()
        })?;
        if env::args().any(|arg| arg == "--list-projects") {
            for project in newengine_project_runtime::discover_projects(&root) {
                println!(
                    "{}\t{}\t{}",
                    project.id,
                    project.name,
                    project.manifest_path.display()
                );
            }
            return Ok(());
        }
        let result = present_project_browser_via_plugin(&root)?;
        if result.selection.cancelled {
            return Ok(());
        }
        let manifest_path = result
            .selection
            .manifest_path
            .ok_or_else(|| "project selection returned no game.toml".to_owned())?;
        if result.action == ProjectBrowserAction::BuildStandalone {
            let project = load_project_from_request_with_launch(
                &manifest_path,
                result.selection.launch_id.as_deref(),
            )?;
            let output = standalone_package::build_game_ready_standalone_with_options(
                runtime_config_path,
                &project,
                &result.build_options,
            )?;
            println!(
                "North Star: standalone Game Ready package built at '{}'",
                output.display()
            );
            return Ok(());
        }
        (manifest_path, result.selection.launch_id)
    };

    let project = load_project_from_request_with_launch(&manifest_path, launch_request.as_deref())?;
    dispatch_project_launch(runtime_config_path, project)
}

fn present_project_browser_via_plugin(root: &Path) -> Result<ProjectBrowserResult, String> {
    let _invocation_lease = ProjectBrowserInvocationLease::acquire()?;
    init_host_context();
    let host = default_host_api();
    let mut plugins = PluginManager::new();
    let plugin_id = env::var(PROJECT_BROWSER_PLUGIN_ID_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_PROJECT_BROWSER_PLUGIN_ID.to_owned());

    if !has_service(PROJECT_BROWSER_SERVICE_ID) {
        plugins
            .load_plugin_id_default_with_origin(
                &plugin_id,
                host,
                PluginLoadOrigin::FirstPartyPlugin,
            )
            .map_err(|error| format!("load Project Browser plugin '{plugin_id}': {error}"))?;
    }
    if !has_service(PROJECT_BROWSER_SERVICE_ID) {
        plugins.shutdown();
        return Err(format!(
            "Project Browser service '{}' unavailable after loading '{}'",
            PROJECT_BROWSER_SERVICE_ID, plugin_id,
        ));
    }

    let startup_config_path =
        env::var(newengine_runtime_host::runtime_config::ENGINE_STARTUP_CONFIG_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "config.json".to_owned());
    let startup_paths = newengine_core::ConfigPaths::from_startup_str(&startup_config_path);
    let mut startup_document =
        newengine_core::StartupLoader::load_startup_document_preview(&startup_paths)
            .map_err(|error| format!("load Project Browser startup config document: {error}"))?;
    // Project Browser edits the hydrated player-settings document, so that exact
    // document must remain authoritative through patch application and persistence.
    // Applying its patch to the pre-hydration raw JSON makes default-backed fields
    // such as startup_settings.display.resolution appear in the UI but fail on save.
    startup_document = prepare_project_browser_config_document(startup_document)?;
    let payload = serde_json::to_vec(&serde_json::json!({
        "root": root.to_string_lossy(),
        "config_document": startup_document.clone(),
    }))
    .map_err(|error| format!("encode project-browser request: {error}"))?;
    let response = call_service_v1(
        PROJECT_BROWSER_SERVICE_ID.into(),
        PROJECT_BROWSER_PRESENT_METHOD_V1.into(),
        Blob::from(payload),
    )
    .into_result()
    .map_err(|error| format!("Project Browser failed: {error}"))?;
    let value: serde_json::Value = serde_json::from_slice(response.as_slice())
        .map_err(|error| format!("decode project-browser response: {error}"))?;
    if !value
        .get("cancelled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let changed_settings = apply_project_browser_settings_patch(&value, &mut startup_document)?;
        if changed_settings > 0 {
            newengine_core::StartupLoader::persist_startup_document(
                &startup_paths,
                &startup_document,
            )
            .map_err(|error| format!("persist Project Browser startup document: {error}"))?;
            let mut selected_settings: newengine_core::StartupLaunchSettings =
                serde_json::from_value(
                    startup_document
                        .get("startup_settings")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                )
                .map_err(|error| {
                    format!("decode confirmed Project Browser startup settings: {error}")
                })?;
            selected_settings.normalize();
            selected_settings.publish_environment_snapshot();
        }
        // Settings were already confirmed in the project-selection window. Do not
        // present a second PreStart settings window for this browser-selected launch.
        env::set_var("NEWENGINE_STARTUP_WINDOW_SKIP", "1");
    }
    let selection = ProjectBrowserSelection {
        manifest_path: value
            .get("manifest_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from),
        launch_id: value
            .get("launch_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        cancelled: value
            .get("cancelled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    };
    let action = match value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("launch")
    {
        "launch" => ProjectBrowserAction::Launch,
        "build_standalone" => ProjectBrowserAction::BuildStandalone,
        other => {
            plugins.shutdown();
            return Err(format!("Project Browser returned unknown action '{other}'"));
        }
    };
    let build_options = parse_project_browser_build_options(&value);
    plugins.shutdown();
    Ok(ProjectBrowserResult {
        selection,
        action,
        build_options,
    })
}

fn parse_project_browser_build_options(
    value: &serde_json::Value,
) -> standalone_package::StandaloneBuildOptions {
    let build = value.get("build");
    standalone_package::StandaloneBuildOptions {
        output_dir: build
            .and_then(|build| build.get("output_dir"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        package_name: build
            .and_then(|build| build.get("package_name"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        target_os: build
            .and_then(|build| build.get("target_os"))
            .and_then(serde_json::Value::as_str)
            .and_then(standalone_package::StandaloneTargetOs::from_id)
            .unwrap_or(standalone_package::StandaloneTargetOs::Windows),
        rebuild_plugins: build
            .and_then(|build| build.get("rebuild_plugins"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        include_source: build
            .and_then(|build| build.get("include_source"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
    }
}

fn launch_runtime_profile_via_plugins(
    runtime_profile: &str,
    project: &newengine_project_runtime::ProjectRuntimeContext,
) -> Result<(), String> {
    init_host_context();
    let host = default_host_api();
    let mut plugins = PluginManager::new();
    let result = (|| -> Result<(), String> {
        // Project-local plugins are loaded first so a project may provide/override its own
        // game-module descriptor. Runtime-profile ownership remains independent.
        load_game_runtime_plugins(&mut plugins, project, host.clone())?;

        // game_module is a descriptor/capability root, not a runtime-profile launcher.
        // The launcher verifies/loads it in the launcher host, but must not exclude it from the
        // runtime host: GameModuleContractModule validates the descriptor through the runtime-local
        // engine.game.module service after EnginePluginsReady.
        if let Some(game_module) = project
            .manifest
            .game_module
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !has_service(newengine_game_module_api::GAME_MODULE_SERVICE_ID) {
                plugins
                    .load_plugin_id_default_with_origin(
                        game_module,
                        host.clone(),
                        PluginLoadOrigin::FirstPartyPlugin,
                    )
                    .map_err(|error| {
                        format!(
                            "load game-module descriptor plugin '{}' from pluginsRuntime: {error}",
                            game_module
                        )
                    })?;
            }
            if !has_service(newengine_game_module_api::GAME_MODULE_SERVICE_ID) {
                return Err(format!(
                    "game_module '{}' loaded without required descriptor service '{}'",
                    game_module,
                    newengine_game_module_api::GAME_MODULE_SERVICE_ID,
                ));
            }
        }

        // Runtime profile is resolved independently from the module descriptor.
        let service_id = runtime_profile_service_id(runtime_profile);
        if !has_service(&service_id) {
            plugins
                .load_plugin_id_default_with_origin(
                    runtime_profile,
                    host.clone(),
                    PluginLoadOrigin::FirstPartyPlugin,
                )
                .map_err(|error| {
                    format!(
                        "load runtime-profile plugin '{}' from pluginsRuntime: {error}",
                        runtime_profile
                    )
                })?;
        }
        if !has_service(&service_id) {
            let available = plugins
                .snapshot()
                .into_iter()
                .flat_map(|plugin| {
                    plugin
                        .capabilities
                        .into_iter()
                        .map(move |capability| format!("{}:{}", plugin.id, capability.id))
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
            "runtime profile service '{}' is unavailable for profile='{}'; loaded capabilities=[{}]",
            service_id, runtime_profile, available,
        ));
        }
        append_env_list_unique("NEWENGINE_PLUGIN_EXCLUDE_IDS", runtime_profile);

        let resolved_project = newengine_project_api::ResolvedProjectContextPayloadV1::new(
            project.manifest_path.clone(),
            project.project_root.clone(),
            project.manifest.clone(),
            project.launch.clone(),
        );
        let payload = serde_json::to_vec(&serde_json::json!({
            "manifest_path": project.manifest_path.to_string_lossy(),
            "game_manifest_path": project.manifest_path.to_string_lossy(),
            "launch_id": project.launch.preset_id,
            "runtime_profile": runtime_profile,
            "game_module": project.manifest.game_module,
            "resolved_project": resolved_project,
        }))
        .map_err(|error| format!("encode runtime-profile launch request: {error}"))?;

        let launch_result = call_service_v1(
            service_id.clone().into(),
            RUNTIME_PROFILE_LAUNCH_METHOD_V1.into(),
            Blob::from(payload),
        )
        .into_result()
        .map_err(|error| {
            format!(
                "runtime profile '{}' launch failed: {error}",
                runtime_profile
            )
        });

        launch_result.map(|_| ())
    })();
    // Always release launcher-owned plugin DLLs/services, including every early-error path.
    // Returning through `?` after loading the game-module descriptor used to skip this and
    // could leave host callbacks pointing into an unloading DLL, producing STATUS_ACCESS_VIOLATION.
    plugins.shutdown();
    result
}

fn load_game_runtime_plugins(
    plugins: &mut PluginManager,
    project: &newengine_project_runtime::ProjectRuntimeContext,
    host: newengine_plugin_api::HostApiV1,
) -> Result<(), String> {
    let conventional = project.paths().conventional_plugins_dir();
    if conventional.is_dir() {
        plugins
            .load_from_dir_with_policy_and_origin(
                &conventional,
                host.clone(),
                false,
                PluginLoadOrigin::GamePlugin,
            )
            .map_err(|error| {
                format!(
                    "load game plugin directory '{}': {error}",
                    conventional.display()
                )
            })?;
    }

    for plugin in &project.manifest.plugins {
        let Some(path) = plugin.path.as_ref() else {
            continue;
        };
        let path = project.resolve_authored_path(path);

        let result = if path.is_dir() {
            plugins.load_from_dir_with_policy_and_origin(
                &path,
                host.clone(),
                plugin.required,
                PluginLoadOrigin::GamePlugin,
            )
        } else {
            plugins.load_path_with_origin(&path, host.clone(), PluginLoadOrigin::GamePlugin)
        };

        if let Err(error) = result {
            if plugin.required {
                return Err(format!(
                    "required game plugin '{}' failed from '{}': {error}",
                    plugin.id,
                    path.display(),
                ));
            }
            eprintln!(
                "NewEngine: optional game plugin '{}' skipped from '{}': {error}",
                plugin.id,
                path.display(),
            );
        }
    }
    Ok(())
}

#[derive(Debug)]
enum LauncherTarget {
    Executable(PathBuf),
    Cargo { workspace: PathBuf, package: String },
}

fn resolve_launcher_target(launcher: &str) -> Result<LauncherTarget, String> {
    if let Some(bin_dir) = env::var_os("NEWENGINE_LAUNCHER_BIN_DIR") {
        if let Some(path) = executable_candidate(&PathBuf::from(bin_dir), launcher) {
            return Ok(LauncherTarget::Executable(path));
        }
    }

    if let Ok(current) = env::current_exe() {
        if let Some(dir) = current.parent() {
            for candidate_dir in [dir.to_path_buf(), dir.join("bin")] {
                if let Some(path) = executable_candidate(&candidate_dir, launcher) {
                    return Ok(LauncherTarget::Executable(path));
                }
            }
        }
    }

    if let Some(workspace) = find_cargo_workspace() {
        return Ok(LauncherTarget::Cargo {
            workspace,
            package: launcher.to_owned(),
        });
    }

    Err(format!(
        "cannot resolve launcher '{launcher}'; install its executable next to NewEngine, set NEWENGINE_LAUNCHER_BIN_DIR, or run from the Cargo workspace"
    ))
}

fn executable_candidate(dir: &Path, launcher: &str) -> Option<PathBuf> {
    let filename = if cfg!(windows) {
        format!("{launcher}.exe")
    } else {
        launcher.to_owned()
    };
    let direct = dir.join(&filename);
    if direct.is_file() {
        return Some(direct);
    }
    let normalized = launcher.replace('_', "-");
    let alternate = if cfg!(windows) {
        dir.join(format!("{normalized}.exe"))
    } else {
        dir.join(normalized)
    };
    alternate.is_file().then_some(alternate)
}

fn find_cargo_workspace() -> Option<PathBuf> {
    let mut seeds = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        seeds.push(cwd);
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            seeds.push(parent.to_path_buf());
        }
    }
    for seed in seeds {
        for ancestor in seed.ancestors().take(8) {
            if ancestor.join("Cargo.toml").is_file()
                && ancestor.join("crates").is_dir()
                && ancestor.join("apps").is_dir()
            {
                return Some(ancestor.to_path_buf());
            }
        }
    }
    None
}

fn exclude_project_browser_from_runtime() {
    let browser_plugin_id = env::var(PROJECT_BROWSER_PLUGIN_ID_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_PROJECT_BROWSER_PLUGIN_ID.to_owned());
    append_env_list_unique("NEWENGINE_PLUGIN_EXCLUDE_IDS", &browser_plugin_id);
}

fn append_env_list_unique(name: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let mut entries = env::var(name)
        .ok()
        .into_iter()
        .flat_map(|current| {
            current
                .split(|ch: char| ch == ',' || ch == ';' || ch.is_ascii_whitespace())
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if !entries.iter().any(|entry| entry == value) {
        entries.push(value.to_owned());
    }
    env::set_var(name, entries.join(","));
}

fn install_process_root_dir_authority(runtime_config_path: &Path) -> Result<PathBuf, String> {
    if let Some(explicit) = env::var_os(ROOT_DIR_ENV).filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        if !path.is_absolute() {
            return Err(format!(
                "{ROOT_DIR_ENV} must be absolute, got '{}'",
                path.display()
            ));
        }
        let root = std::fs::canonicalize(&path).unwrap_or(path);
        env::set_var(ROOT_DIR_ENV, &root);
        return Ok(root);
    }

    let mut seeds = Vec::<PathBuf>::new();
    seeds.push(runtime_config_path.to_path_buf());
    if let Ok(cwd) = env::current_dir() {
        seeds.push(cwd);
    }
    if let Ok(exe) = env::current_exe() {
        seeds.push(exe);
    }

    for seed in seeds {
        let start = if seed.is_file() {
            seed.parent().map(Path::to_path_buf).unwrap_or(seed)
        } else {
            seed
        };
        for ancestor in start.ancestors() {
            if ancestor.join("NewEngine").is_dir()
                && ancestor.join("pluginsRuntime").is_dir()
                && ancestor.join("Projects").is_dir()
            {
                let root =
                    std::fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
                env::set_var(ROOT_DIR_ENV, &root);
                return Ok(root);
            }
        }
    }

    Err(format!(
        "{ROOT_DIR_ENV} is not set and NorthStar root auto-detection failed from runtime config '{}'",
        runtime_config_path.display()
    ))
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}
