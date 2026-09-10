use serde::{Deserialize, Serialize};

use crate::validation::ensure_optional_non_blank;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLaunchProfile {
    #[default]
    Game,
    Server,
    Test,
}

impl RuntimeLaunchProfile {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Server => "server",
            Self::Test => "test",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "game" | "play" => Some(Self::Game),
            "server" | "dedicated" | "headless" => Some(Self::Server),
            "test" | "smoke" => Some(Self::Test),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectLaunchPreset {
    /// Runtime mode selected by this preset. When omitted, the project-level
    /// `launch_profile` remains the default.
    pub profile: Option<RuntimeLaunchProfile>,
    /// Optional runtime composition/profile override. This is resolved through the
    /// runtime-profile registry instead of a launcher-side hardcoded branch.
    pub runtime_profile: Option<String>,
    /// Optional startup scene override for this launch preset.
    pub startup_scene: Option<String>,
    /// Optional authored UI presentation state override for this launch preset.
    pub startup_presentation_state: Option<String>,
}

impl ProjectLaunchPreset {
    pub fn validate(&self, id: &str) -> Result<(), String> {
        for (field, value) in [
            ("runtime_profile", self.runtime_profile.as_deref()),
            ("startup_scene", self.startup_scene.as_deref()),
            (
                "startup_presentation_state",
                self.startup_presentation_state.as_deref(),
            ),
        ] {
            ensure_optional_non_blank(value, || {
                format!("launch preset '{id}' field '{field}' must be non-empty when specified")
            })?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjectLaunch {
    pub preset_id: String,
    pub profile: RuntimeLaunchProfile,
    pub runtime_profile: Option<String>,
    pub startup_scene: Option<String>,
    pub startup_presentation_state: Option<String>,
}
