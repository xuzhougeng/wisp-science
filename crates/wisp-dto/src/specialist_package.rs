//! Portable Specialist package (`*.wisp-specialist.json`) for sharing a persona
//! between people and machines.
//!
//! The package carries only what is meaningful on another machine: the
//! persona text plus skill/connector whitelists and an optional model hint.
//! Machine-local ids (`id`, the bound model profile id, the Reviewer backend
//! profile) and the `builtin` flag are never exported; parsing rejects them
//! through `deny_unknown_fields`, so a crafted package cannot target a
//! built-in row or bind a local model profile by id.
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "wisp-specialist";
pub const VERSION: u32 = 1;
/// Suggested file suffix; importing does not depend on it.
pub const FILE_SUFFIX: &str = ".wisp-specialist.json";
/// Packages are hand-inspectable text. Anything larger is rejected before parsing.
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpecialistPackage {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    /// `null` = inherit the project skill config; `[]` = no skills.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// `null` = inherit; `[]` = no connectors. Entries are connector slugs,
    /// MCP connection ids or `plugin:{id}:{server}` and may not exist locally.
    #[serde(default)]
    pub connectors: Option<Vec<String>>,
    /// The exporter's bound model, by provider and model name only. Never an
    /// endpoint, profile id or credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_hint: Option<SpecialistModelHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpecialistModelHint {
    pub provider: String,
    pub model: String,
}

/// A local chat model profile whose model name matches the package hint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialistModelMatch {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub model: String,
    /// The provider matches too, not just the model name.
    pub same_provider: bool,
}

/// An existing specialist whose name collides (case-insensitively) with the package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialistNameConflict {
    pub id: String,
    pub name: String,
    pub builtin: bool,
}

/// Everything the user reviews before confirming an import. Nothing has been
/// written when this is returned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialistImportPreview {
    pub package: SpecialistPackage,
    pub file_name: String,
    pub bytes: u64,
    pub name_conflict: Option<SpecialistNameConflict>,
    pub model_matches: Vec<SpecialistModelMatch>,
    pub missing_skills: Vec<String>,
    pub missing_connectors: Vec<String>,
    /// False when no project is open in this window, so skill availability
    /// could not be checked; `missing_skills` is then empty.
    pub skills_checked: bool,
}

/// One instructions line that looks like it carries a credential.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretFinding {
    /// `private_key`, `token_prefix`, `bearer` or `assignment`.
    pub kind: String,
    /// 1-based line in the instructions.
    pub line: u32,
    /// A masked excerpt (first characters only) that locates the match
    /// without echoing the full value.
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SpecialistExportOutcome {
    /// Nothing was written; the caller must confirm before exporting again.
    SecretWarning {
        findings: Vec<SecretFinding>,
    },
    Cancelled,
    Exported {
        path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_keeps_inherit_and_allow_none_distinct() {
        let package: SpecialistPackage = serde_json::from_str(
            r#"{"format":"wisp-specialist","version":1,"name":"A","skills":null,"connectors":[]}"#,
        )
        .unwrap();
        assert_eq!(package.skills, None);
        assert_eq!(package.connectors, Some(vec![]));
        let json = serde_json::to_value(&package).unwrap();
        assert!(json["skills"].is_null());
        assert_eq!(json["connectors"], serde_json::json!([]));
        assert!(json.get("model_hint").is_none());
    }

    #[test]
    fn package_rejects_machine_local_fields() {
        for field in [
            r#""id":"reviewer""#,
            r#""builtin":true"#,
            r#""model_id":"m1""#,
            r#""review_backend":{"kind":"follow_session"}"#,
            r#""api_key":"sk-x""#,
        ] {
            let raw = format!(r#"{{"format":"wisp-specialist","version":1,"name":"A",{field}}}"#);
            assert!(
                serde_json::from_str::<SpecialistPackage>(&raw).is_err(),
                "{field} must be rejected"
            );
        }
        assert!(serde_json::from_str::<SpecialistPackage>(
            r#"{"format":"wisp-specialist","version":1,"name":"A","model_hint":{"provider":"openai","model":"gpt","api_url":"https://x"}}"#,
        )
        .is_err());
    }

    #[test]
    fn export_outcome_is_tagged_by_status() {
        let value = serde_json::to_value(SpecialistExportOutcome::Exported {
            path: "/tmp/a.wisp-specialist.json".into(),
        })
        .unwrap();
        assert_eq!(value["status"], "exported");
        let cancelled: SpecialistExportOutcome =
            serde_json::from_str(r#"{"status":"cancelled"}"#).unwrap();
        assert_eq!(cancelled, SpecialistExportOutcome::Cancelled);
    }
}
