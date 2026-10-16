//! Specialist package exchange (`*.wisp-specialist.json`): export one custom
//! specialist to an inspectable file, preview a package, and import it as a
//! new custom specialist.
//!
//! Only portable fields cross machines. The bound model travels as a
//! provider/model hint that the importer explicitly maps to a local profile;
//! profile ids, the Reviewer backend and credentials never leave this device.

use crate::specialists::{self, Specialist};
use regex::Regex;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use tauri::{AppHandle, State};
use wisp_dto::specialist_package::{
    SecretFinding, SpecialistExportOutcome, SpecialistImportPreview, SpecialistModelHint,
    SpecialistModelMatch, SpecialistNameConflict, SpecialistPackage, FILE_SUFFIX, FORMAT,
    MAX_PACKAGE_BYTES, VERSION,
};
use wisp_store::Store;

const MAX_NAME_CHARS: usize = 120;
const MAX_TAG_CHARS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 2_000;
const MAX_LIST_ENTRIES: usize = 256;
const MAX_ENTRY_CHARS: usize = 200;
const MAX_SECRET_FINDINGS: usize = 20;

pub(crate) fn package_from_specialist(
    spec: &Specialist,
    model_hint: Option<SpecialistModelHint>,
) -> SpecialistPackage {
    SpecialistPackage {
        format: FORMAT.into(),
        version: VERSION,
        name: spec.name.trim().into(),
        icon: spec.icon.clone(),
        color: spec.color.clone(),
        description: spec.description.clone(),
        instructions: spec.instructions.clone(),
        skills: spec.skills.clone(),
        connectors: spec.connectors.clone(),
        model_hint,
    }
}

fn check_text(label: &str, value: &str, max_chars: usize, multiline: bool) -> Result<(), String> {
    if value.chars().count() > max_chars {
        return Err(format!(
            "Specialist package {label} is longer than {max_chars} characters."
        ));
    }
    if value
        .chars()
        .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(format!(
            "Specialist package {label} contains control characters."
        ));
    }
    Ok(())
}

fn check_list(label: &str, list: &Option<Vec<String>>) -> Result<(), String> {
    let Some(list) = list else {
        return Ok(());
    };
    if list.len() > MAX_LIST_ENTRIES {
        return Err(format!(
            "Specialist package lists more than {MAX_LIST_ENTRIES} {label}."
        ));
    }
    for entry in list {
        if entry.trim().is_empty() {
            return Err(format!("Specialist package has an empty {label} entry."));
        }
        check_text(label, entry, MAX_ENTRY_CHARS, false)?;
    }
    Ok(())
}

pub(crate) fn validate_package(package: &SpecialistPackage) -> Result<(), String> {
    if package.format != FORMAT {
        return Err(format!(
            "Not a Wisp specialist package (format must be \"{FORMAT}\")."
        ));
    }
    if package.version == 0 || package.version > VERSION {
        return Err(format!(
            "Unsupported specialist package version {}. This version of Wisp reads version {VERSION}.",
            package.version
        ));
    }
    if package.name.trim().is_empty() {
        return Err("Specialist name is required.".into());
    }
    check_text("name", &package.name, MAX_NAME_CHARS, false)?;
    check_text("icon", &package.icon, MAX_TAG_CHARS, false)?;
    check_text("color", &package.color, MAX_TAG_CHARS, false)?;
    check_text(
        "description",
        &package.description,
        MAX_DESCRIPTION_CHARS,
        true,
    )?;
    // The whole file is already bounded by MAX_PACKAGE_BYTES.
    check_text("instructions", &package.instructions, usize::MAX, true)?;
    check_list("skills", &package.skills)?;
    check_list("connectors", &package.connectors)?;
    if let Some(hint) = &package.model_hint {
        for (label, value) in [("model provider", &hint.provider), ("model", &hint.model)] {
            if value.trim().is_empty() {
                return Err(format!("Specialist package {label} hint is empty."));
            }
            check_text(label, value, MAX_ENTRY_CHARS, false)?;
        }
    }
    Ok(())
}

pub(crate) fn parse_package(bytes: &[u8]) -> Result<SpecialistPackage, String> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(format!(
            "Specialist package is larger than {} KiB.",
            MAX_PACKAGE_BYTES / 1024
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Specialist package is not UTF-8 text.".to_string())?;
    // Editors on Windows commonly prepend a byte-order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let package: SpecialistPackage = serde_json::from_str(text)
        .map_err(|error| format!("Not a valid Wisp specialist package: {error}"))?;
    validate_package(&package)?;
    Ok(package)
}

fn read_package_file(path: &Path) -> Result<(SpecialistPackage, u64), String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a specialist package file.".into());
    }
    if metadata.len() > MAX_PACKAGE_BYTES {
        return Err(format!(
            "Specialist package is larger than {} KiB.",
            MAX_PACKAGE_BYTES / 1024
        ));
    }
    let mut bytes = Vec::new();
    // Bound the read too: the file may grow after the metadata check.
    file.take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let package = parse_package(&bytes)?;
    Ok((package, bytes.len() as u64))
}

fn normalized_list(list: &Option<Vec<String>>) -> Option<Vec<String>> {
    list.as_ref().map(|entries| {
        let mut seen = HashSet::new();
        entries
            .iter()
            .map(|entry| entry.trim().to_string())
            .filter(|entry| seen.insert(entry.clone()))
            .collect()
    })
}

/// A new custom specialist from a validated package. `None` (inherit) and
/// `Some([])` (allow none) survive unchanged.
pub(crate) fn specialist_from_package(package: &SpecialistPackage, model_id: &str) -> Specialist {
    Specialist {
        id: String::new(),
        name: package.name.trim().into(),
        icon: if package.icon.trim().is_empty() {
            "review".into()
        } else {
            package.icon.trim().into()
        },
        color: if package.color.trim().is_empty() {
            "clay".into()
        } else {
            package.color.trim().into()
        },
        description: package.description.clone(),
        instructions: package.instructions.clone(),
        model_id: model_id.trim().into(),
        review_backend: None,
        skills: normalized_list(&package.skills),
        connectors: normalized_list(&package.connectors),
        builtin: false,
    }
}

/// Default save name: the specialist name reduced to filesystem-safe characters.
pub(crate) fn package_file_name(name: &str) -> String {
    let mut stem = String::new();
    for character in name.trim().chars() {
        let character = if character.is_alphanumeric() || matches!(character, '-' | '_') {
            character
        } else {
            '-'
        };
        if character == '-' && stem.ends_with('-') {
            continue;
        }
        stem.push(character);
        if stem.chars().count() >= 60 {
            break;
        }
    }
    let stem = stem.trim_matches(['-', '_']);
    let stem = if stem.is_empty() { "specialist" } else { stem };
    format!("{stem}{FILE_SUFFIX}")
}

fn secret_patterns() -> &'static [(&'static str, Regex)] {
    static PATTERNS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            ("private_key", r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
            (
                "token_prefix",
                r"\b(?:sk-(?:ant-|proj-)?[A-Za-z0-9_\-]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}|xox[abprs]-[A-Za-z0-9\-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_\-]{35}|glpat-[A-Za-z0-9_\-]{20,}|hf_[A-Za-z0-9]{30,}|xai-[A-Za-z0-9]{20,}|(?:sk|rk|pk)_live_[A-Za-z0-9]{20,})",
            ),
            ("bearer", r"(?i)\bbearer\s+[A-Za-z0-9._~+/=\-]{20,}"),
            (
                "assignment",
                r#"(?i)\b(?:api[_-]?key|api[_-]?token|access[_-]?token|auth[_-]?token|access[_-]?key|secret[_-]?key|client[_-]?secret|secret|token|password|passwd)\b["']?\s*[:=]\s*["']?([^\s"'<>{}$]{8,})"#,
            ),
        ]
        .into_iter()
        .map(|(kind, pattern)| (kind, Regex::new(pattern).expect("valid secret pattern")))
        .collect()
    })
}

fn looks_like_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "your",
        "xxxx",
        "example",
        "placeholder",
        "redacted",
        "changeme",
        "env:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || value
            .chars()
            .all(|c| c == '*' || c == '.' || c == 'x' || c == 'X')
}

fn assignment_value_is_secret(value: &str) -> bool {
    if looks_like_placeholder(value) {
        return false;
    }
    let has_digit = value.chars().any(|c| c.is_ascii_digit());
    let has_alpha = value.chars().any(|c| c.is_ascii_alphabetic());
    (has_digit && has_alpha) || value.chars().count() >= 16
}

fn mask(value: &str) -> String {
    let shown = value.chars().take(4).collect::<String>();
    format!("{shown}…")
}

/// Lines in persona text that look like they carry credentials. Heuristic:
/// a hit only asks the user to confirm before exporting.
pub(crate) fn secret_findings(text: &str) -> Vec<SecretFinding> {
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let mut kinds = HashSet::new();
        for (kind, pattern) in secret_patterns() {
            for captures in pattern.captures_iter(line) {
                let whole = captures.get(0).map(|m| m.as_str()).unwrap_or_default();
                let excerpt = match *kind {
                    "private_key" => "-----BEGIN … PRIVATE KEY-----".to_string(),
                    "assignment" => {
                        let value = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
                        if !assignment_value_is_secret(value) {
                            continue;
                        }
                        let key = whole.split([':', '=']).next().unwrap_or_default().trim();
                        format!("{key}: {}", mask(value))
                    }
                    "bearer" => {
                        let token = whole.split_whitespace().nth(1).unwrap_or_default();
                        if looks_like_placeholder(token) {
                            continue;
                        }
                        format!("Bearer {}", mask(token))
                    }
                    _ if looks_like_placeholder(whole) => continue,
                    _ => mask(whole),
                };
                if kinds.insert(*kind) {
                    findings.push(SecretFinding {
                        kind: (*kind).into(),
                        line: (index + 1) as u32,
                        excerpt,
                    });
                }
                if findings.len() >= MAX_SECRET_FINDINGS {
                    return findings;
                }
            }
        }
    }
    findings
}

fn export_secret_findings(spec: &Specialist) -> Vec<SecretFinding> {
    secret_findings(&spec.instructions)
}

async fn custom_specialist(store: &Store, id: &str) -> Result<Specialist, String> {
    let spec = specialists::get(store, id)
        .await
        .ok_or_else(|| format!("Unknown specialist '{id}'."))?;
    if spec.builtin {
        return Err("Built-in specialists ship with Wisp and cannot be exported.".into());
    }
    Ok(spec)
}

/// Provider and model name of the bound chat profile, when it still exists.
async fn model_hint(store: &Store, spec: &Specialist) -> Option<SpecialistModelHint> {
    let id = spec.model_id.trim();
    if id.is_empty() {
        return None;
    }
    crate::models::chat_profiles(store)
        .await
        .into_iter()
        .find(|profile| profile.id == id)
        .map(|profile| SpecialistModelHint {
            provider: profile.provider,
            model: profile.model,
        })
}

pub(crate) async fn build_export(
    store: &Store,
    id: &str,
) -> Result<(SpecialistPackage, Vec<SecretFinding>), String> {
    let spec = custom_specialist(store, id).await?;
    let findings = export_secret_findings(&spec);
    let hint = model_hint(store, &spec).await;
    Ok((package_from_specialist(&spec, hint), findings))
}

pub(crate) fn package_json(package: &SpecialistPackage) -> Result<String, String> {
    let mut json = serde_json::to_string_pretty(package).map_err(|error| error.to_string())?;
    json.push('\n');
    Ok(json)
}

#[tauri::command]
pub async fn export_specialist(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    id: String,
    acknowledge_secrets: bool,
) -> Result<SpecialistExportOutcome, String> {
    use tauri_plugin_dialog::DialogExt;

    let (package, findings) = build_export(&state.store, &id).await?;
    if !findings.is_empty() && !acknowledge_secrets {
        return Ok(SpecialistExportOutcome::SecretWarning { findings });
    }
    let json = package_json(&package)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Wisp specialist", &["json"])
        .set_file_name(package_file_name(&package.name))
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let destination = receiver
        .await
        .map_err(|error| error.to_string())?
        .map(|path| path.into_path().map_err(|error| error.to_string()))
        .transpose()?;
    let Some(destination) = destination else {
        return Ok(SpecialistExportOutcome::Cancelled);
    };
    tokio::fs::write(&destination, json)
        .await
        .map_err(|error| error.to_string())?;
    Ok(SpecialistExportOutcome::Exported {
        path: destination.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn pick_specialist_package(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Wisp specialist", &["json"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let picked = receiver.await.map_err(|error| error.to_string())?;
    Ok(picked.map(|path| path.to_string()))
}

/// Local chat profiles whose model name matches the hint exactly (gateway
/// `vendor/model` ids compare on the tail segment). Same-provider first.
pub(crate) fn model_matches(
    hint: Option<&SpecialistModelHint>,
    profiles: &[crate::models::ModelProfile],
) -> Vec<SpecialistModelMatch> {
    let Some(hint) = hint else {
        return Vec::new();
    };
    let wanted = crate::models::model_id_tail(hint.model.trim());
    let mut matches = profiles
        .iter()
        .filter(|profile| {
            crate::models::model_id_tail(profile.model.trim()).eq_ignore_ascii_case(wanted)
        })
        .map(|profile| SpecialistModelMatch {
            id: profile.id.clone(),
            label: if profile.label.trim().is_empty() {
                profile.model.clone()
            } else {
                profile.label.clone()
            },
            provider: profile.provider.clone(),
            model: profile.model.clone(),
            same_provider: profile
                .provider
                .trim()
                .eq_ignore_ascii_case(hint.provider.trim()),
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|matched| !matched.same_provider);
    matches
}

pub(crate) fn name_conflict(existing: &[Specialist], name: &str) -> Option<SpecialistNameConflict> {
    specialists::find_by_name(existing, name).map(|spec| SpecialistNameConflict {
        id: spec.id.clone(),
        name: spec.name.clone(),
        builtin: spec.builtin,
    })
}

fn missing(entries: &Option<Vec<String>>, available: &HashSet<String>) -> Vec<String> {
    normalized_list(entries)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| !available.contains(entry))
        .collect()
}

/// Connector ids this device knows, enabled or not: bundled slugs, custom
/// MCP connections, the active project's plugin servers and the dev bridge.
async fn known_connectors(
    store: &Store,
    project: Option<&crate::ActiveProject>,
) -> HashSet<String> {
    let mut known = crate::bio_domains()
        .into_iter()
        .map(|domain| domain.slug)
        .collect::<HashSet<_>>();
    known.extend(
        crate::load_mcp_connections(store)
            .await
            .into_iter()
            .map(|connection| connection.id),
    );
    if let Some(project) = project {
        for (installation, manifest) in
            crate::plugins::enabled_plugin_manifests(store, &project.id).await
        {
            for server in manifest.mcp_servers {
                known.insert(format!("plugin:{}:{}", installation.plugin_id, server.id));
            }
        }
    }
    if std::env::var("WISP_MCP_COMMAND")
        .ok()
        .is_some_and(|command| command.split_whitespace().next().is_some())
    {
        known.insert("dev-mcp".into());
    }
    known
}

#[tauri::command]
pub async fn preview_specialist_import(
    state: State<'_, crate::AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    path: String,
) -> Result<SpecialistImportPreview, String> {
    let source = std::path::PathBuf::from(&path);
    let file_name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (package, bytes) = tokio::task::spawn_blocking(move || read_package_file(&source))
        .await
        .map_err(|error| error.to_string())??;
    let project = state.require_active(window.label()).ok();
    let existing = specialists::ensure(&state.store).await;
    let profiles = crate::models::chat_profiles(&state.store).await;
    let (missing_skills, skills_checked) = match &project {
        Some(project) => {
            let (catalog, _) = crate::project_skill_catalog(&state.store, project).await;
            let installed = catalog
                .all()
                .iter()
                .map(|skill| skill.name.clone())
                .collect::<HashSet<_>>();
            (missing(&package.skills, &installed), true)
        }
        None => (Vec::new(), false),
    };
    let connectors = known_connectors(&state.store, project.as_ref()).await;
    Ok(SpecialistImportPreview {
        name_conflict: name_conflict(&existing, &package.name),
        model_matches: model_matches(package.model_hint.as_ref(), &profiles),
        missing_skills,
        missing_connectors: missing(&package.connectors, &connectors),
        skills_checked,
        file_name,
        bytes,
        package,
    })
}

/// Validate the reviewed package again and add it under a fresh id. The
/// model binding is whatever local chat profile the user picked ("" follows
/// the active model); a package can never name a profile id itself.
pub(crate) async fn import_package(
    store: &Store,
    package: &SpecialistPackage,
    model_id: &str,
) -> Result<(String, Vec<Specialist>), String> {
    validate_package(package)?;
    let model_id = model_id.trim();
    if !model_id.is_empty()
        && !crate::models::chat_profiles(store)
            .await
            .iter()
            .any(|profile| profile.id == model_id)
    {
        return Err("The selected model no longer exists on this device.".into());
    }
    specialists::insert_new(store, specialist_from_package(package, model_id)).await
}

#[tauri::command]
pub async fn import_specialist(
    state: State<'_, crate::AppState>,
    package: SpecialistPackage,
    model_id: String,
) -> Result<Vec<Specialist>, String> {
    import_package(&state.store, &package, &model_id)
        .await
        .map(|(_, list)| list)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_store() -> (Store, std::path::PathBuf) {
        let tmp =
            std::env::temp_dir().join(format!("wisp_spec_pkg_{}.sqlite", uuid::Uuid::new_v4()));
        (Store::open(&tmp).await.unwrap(), tmp)
    }

    fn custom(name: &str) -> Specialist {
        Specialist {
            id: String::new(),
            name: name.into(),
            icon: "search".into(),
            color: "clay".into(),
            description: "finds papers".into(),
            instructions: "You hunt papers.\nCite DOIs.".into(),
            model_id: "m7".into(),
            review_backend: Some(crate::review::ReviewBackendConfig::AcpAgent {
                profile_id: "acp-1".into(),
            }),
            skills: Some(vec!["bear-support".into()]),
            connectors: None,
            builtin: false,
        }
    }

    fn package(name: &str) -> SpecialistPackage {
        package_from_specialist(&custom(name), None)
    }

    #[test]
    fn export_omits_machine_local_fields() {
        let package = package_from_specialist(
            &custom("Paper hunter"),
            Some(SpecialistModelHint {
                provider: "openai".into(),
                model: "gpt-5".into(),
            }),
        );
        let json = serde_json::to_value(&package).unwrap();
        let keys = json
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys.iter().map(String::as_str).collect::<HashSet<_>>(),
            HashSet::from([
                "format",
                "version",
                "name",
                "icon",
                "color",
                "description",
                "instructions",
                "skills",
                "connectors",
                "model_hint",
            ])
        );
        assert_eq!(json["format"], FORMAT);
        assert_eq!(json["version"], VERSION);
        assert!(json["connectors"].is_null());
        assert_eq!(
            json["model_hint"],
            serde_json::json!({"provider": "openai", "model": "gpt-5"})
        );
        let text = package_json(&package).unwrap();
        for leaked in ["m7", "acp-1", "builtin", "\"id\"", "api_key"] {
            assert!(!text.contains(leaked), "{leaked} leaked into {text}");
        }
    }

    #[test]
    fn parse_validates_format_version_size_and_fields() {
        let ok = package_json(&package("A")).unwrap();
        assert!(parse_package(ok.as_bytes()).is_ok());
        let with_bom = format!("\u{feff}{ok}");
        assert!(parse_package(with_bom.as_bytes()).is_ok());

        let wrong_format = ok.replace("\"wisp-specialist\"", "\"other\"");
        assert!(parse_package(wrong_format.as_bytes())
            .unwrap_err()
            .contains("format"));
        let newer = ok.replace("\"version\": 1", "\"version\": 2");
        assert!(parse_package(newer.as_bytes())
            .unwrap_err()
            .contains("version 2"));
        let no_name = ok.replace("\"name\": \"A\"", "\"name\": \"  \"");
        assert!(parse_package(no_name.as_bytes()).is_err());
        let id_field = ok.replacen('{', "{\"id\": \"reviewer\",", 1);
        assert!(parse_package(id_field.as_bytes()).is_err());
        let builtin_field = ok.replacen('{', "{\"builtin\": true,", 1);
        assert!(parse_package(builtin_field.as_bytes()).is_err());

        let huge = vec![b' '; MAX_PACKAGE_BYTES as usize + 1];
        assert!(parse_package(&huge).unwrap_err().contains("KiB"));
        assert!(parse_package(&[0xff, 0xfe]).unwrap_err().contains("UTF-8"));

        let mut bad = package("A");
        bad.skills = Some(vec!["  ".into()]);
        assert!(validate_package(&bad).is_err());
        let mut bad = package("A");
        bad.name = "line\nbreak".into();
        assert!(validate_package(&bad).is_err());
        let mut bad = package("A");
        bad.connectors = Some(vec!["x".into(); MAX_LIST_ENTRIES + 1]);
        assert!(validate_package(&bad).is_err());
    }

    #[test]
    fn read_package_file_rejects_oversized_files_before_parsing() {
        let dir = std::env::temp_dir().join(format!("wisp_spec_pkg_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.wisp-specialist.json");
        std::fs::write(&big, vec![b' '; MAX_PACKAGE_BYTES as usize + 1]).unwrap();
        assert!(read_package_file(&big).unwrap_err().contains("KiB"));
        let ok = dir.join("ok.wisp-specialist.json");
        let json = package_json(&package("A")).unwrap();
        std::fs::write(&ok, &json).unwrap();
        let (parsed, bytes) = read_package_file(&ok).unwrap();
        assert_eq!(parsed.name, "A");
        assert_eq!(bytes, json.len() as u64);
        assert!(read_package_file(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_keeps_inherit_and_allow_none_distinct() {
        let mut inherit = package("A");
        inherit.skills = None;
        inherit.connectors = Some(vec![]);
        let spec = specialist_from_package(&inherit, "");
        assert_eq!(spec.skills, None);
        assert_eq!(spec.connectors, Some(vec![]));
        assert!(!spec.builtin);
        assert!(spec.id.is_empty());
        assert!(spec.review_backend.is_none());

        let mut listed = package("A");
        listed.skills = Some(vec![" a ".into(), "a".into(), "b".into()]);
        let spec = specialist_from_package(&listed, " m1 ");
        assert_eq!(spec.skills, Some(vec!["a".into(), "b".into()]));
        assert_eq!(spec.model_id, "m1");
    }

    #[test]
    fn file_name_is_filesystem_safe() {
        assert_eq!(
            package_file_name("Paper hunter / v2"),
            "Paper-hunter-v2.wisp-specialist.json"
        );
        assert_eq!(
            package_file_name("文献猎手"),
            "文献猎手.wisp-specialist.json"
        );
        assert_eq!(
            package_file_name("../.."),
            "specialist.wisp-specialist.json"
        );
    }

    #[test]
    fn secret_scan_flags_credentials_and_ignores_placeholders() {
        let text = "Be concise.\n\
            Use key sk-proj-0000000000000000000000fake for calls.\n\
            Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345\n\
            api_key = \"a1b2c3d4e5f6\"\n\
            -----BEGIN OPENSSH PRIVATE KEY-----\n\
            password: hunter2hunter\n";
        let findings = secret_findings(text);
        let kinds = findings
            .iter()
            .map(|finding| (finding.line, finding.kind.as_str()))
            .collect::<Vec<_>>();
        assert!(kinds.contains(&(2, "token_prefix")));
        assert!(kinds.contains(&(3, "bearer")));
        assert!(kinds.contains(&(4, "assignment")));
        assert!(kinds.contains(&(5, "private_key")));
        assert!(kinds.contains(&(6, "assignment")));
        for finding in &findings {
            assert!(!finding.excerpt.contains("a1b2c3d4e5f6"));
            assert!(!finding.excerpt.contains("hunter2hunter"));
            assert!(!finding.excerpt.contains("abcdefghijklmnopqrstuvwxyz012345"));
        }

        for benign in [
            "Report the token count and max_tokens: 100000.",
            "api_key = <your key here>",
            "API_KEY=${OPENAI_API_KEY}",
            "password: required",
            "Ask the user for their api_key: YOUR_KEY_123",
            "Use sk-learn for models.",
            "Keys look like sk-xxxxxxxxxxxxxxxxxxxxxxxx.",
        ] {
            assert!(
                secret_findings(benign).is_empty(),
                "false positive: {benign}"
            );
        }
    }

    #[test]
    fn model_hint_matches_exact_model_names_only() {
        let profile = |id: &str, provider: &str, model: &str| -> crate::models::ModelProfile {
            serde_json::from_value(serde_json::json!({
                "id": id,
                "provider": provider,
                "api_url": "https://example.invalid",
                "model": model,
            }))
            .unwrap()
        };
        let profiles = vec![
            profile("m1", "openrouter", "openai/gpt-5"),
            profile("m2", "openai", "gpt-5"),
            profile("m3", "openai", "gpt-5-mini"),
            profile("m4", "openai", "gpt-5.1"),
        ];
        let hint = SpecialistModelHint {
            provider: "openai".into(),
            model: "gpt-5".into(),
        };
        let matches = model_matches(Some(&hint), &profiles);
        assert_eq!(
            matches.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["m2", "m1"]
        );
        assert!(matches[0].same_provider);
        assert!(!matches[1].same_provider);
        assert_eq!(matches[0].label, "gpt-5");
        assert!(model_matches(None, &profiles).is_empty());
    }

    #[tokio::test]
    async fn import_adds_fresh_custom_row_and_refuses_name_conflicts() {
        let (store, tmp) = test_store().await;
        let before = specialists::ensure(&store).await;
        let builtin_ids = before.iter().map(|s| s.id.clone()).collect::<Vec<_>>();

        let (id, list) = import_package(&store, &package("Paper hunter"), "")
            .await
            .unwrap();
        // A fresh custom id (`sp-<hex>`, never reused), not a built-in's.
        assert!(id.starts_with("sp-"), "{id}");
        assert!(!builtin_ids.contains(&id), "{id}");
        let imported = list.iter().find(|s| s.id == id).unwrap();
        assert!(!imported.builtin);
        assert!(imported.model_id.is_empty());
        assert!(imported.review_backend.is_none());
        assert_eq!(imported.skills, Some(vec!["bear-support".into()]));
        assert_eq!(imported.connectors, None);

        // Case-insensitive, built-ins included; nothing is overwritten.
        for name in [
            "paper HUNTER",
            "  Paper hunter ",
            "reviewer",
            "Scientific illustrator",
        ] {
            let error = import_package(&store, &package(name), "")
                .await
                .unwrap_err();
            assert!(error.contains("already exists"), "{name}: {error}");
        }
        let after = specialists::ensure(&store).await;
        assert_eq!(after.len(), before.len() + 1);
        for (id, original) in builtin_ids.iter().zip(&before) {
            let current = after.iter().find(|s| &s.id == id).unwrap();
            assert_eq!(current, original);
        }

        // A dangling or invented model id is refused instead of bound.
        let error = import_package(&store, &package("Other"), "m404")
            .await
            .unwrap_err();
        assert!(error.contains("no longer exists"));

        let mut invalid = package("Third");
        invalid.version = 9;
        assert!(import_package(&store, &invalid, "").await.is_err());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn preview_conflict_lookup_includes_builtins() {
        let (store, tmp) = test_store().await;
        let list = specialists::ensure(&store).await;
        let conflict = name_conflict(&list, "READER").unwrap();
        assert_eq!(conflict.id, "reader");
        assert!(conflict.builtin);
        assert!(name_conflict(&list, "Readers").is_none());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn export_refuses_builtins_and_flags_secrets() {
        let (store, tmp) = test_store().await;
        assert!(build_export(&store, "reviewer")
            .await
            .unwrap_err()
            .contains("Built-in"));
        assert!(build_export(&store, "missing").await.is_err());

        let mut spec = custom("Keyholder");
        spec.instructions = "Call the API with token: ab12cd34ef56gh78".into();
        let list = specialists::upsert(&store, spec).await.unwrap();
        let id = list
            .iter()
            .find(|s| s.name == "Keyholder")
            .unwrap()
            .id
            .clone();
        let (package, findings) = build_export(&store, &id).await.unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, "assignment");
        // The bound profile "m7" does not exist here, so no hint is invented.
        assert!(package.model_hint.is_none());
        assert_eq!(package.skills, Some(vec!["bear-support".into()]));
        assert_eq!(package.connectors, None);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn missing_reports_unknown_entries_only() {
        let available = HashSet::from(["pubmed".to_string(), "mcp-1".to_string()]);
        assert_eq!(
            missing(
                &Some(vec!["pubmed".into(), "plugin:x:y".into(), "mcp-1".into()]),
                &available
            ),
            vec!["plugin:x:y".to_string()]
        );
        assert!(missing(&None, &available).is_empty());
        assert!(missing(&Some(vec![]), &available).is_empty());
    }
}
