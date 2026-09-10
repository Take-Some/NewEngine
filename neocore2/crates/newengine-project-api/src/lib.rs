#![forbid(unsafe_op_in_unsafe_fn)]

mod filesystem;
mod launch;
mod manifest;
mod mounts;
mod scripting;
mod service;
mod validation;

pub use filesystem::*;
pub use launch::*;
pub use manifest::*;
pub use mounts::*;
pub use scripting::*;
pub use service::*;

pub const PROJECT_MANIFEST_FILE: &str = "game.toml";
/// Compatibility request accepting either a project directory or manifest path.
pub const PROJECT_REQUEST_ENV: &str = "NEWENGINE_PROJECT";
/// Canonical repository/deployment root used by all authored filesystem paths.
pub const ROOT_DIR_ENV: &str = "ROOT-DIR";
/// Canonical active project directory. Published after `game.toml` is resolved.
pub const PROJECT_DIR_ENV: &str = "PROJECT-DIR";
/// Canonical selected editor-project root.
pub const PROJECT_ROOT_ENV: &str = "NEWENGINE_PROJECT_ROOT";
/// Canonical selected editor-project manifest.
pub const PROJECT_MANIFEST_ENV: &str = "NEWENGINE_PROJECT_MANIFEST";
/// Canonical selected game root.
pub const GAME_ROOT_ENV: &str = "NEWENGINE_GAME_ROOT";
/// Canonical selected game manifest.
pub const GAME_MANIFEST_ENV: &str = "NEWENGINE_GAME_MANIFEST";
pub const PROJECT_MANIFEST_CONTRACT: &str = "newengine.project.v1";
pub const PROJECT_STARTUP_SCENE_ENV: &str = "NEWENGINE_PROJECT_STARTUP_SCENE";
pub const PROJECT_LAUNCH_PRESET_ENV: &str = "NEWENGINE_PROJECT_LAUNCH_PRESET";
pub const PROJECT_RUNTIME_PROFILE_ABI_V1: &str = "newengine.runtime-profile/v1";
pub const RESOLVED_PROJECT_CONTEXT_PAYLOAD_SCHEMA_V1: u32 = 1;

/// Fully resolved project handoff carried across the runtime-profile ABI.
/// The launcher parses and validates `game.toml` once; downstream runtime hosts
/// reconstruct their in-memory registries from this immutable payload instead of
/// reopening and re-resolving the authored manifest during the same process launch.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedProjectContextPayloadV1 {
    pub schema_version: u32,
    pub manifest_path: std::path::PathBuf,
    pub project_root: std::path::PathBuf,
    pub manifest: ProjectManifest,
    pub launch: ResolvedProjectLaunch,
}

impl ResolvedProjectContextPayloadV1 {
    pub fn new(
        manifest_path: std::path::PathBuf,
        project_root: std::path::PathBuf,
        manifest: ProjectManifest,
        launch: ResolvedProjectLaunch,
    ) -> Self {
        Self {
            schema_version: RESOLVED_PROJECT_CONTEXT_PAYLOAD_SCHEMA_V1,
            manifest_path,
            project_root,
            manifest,
            launch,
        }
    }
}
pub const PROJECT_MANIFEST_CONTRACT_SPEC: newengine_contract_api::ContractSpec =
    newengine_contract_api::ContractSpec::new(
        "project.manifest",
        newengine_contract_api::ContractKind::Manifest,
        newengine_contract_api::ContractVersion::major(1),
        newengine_contract_api::ContractCompatibility::Exact,
        "newengine-project-api",
        Some(PROJECT_MANIFEST_CONTRACT),
    );
pub const PROJECT_RUNTIME_PROFILE_ABI_CONTRACT_SPEC: newengine_contract_api::ContractSpec =
    newengine_contract_api::ContractSpec::new(
        "runtime.profile.abi",
        newengine_contract_api::ContractKind::Abi,
        newengine_contract_api::ContractVersion::major(1),
        newengine_contract_api::ContractCompatibility::Exact,
        "newengine-project-api",
        Some(PROJECT_RUNTIME_PROFILE_ABI_V1),
    );
pub const RUNTIME_PROFILE_LAUNCH_METHOD_V1: &str = "runtime.launch_v1";
pub const RUNTIME_PROFILE_SERVICE_PREFIX: &str = "engine.runtime-profile.";
pub const PROJECT_BROWSER_SERVICE_ID: &str = "engine.project-browser";
pub const PROJECT_BROWSER_PRESENT_METHOD_V1: &str = "project.present_v1";

#[inline]
pub fn runtime_profile_service_id(profile_id: &str) -> String {
    format!("{RUNTIME_PROFILE_SERVICE_PREFIX}{}", profile_id.trim())
}

/// Resolves the ABI service used to launch a concrete runtime composition.
/// A project game module gets its own service identity so generic GameReady,
/// FPS, top-down and third-person compositions can coexist in one plugin directory.
#[inline]
pub fn runtime_profile_service_id_for_game(profile_id: &str, game_module: Option<&str>) -> String {
    let base = runtime_profile_service_id(profile_id);
    match game_module.map(str::trim).filter(|value| !value.is_empty()) {
        Some(module_id) => format!("{base}.game-module.{module_id}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, path::PathBuf};

    #[test]
    fn project_manifest_accepts_direct_gameplay_presentation_state() {
        let manifest = ProjectManifest {
            id: "direct-game".to_owned(),
            name: "Direct Game".to_owned(),
            launch_profile: Some(RuntimeLaunchProfile::Game),
            startup_presentation_state: Some("gameplay".to_owned()),
            ..ProjectManifest::default()
        };
        assert_eq!(manifest.launch_profile, Some(RuntimeLaunchProfile::Game));
        assert_eq!(
            manifest.startup_presentation_state.as_deref(),
            Some("gameplay")
        );
        manifest
            .validate()
            .expect("valid direct-game project manifest");
    }

    #[test]
    fn script_registry_accepts_arbitrary_module_ids_and_bindings() {
        let manifest = ProjectScriptingManifest {
            runtime: Some("lua".to_owned()),
            entrypoint: Some("boot".to_owned()),
            modules: BTreeMap::from([
                ("boot".to_owned(), "scripts:/game.ysc".to_owned()),
                (
                    "my_weird_data".to_owned(),
                    "scripts:/custom/data.ysc".to_owned(),
                ),
            ]),
            bindings: BTreeMap::from([(
                "consumer.anything".to_owned(),
                ProjectScriptBinding {
                    module: "my_weird_data".to_owned(),
                    operation: Some("produce_whatever".to_owned()),
                },
            )]),
        };
        let registry = ProjectScriptRegistry::from_manifest(&manifest).unwrap();
        assert_eq!(registry.entrypoint().as_deref(), Some("scripts/game.ysc"));
        let binding = registry.binding("consumer.anything").unwrap();
        assert_eq!(binding.script_ref, "scripts/custom/data.ysc");
        assert_eq!(binding.operation.as_deref(), Some("produce_whatever"));
    }

    #[test]
    fn launch_presets_overlay_project_defaults() {
        let manifest = ProjectManifest {
            id: "launchable".to_owned(),
            runtime_profile: Some("runtime.default".to_owned()),
            startup_scene: Some("maps/default.ymap".to_owned()),
            launch_profile: Some(RuntimeLaunchProfile::Game),
            launch: BTreeMap::from([(
                "editor".to_owned(),
                ProjectLaunchPreset {
                    profile: Some(RuntimeLaunchProfile::Game),
                    runtime_profile: None,
                    startup_scene: Some("maps/edit.ymap".to_owned()),
                    startup_presentation_state: None,
                },
            )]),
            ..ProjectManifest::default()
        };
        let resolved = manifest.resolve_launch(Some("editor")).unwrap();
        assert_eq!(resolved.profile, RuntimeLaunchProfile::Game);
        assert_eq!(resolved.runtime_profile.as_deref(), Some("runtime.default"));
        assert_eq!(resolved.startup_scene.as_deref(), Some("maps/edit.ymap"));
    }

    #[test]
    fn standard_launch_modes_override_legacy_manifest_without_authored_presets() {
        let manifest = ProjectManifest {
            id: "legacy".to_owned(),
            launch_profile: Some(RuntimeLaunchProfile::Game),
            ..ProjectManifest::default()
        };
        let resolved = manifest.resolve_launch(Some("server")).unwrap();
        assert_eq!(resolved.profile, RuntimeLaunchProfile::Server);
        assert_eq!(resolved.preset_id, "server");
    }

    #[test]
    fn project_owned_presentation_flow_accepts_project_documents() {
        let manifest = ProjectManifest {
            id: "frontend-game".to_owned(),
            name: "Frontend Game".to_owned(),
            ui: ProjectUiManifest {
                presentation_flow: Some(ProjectUiPresentationFlowManifest {
                    id: "frontend".to_owned(),
                    initial_state: "menu".to_owned(),
                    states: vec![
                        ProjectUiPresentationStateManifest {
                            id: "menu".to_owned(),
                            document_ref: Some("ui/frontend/main_menu.neui@surface".to_owned()),
                            surface_id: Some("game.frontend.main_menu".to_owned()),
                            blocks_world_bootstrap: true,
                            blocks_gameplay_input: true,
                            ..ProjectUiPresentationStateManifest::default()
                        },
                        ProjectUiPresentationStateManifest {
                            id: "gameplay".to_owned(),
                            input_focus_policy: ProjectUiInputFocusPolicy::GameViewport,
                            ..ProjectUiPresentationStateManifest::default()
                        },
                    ],
                    transitions: vec![ProjectUiPresentationTransitionManifest {
                        from: "menu".to_owned(),
                        to: "gameplay".to_owned(),
                        on_action: Some("game.start".to_owned()),
                        ..ProjectUiPresentationTransitionManifest::default()
                    }],
                    ..ProjectUiPresentationFlowManifest::default()
                }),
                ..ProjectUiManifest::default()
            },
            ..ProjectManifest::default()
        };
        manifest
            .validate()
            .expect("project-owned frontend is valid");
    }

    #[test]
    fn project_owned_presentation_flow_rejects_engine_menu_assets() {
        let manifest = ProjectManifest {
            id: "bad-frontend".to_owned(),
            name: "Bad Frontend".to_owned(),
            ui: ProjectUiManifest {
                presentation_flow: Some(ProjectUiPresentationFlowManifest {
                    id: "frontend".to_owned(),
                    initial_state: "menu".to_owned(),
                    states: vec![ProjectUiPresentationStateManifest {
                        id: "menu".to_owned(),
                        document_ref: Some("ui/engine/main_menu.neui@surface".to_owned()),
                        surface_id: Some("game.frontend.main_menu".to_owned()),
                        blocks_world_bootstrap: true,
                        blocks_gameplay_input: true,
                        ..ProjectUiPresentationStateManifest::default()
                    }],
                    ..ProjectUiPresentationFlowManifest::default()
                }),
                ..ProjectUiManifest::default()
            },
            ..ProjectManifest::default()
        };
        let errors = manifest
            .validate()
            .expect_err("engine-owned menu must be rejected");
        assert!(errors.iter().any(|error| error.contains("engine-owned UI")));
    }

    #[test]
    fn registry_resolves_namespace_paths_by_priority_order() {
        let mut registry = ContentMountRegistry::default();
        registry
            .register(ContentMountDescriptor {
                id: "game".to_owned(),
                root: PathBuf::from("C:/project/content"),
                ..ContentMountDescriptor::default()
            })
            .unwrap();
        assert_eq!(
            registry.resolve_logical("game:/models/a.nef8"),
            Some(
                PathBuf::from("C:/project/content")
                    .join("models")
                    .join("a.nef8")
            )
        );
        assert_eq!(
            registry.logical_for_physical(
                &PathBuf::from("C:/project/content")
                    .join("models")
                    .join("a.nef8")
            ),
            Some("game:/models/a.nef8".to_owned())
        );
        assert_eq!(
            registry.asset_ref_for_physical(
                &PathBuf::from("C:/project/content")
                    .join("models")
                    .join("a.nef8")
            ),
            Some("game/models/a.nef8".to_owned())
        );
        assert_eq!(
            registry.resolve_asset_ref("game/models/a.nef8"),
            Some(
                PathBuf::from("C:/project/content")
                    .join("models")
                    .join("a.nef8")
            )
        );
    }
}
