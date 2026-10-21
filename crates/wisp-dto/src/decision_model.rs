//! Settings → Models → Decision model: the one System One endpoint behind
//! `map_items`' `decide` step and `desktop_autopilot`.
use serde::{Deserialize, Serialize};

/// A ready-made endpoint. The form fills these defaults when the preset is
/// picked; the host uses `id` to choose the keyring entry, so switching
/// presets keeps each service's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionPreset {
    pub id: &'static str,
    pub base_url: &'static str,
    /// Empty lets the server pick its default model.
    pub model: &'static str,
    pub timeout_secs: u64,
    /// Hosted services reject keyless requests; a self-hosted server only
    /// needs a key when it was started with one.
    pub key_required: bool,
    /// Where to create a key; empty when there is nowhere to go.
    pub key_url: &'static str,
}

pub const DECISION_PRESETS: &[DecisionPreset] = &[
    DecisionPreset {
        id: "typesafe",
        base_url: "https://api.typesafe.ai",
        model: "jev-latest",
        timeout_secs: 10,
        key_required: true,
        key_url: "https://console.typesafe.ai",
    },
    DecisionPreset {
        id: "startlux_bamboo",
        base_url: "https://api.bamboonode.dev",
        model: "startlux-decision-27b-autothink",
        timeout_secs: 60,
        key_required: true,
        key_url: "https://bamboonode.dev",
    },
    DecisionPreset {
        id: "startlux_local",
        base_url: "http://127.0.0.1:8090",
        model: "",
        timeout_secs: 60,
        key_required: false,
        key_url: "https://github.com/StartLuxLabs/StartLux-Decision",
    },
    DecisionPreset {
        id: "custom",
        base_url: "",
        model: "",
        timeout_secs: 30,
        key_required: false,
        key_url: "",
    },
];

/// `think_mode` values; empty sends nothing and keeps the server default.
pub const DECISION_THINK_MODES: &[&str] = &["", "auto", "never", "always"];

impl DecisionPreset {
    pub fn find(id: &str) -> Option<&'static DecisionPreset> {
        DECISION_PRESETS.iter().find(|preset| preset.id == id)
    }
}

/// The saved decision model. The API key never crosses this boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionModelSettings {
    pub enabled: bool,
    pub preset: String,
    pub base_url: String,
    pub model: String,
    pub think_mode: String,
    pub timeout_secs: u64,
    /// Read-only: presets whose key is in the keyring. Ignored on save.
    pub stored_keys: Vec<String>,
}

impl DecisionModelSettings {
    /// A preset's defaults, disabled, with nothing stored.
    pub fn from_preset(preset: &DecisionPreset) -> Self {
        Self {
            enabled: false,
            preset: preset.id.into(),
            base_url: preset.base_url.into(),
            model: preset.model.into(),
            think_mode: String::new(),
            timeout_secs: preset.timeout_secs,
            stored_keys: Vec::new(),
        }
    }

    pub fn has_key(&self, preset: &str) -> bool {
        self.stored_keys.iter().any(|id| id == preset)
    }
}

/// One answered test question from the "Test connection" button.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionModelProbe {
    /// The model that answered, as the server names it.
    pub model: String,
    pub elapsed_ms: u64,
}
