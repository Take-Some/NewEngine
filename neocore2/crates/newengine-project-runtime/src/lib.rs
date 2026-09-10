mod content_mounts;
mod launch_env;
mod project_browser;
mod project_paths;
mod runtime_profiles;
mod shared_ui;

pub use shared_ui::{
    effective_project_ui_presentation_flow, SHARED_UI_HUD_DOCUMENT_REF, SHARED_UI_HUD_SURFACE_ID,
    SHARED_UI_PAUSE_DOCUMENT_REF, SHARED_UI_PAUSE_SURFACE_ID, SHARED_UI_PRIMARY_TOGGLE_ACTION,
    SHARED_UI_RESUME_ACTION,
};

pub use content_mounts::{
    mount_content_registry_best_effort, register_engine_bootstrap_asset_roots,
};
pub use launch_env::*;
pub use project_browser::{
    default_projects_root, discover_game_projects, discover_projects, preferred_game_launch_id,
    preferred_launch_id, ProjectBrowserEntry, ProjectBrowserSelection,
};
pub use project_paths::*;
pub use runtime_profiles::{RuntimeProfileRegistration, RuntimeProfileRegistry};

use std::path::{Path, PathBuf};

use newengine_project_api::{
    resolve_project_manifest_request, ContentMountRegistry, ProjectManifest, ProjectScriptRegistry,
    ResolvedProjectLaunch, RuntimeLaunchProfile, GAME_MANIFEST_ENV, GAME_ROOT_ENV,
    PROJECT_LAUNCH_PRESET_ENV, PROJECT_MANIFEST_ENV, PROJECT_MANIFEST_FILE, PROJECT_REQUEST_ENV,
    PROJECT_ROOT_ENV,
};

#[derive(Clone, Debug)]
pub struct ProjectRuntimeContext {
    pub manifest_path: PathBuf,
    pub project_root: PathBuf,
    pub manifest: ProjectManifest,
    pub launch: ResolvedProjectLaunch,
    pub mounts: ContentMountRegistry,
    pub scripts: ProjectScriptRegistry,
}

impl ProjectRuntimeContext {
    #[inline]
    pub fn paths(&self) -> ProjectPaths {
        ProjectPaths::new(self.project_root.clone())
    }

    #[inline]
    pub fn resolve_authored_path(&self, path: &Path) -> PathBuf {
        self.paths().resolve_authored_path(path)
    }
}

/// Reconstruct the runtime-only registries from a launcher-authoritative resolved
/// project payload without reopening `game.toml`. This is the in-process half of
/// the runtime-profile launch handoff.
pub fn project_context_from_resolved_payload(
    payload: newengine_project_api::ResolvedProjectContextPayloadV1,
) -> Result<ProjectRuntimeContext, String> {
    if payload.schema_version
        != newengine_project_api::RESOLVED_PROJECT_CONTEXT_PAYLOAD_SCHEMA_V1
    {
        return Err(format!(
            "unsupported resolved project payload schema={} expected={}",
            payload.schema_version,
            newengine_project_api::RESOLVED_PROJECT_CONTEXT_PAYLOAD_SCHEMA_V1
        ));
    }
    if let Err(errors) = payload.manifest.validate() {
        return Err(format!(
            "resolved project manifest invalid: {}",
            errors.join("; ")
        ));
    }
    let scripts = ProjectScriptRegistry::from_manifest(&payload.manifest.scripting)
        .map_err(|error| format!("resolved project scripting invalid: {error}"))?;
    let mut mounts = ContentMountRegistry::default();
    for descriptor in &payload.manifest.content {
        mounts.register(descriptor.clone().normalized(&payload.project_root))?;
    }
    Ok(ProjectRuntimeContext {
        manifest_path: payload.manifest_path,
        project_root: payload.project_root,
        manifest: payload.manifest,
        launch: payload.launch,
        mounts,
        scripts,
    })
}

#[derive(Clone, Debug)]
pub struct RuntimeCompositionContext {
    pub manifest_path: PathBuf,
    pub runtime_root: PathBuf,
    pub runtime_profile: String,
    pub game_module: Option<String>,
    pub launch_profile: RuntimeLaunchProfile,
    pub startup_scene: Option<String>,
    pub startup_presentation_state: Option<String>,
    pub definitions: Vec<PathBuf>,
    pub mounts: ContentMountRegistry,
    pub scripts: ProjectScriptRegistry,
}

impl RuntimeCompositionContext {
    pub fn from_project(project: &ProjectRuntimeContext) -> Self {
        Self {
            manifest_path: project.manifest_path.clone(),
            runtime_root: project.project_root.clone(),
            runtime_profile: project
                .launch
                .runtime_profile
                .clone()
                .or_else(|| project.manifest.runtime_profile.clone())
                .unwrap_or_default(),
            game_module: project.manifest.game_module.clone(),
            launch_profile: project.launch.profile,
            startup_scene: project.launch.startup_scene.clone(),
            startup_presentation_state: project.launch.startup_presentation_state.clone(),
            definitions: project.manifest.definitions.clone(),
            mounts: project.mounts.clone(),
            scripts: project.scripts.clone(),
        }
    }
}

fn path_request_from_args(args: &[std::ffi::OsString], flags: &[&str]) -> Option<PathBuf> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if flags
            .iter()
            .any(|flag| arg.as_os_str() == std::ffi::OsStr::new(*flag))
        {
            return args
                .next()
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value.as_os_str()));
        }
        let Some(text) = arg.to_str() else {
            continue;
        };
        for flag in flags {
            let prefix = format!("{flag}=");
            if let Some(value) = text.strip_prefix(&prefix) {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(PathBuf::from(value));
                }
            }
        }
    }
    None
}

#[inline]
fn environment_path(
    environment_var_os: &mut impl FnMut(&str) -> Option<std::ffi::OsString>,
    key: &str,
) -> Option<PathBuf> {
    environment_var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[inline]
fn manifest_request_from_root(root: PathBuf) -> PathBuf {
    ProjectPaths::new(root).manifest_path()
}

fn project_request_from_sources(
    args: &[std::ffi::OsString],
    environment_var_os: &mut impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    path_request_from_args(args, &["--project"])
        .or_else(|| environment_path(environment_var_os, PROJECT_MANIFEST_ENV))
        .or_else(|| {
            environment_path(environment_var_os, PROJECT_ROOT_ENV).map(manifest_request_from_root)
        })
        .or_else(|| environment_path(environment_var_os, PROJECT_REQUEST_ENV))
}

fn game_manifest_request_from_sources(
    args: &[std::ffi::OsString],
    environment_var_os: &mut impl FnMut(&str) -> Option<std::ffi::OsString>,
    adjacent_manifest: Option<PathBuf>,
) -> Option<PathBuf> {
    path_request_from_args(args, &["--game-manifest"])
        .or_else(|| environment_path(environment_var_os, GAME_MANIFEST_ENV))
        .or_else(|| {
            environment_path(environment_var_os, GAME_ROOT_ENV).map(manifest_request_from_root)
        })
        .or_else(|| project_request_from_sources(args, environment_var_os))
        .or(adjacent_manifest)
}

pub fn game_manifest_request_from_environment(
    mut environment_var_os: impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    game_manifest_request_from_sources(
        &args,
        &mut environment_var_os,
        adjacent_game_manifest_from_exe(),
    )
}

pub fn game_manifest_request_from_process() -> Option<PathBuf> {
    game_manifest_request_from_environment(|name| std::env::var_os(name))
}

pub fn adjacent_game_manifest_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(PROJECT_MANIFEST_FILE);
    candidate.is_file().then_some(candidate)
}

pub fn project_request_from_cli() -> Option<PathBuf> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    path_request_from_args(&args, &["--project"])
}

pub fn project_request_from_environment(
    mut environment_var_os: impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    project_request_from_sources(&args, &mut environment_var_os)
}

pub fn project_request_from_process() -> Option<PathBuf> {
    project_request_from_environment(|name| std::env::var_os(name))
}

pub fn project_launch_request_from_environment(
    mut environment_var: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--launch" || arg == "--launch-profile" {
            return args
                .next()
                .and_then(|value| value.to_str().map(str::to_owned))
                .filter(|value| !value.trim().is_empty());
        }
        if let Some(text) = arg.to_str() {
            for prefix in ["--launch=", "--launch-profile="] {
                if let Some(value) = text.strip_prefix(prefix) {
                    if !value.trim().is_empty() {
                        return Some(value.trim().to_owned());
                    }
                }
            }
        }
    }
    environment_var(PROJECT_LAUNCH_PRESET_ENV)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub fn project_launch_request_from_process() -> Option<String> {
    project_launch_request_from_environment(|name| std::env::var(name).ok())
}

pub fn load_project_from_request(request: &Path) -> Result<ProjectRuntimeContext, String> {
    load_project_from_request_with_launch(request, None)
}

pub fn load_project_from_request_with_launch(
    request: &Path,
    launch_request: Option<&str>,
) -> Result<ProjectRuntimeContext, String> {
    let working_dir = std::env::current_dir()
        .map_err(|error| format!("resolve project request base directory: {error}"))?;
    let manifest_path = resolve_project_manifest_request(request, &working_dir);
    let project_paths = ProjectPaths::from_manifest_path(&manifest_path);
    let project_root = project_paths.root().to_path_buf();
    let source = std::fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "read project manifest '{}': {error}",
            manifest_path.display()
        )
    })?;
    let manifest: ProjectManifest = toml::from_str(&source).map_err(|error| {
        format!(
            "parse project manifest '{}': {error}",
            manifest_path.display()
        )
    })?;
    if let Err(errors) = manifest.validate() {
        return Err(format!("project manifest invalid: {}", errors.join("; ")));
    }
    let launch = manifest
        .resolve_launch(launch_request)
        .map_err(|error| format!("project launch invalid: {error}"))?;
    let scripts = ProjectScriptRegistry::from_manifest(&manifest.scripting)
        .map_err(|error| format!("project scripting invalid: {error}"))?;
    let mut mounts = ContentMountRegistry::default();
    for descriptor in &manifest.content {
        mounts.register(descriptor.clone().normalized(&project_root))?;
    }
    Ok(ProjectRuntimeContext {
        manifest_path,
        project_root,
        manifest,
        launch,
        mounts,
        scripts,
    })
}

#[cfg(test)]
mod project_request_resolution_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn os_args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values
            .iter()
            .map(|value| std::ffi::OsString::from(*value))
            .collect()
    }

    fn environment(
        values: BTreeMap<&'static str, std::ffi::OsString>,
    ) -> impl FnMut(&str) -> Option<std::ffi::OsString> {
        move |name| values.get(name).cloned()
    }

    #[test]
    fn explicit_game_manifest_precedes_all_environment_fallbacks() {
        let args = os_args(&["--game-manifest", "cli/game.toml"]);
        let mut env = environment(BTreeMap::from([
            (GAME_MANIFEST_ENV, std::ffi::OsString::from("env/game.toml")),
            (PROJECT_REQUEST_ENV, std::ffi::OsString::from("legacy")),
        ]));
        assert_eq!(
            game_manifest_request_from_sources(
                &args,
                &mut env,
                Some(PathBuf::from("adjacent/game.toml")),
            ),
            Some(PathBuf::from("cli/game.toml"))
        );
    }

    #[test]
    fn canonical_root_environment_resolves_to_manifest() {
        let args = Vec::new();
        let game_root = PathBuf::from("games").join("sample");
        let mut env = environment(BTreeMap::from([(
            GAME_ROOT_ENV,
            game_root.clone().into_os_string(),
        )]));
        assert_eq!(
            game_manifest_request_from_sources(&args, &mut env, None),
            Some(ProjectPaths::new(game_root).manifest_path())
        );
    }

    #[test]
    fn editor_manifest_environment_precedes_legacy_request() {
        let args = Vec::new();
        let mut env = environment(BTreeMap::from([
            (
                PROJECT_MANIFEST_ENV,
                std::ffi::OsString::from("editor/game.toml"),
            ),
            (
                PROJECT_REQUEST_ENV,
                std::ffi::OsString::from("legacy-project"),
            ),
        ]));
        assert_eq!(
            project_request_from_sources(&args, &mut env),
            Some(PathBuf::from("editor/game.toml"))
        );
    }

    #[test]
    fn equals_form_project_cli_is_shared_by_all_launchers() {
        let args = os_args(&["--project=projects/sample"]);
        assert_eq!(
            path_request_from_args(&args, &["--project"]),
            Some(PathBuf::from("projects/sample"))
        );
    }

    #[test]
    fn resolved_payload_reconstructs_project_without_manifest_io() {
        let manifest = ProjectManifest {
            id: "resolved-only".to_owned(),
            name: "Resolved Only".to_owned(),
            runtime_profile: Some("newengine.runtime-profile.game-ready".to_owned()),
            ..ProjectManifest::default()
        };
        let launch = manifest.resolve_launch(Some("game")).unwrap();
        let payload = newengine_project_api::ResolvedProjectContextPayloadV1::new(
            PathBuf::from("Z:/definitely-not-present/game.toml"),
            PathBuf::from("Z:/definitely-not-present"),
            manifest,
            launch,
        );
        let context = project_context_from_resolved_payload(payload).unwrap();
        assert_eq!(context.manifest.id, "resolved-only");
        assert_eq!(context.launch.preset_id, "game");
        assert_eq!(
            context.manifest_path,
            PathBuf::from("Z:/definitely-not-present/game.toml")
        );
    }
}
