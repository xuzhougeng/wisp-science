//! Deterministic Publication Evidence Capsule construction.
//!
//! A Capsule is derived only from a Frozen/Published revision manifest and
//! exact content-addressed ArtifactVersion snapshots. Live workspace files are
//! never a fallback.
//!
//! Shared by the WebView host (`src-tauri`) and the embedded desktop host
//! (`wisp-server`). Each host supplies what this crate does not depend on: the
//! security scanner of its freeze module and the ZIP writer ([`CapsuleHost`]).

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};
use wisp_store::{
    canonical_json, canonical_json_sha256, ArtifactMaterialization, CapsuleBuild,
    EvidenceVisibility, PublicationRevisionState, Store,
};

pub const CAPSULE_KIND: &str = "wisp.publication_evidence_capsule";
const CAPSULE_SCHEMA_VERSION: i64 = 1;
const SECURITY_SCAN_OVERLAP: usize = 2_048;

/// Finds credentials, and for a public capsule machine-local or internal
/// network details, in text: the freeze module's `capsule_security_violations`.
pub type SecurityScan = fn(&str, bool) -> Vec<String>;

/// The archive a capsule is written into. Entries arrive in path order;
/// a host writes each deflated (level 6) with the zero timestamp and mode
/// 0644 so that the same revision always gives the same bytes.
pub trait CapsuleArchive {
    fn start_file(&mut self, path: &str, size_bytes: u64) -> Result<(), String>;
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Ends the archive and hands back its file, not yet synced.
    fn finish(self: Box<Self>) -> Result<File, String>;
}

/// What a host lends the capsule builder.
#[derive(Clone, Copy)]
pub struct CapsuleHost {
    pub scan: SecurityScan,
    /// Starts an archive in a new, empty file.
    pub open: fn(File) -> Box<dyn CapsuleArchive>,
}

#[derive(Clone)]
pub struct BlobSource {
    pub path: PathBuf,
    pub project_root: PathBuf,
    pub expected_sha256: String,
    pub expected_size: u64,
    pub scan_as_text: bool,
    pub public: bool,
}

enum EntryBody {
    Generated(Vec<u8>),
    Blob(BlobSource),
}

struct CapsuleEntry {
    path: String,
    mime: String,
    sha256: String,
    size_bytes: u64,
    source_kind: String,
    source_id: String,
    visibility: String,
    license: Option<String>,
    producing_run_id: Option<String>,
    dependency_roles: Vec<String>,
    body: EntryBody,
}

#[derive(Serialize)]
struct CapsuleEntryRecord<'a> {
    path: &'a str,
    mime: &'a str,
    sha256: &'a str,
    size_bytes: u64,
    source_kind: &'a str,
    source_id: &'a str,
    visibility: &'a str,
    license: Option<&'a str>,
    producing_run_id: Option<&'a str>,
    dependency_roles: &'a [String],
}

impl CapsuleEntry {
    fn record(&self) -> CapsuleEntryRecord<'_> {
        CapsuleEntryRecord {
            path: &self.path,
            mime: &self.mime,
            sha256: &self.sha256,
            size_bytes: self.size_bytes,
            source_kind: &self.source_kind,
            source_id: &self.source_id,
            visibility: &self.visibility,
            license: self.license.as_deref(),
            producing_run_id: self.producing_run_id.as_deref(),
            dependency_roles: &self.dependency_roles,
        }
    }
}

struct CapsulePlan {
    manifest: Value,
    revision_manifest_sha256: String,
    visibility: EvidenceVisibility,
    entries: BTreeMap<String, CapsuleEntry>,
}

fn parse_visibility(value: &str) -> Result<EvidenceVisibility, String> {
    match value {
        "public" => Ok(EvidenceVisibility::Public),
        "restricted" => Ok(EvidenceVisibility::Restricted),
        "private" => Ok(EvidenceVisibility::Private),
        _ => Err(format!("Unsupported Capsule visibility '{value}'")),
    }
}

fn visibility_rank(visibility: EvidenceVisibility) -> u8 {
    match visibility {
        EvidenceVisibility::Public => 0,
        EvidenceVisibility::Restricted => 1,
        EvidenceVisibility::Private => 2,
    }
}

fn visibility_allows(target: EvidenceVisibility, source: EvidenceVisibility) -> bool {
    visibility_rank(source) <= visibility_rank(target)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn safe_archive_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 512
        || path.contains('\\')
        || path.contains('\0')
        || path.starts_with('/')
    {
        return false;
    }
    let mut count = 0;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(value)
                if value.to_str().is_some_and(|value| {
                    !value.is_empty()
                        && value.chars().all(|character| {
                            character.is_ascii_alphanumeric()
                                || matches!(character, '.' | '-' | '_')
                        })
                }) =>
            {
                count += 1
            }
            _ => return false,
        }
    }
    count >= 1
}

fn allowed_artifact_path(path: &str) -> bool {
    safe_archive_path(path)
        && matches!(
            path.split('/').next(),
            Some("figures" | "tables" | "evidence" | "data" | "reference-results")
        )
        && !matches!(
            path,
            "evidence/manifest.json"
                | "evidence/selection.json"
                | "data/access.json"
                | "provenance/lineage.json"
                | "reference-results/manifest.json"
                | "verification/report.json"
        )
}

fn safe_snapshot_path(
    project_root: &Path,
    storage_path: &str,
    checksum: &str,
) -> Result<PathBuf, String> {
    if storage_path.contains('\\') || Path::new(storage_path).is_absolute() {
        return Err("Capsule snapshot path must be project-relative".into());
    }
    let components = Path::new(storage_path)
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "Capsule snapshot path contains an unsafe component".to_string())?;
    if components.len() != 5
        || components[0] != ".wisp"
        || components[1] != "artifacts"
        || components[2] != "sha256"
        || components[3] != checksum[..2]
        || !components[4].starts_with(checksum)
    {
        return Err("Capsule bytes must come from content-addressed snapshot storage".into());
    }
    Ok(project_root.join(storage_path))
}

fn text_like(mime: &str, path: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/ld+json"
                | "application/xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/x-python"
                | "application/x-r"
                | "application/x-sh"
        )
        || Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "txt"
                        | "md"
                        | "csv"
                        | "tsv"
                        | "json"
                        | "jsonl"
                        | "yaml"
                        | "yml"
                        | "xml"
                        | "py"
                        | "r"
                        | "sh"
                        | "toml"
                )
            })
}

fn one_line(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect()
}

fn manifest_array(manifest: &Value, key: &str) -> Value {
    manifest
        .get(key)
        .filter(|value| value.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn generated_entry(
    scan: SecurityScan,
    path: &str,
    mime: &str,
    body: String,
    revision_id: &str,
    visibility: EvidenceVisibility,
) -> Result<CapsuleEntry, String> {
    if !safe_archive_path(path) {
        return Err(format!("Generated Capsule path is unsafe: {path}"));
    }
    let violations = scan(&body, visibility == EvidenceVisibility::Public);
    if !violations.is_empty() {
        return Err(format!(
            "Generated Capsule metadata failed security validation: {}",
            violations.join(", ")
        ));
    }
    let bytes = body.into_bytes();
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(CapsuleEntry {
        path: path.into(),
        mime: mime.into(),
        sha256,
        size_bytes: bytes.len() as u64,
        source_kind: "publication_revision".into(),
        source_id: revision_id.into(),
        visibility: visibility.as_str().into(),
        license: None,
        producing_run_id: None,
        dependency_roles: Vec::new(),
        body: EntryBody::Generated(bytes),
    })
}

fn insert_entry(
    entries: &mut BTreeMap<String, CapsuleEntry>,
    entry: CapsuleEntry,
) -> Result<(), String> {
    if entries.insert(entry.path.clone(), entry).is_some() {
        return Err("Capsule contains duplicate entry paths".into());
    }
    Ok(())
}

fn render_readme(manifest: &Value, manifest_sha256: &str) -> String {
    let title = manifest
        .pointer("/publication/title")
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Untitled publication".into());
    let revision_id = manifest
        .get("publication_revision_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let capability = manifest
        .get("capability_level")
        .and_then(Value::as_str)
        .unwrap_or("archived");
    let visibility = manifest
        .get("target_visibility")
        .and_then(Value::as_str)
        .unwrap_or("private");
    format!(
        "# {title}\n\n\
         This is a Wisp Publication Evidence Capsule for revision `{revision_id}`.\n\n\
         - Revision manifest SHA-256: `{manifest_sha256}`\n\
         - Capability level: `{capability}`\n\
         - Visibility: `{visibility}`\n\n\
         `capsule.json` is the Capsule index. `evidence/manifest.json` is the exact frozen \
         revision manifest. `checksums.sha256` authenticates every other archive entry.\n"
    )
}

fn render_reproduce(manifest: &Value) -> String {
    let capability = manifest
        .get("capability_level")
        .and_then(Value::as_str)
        .unwrap_or("archived");
    format!(
        "# Reproduce or inspect\n\n\
         Capability level: `{capability}`.\n\n\
         1. Verify `checksums.sha256` before using any file.\n\
         2. Read `data/access.json` and obtain every reference-only or restricted input.\n\
         3. Inspect exact Run, code, input, output, and environment identities in \
         `provenance/lineage.json`.\n\
         4. Compare produced outputs with the immutable files listed in \
         `reference-results/manifest.json` using `verification/report.json`.\n\n\
         This Capsule does not claim a clean rerun unless its capability level and verification \
         report explicitly record one.\n"
    )
}

fn render_citation(manifest: &Value) -> String {
    let title = manifest
        .pointer("/publication/title")
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "Untitled publication".into());
    let title = serde_json::to_string(&title).unwrap_or_else(|_| "\"Untitled publication\"".into());
    "cff-version: 1.2.0\n".to_string()
        + "message: \"If you use this evidence capsule, cite the associated publication.\"\n"
        + &format!("title: {title}\n")
        + "type: dataset\n"
}

fn json_document(value: &Value) -> String {
    canonical_json(value)
}

async fn prepare_capsule_plan(
    store: &Store,
    revision_id: &str,
    scan: SecurityScan,
) -> Result<CapsulePlan, String> {
    let revision = store
        .get_publication_revision(revision_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Publication revision not found".to_string())?;
    if !matches!(
        revision.state,
        PublicationRevisionState::Frozen | PublicationRevisionState::Published
    ) {
        return Err("Capsules can be built only from Frozen or Published revisions".into());
    }
    let manifest_json = revision
        .manifest_json
        .as_deref()
        .ok_or_else(|| "Frozen Publication revision has no manifest".to_string())?;
    let stored_sha256 = revision
        .manifest_sha256
        .as_deref()
        .ok_or_else(|| "Frozen Publication revision has no manifest SHA-256".to_string())?;
    let manifest: Value = serde_json::from_str(manifest_json)
        .map_err(|error| format!("Frozen Publication manifest is invalid: {error}"))?;
    let (canonical, calculated_sha256) = canonical_json_sha256(&manifest);
    if canonical != manifest_json || calculated_sha256 != stored_sha256 {
        return Err("Frozen Publication manifest hash or canonical form is invalid".into());
    }
    if manifest.get("schema_version").and_then(Value::as_i64) != Some(CAPSULE_SCHEMA_VERSION)
        || manifest
            .get("publication_revision_id")
            .and_then(Value::as_str)
            != Some(revision_id)
    {
        return Err("Frozen Publication manifest identity is invalid".into());
    }
    let visibility = manifest
        .get("target_visibility")
        .and_then(Value::as_str)
        .ok_or_else(|| "Frozen Publication manifest has no target visibility".to_string())
        .and_then(parse_visibility)?;
    let violations = scan(manifest_json, visibility == EvidenceVisibility::Public);
    if !violations.is_empty() {
        return Err(format!(
            "Frozen Publication manifest failed Capsule security validation: {}",
            violations.join(", ")
        ));
    }

    let publication = store
        .get_publication(&revision.publication_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Publication not found".to_string())?;
    let (_, workspace_dir) = store
        .get_project(&publication.project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Publication project not found".to_string())?;
    if workspace_dir.trim().is_empty() {
        return Err("Publication project has no workspace".into());
    }
    let project_root = dunce::canonicalize(&workspace_dir)
        .map_err(|error| format!("Publication workspace is unavailable: {error}"))?;

    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| "Frozen Publication manifest has no files array".to_string())?;
    let mut entries = BTreeMap::<String, CapsuleEntry>::new();
    let mut seen_source_ids = BTreeSet::<String>::new();
    for file in files {
        if file.get("include_bytes").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let source_kind = file
            .get("source_kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "Capsule file lacks source_kind".to_string())?;
        if source_kind != "artifact_version" {
            return Err("Capsule bytes require an exact ArtifactVersion source".into());
        }
        let source_id = file
            .get("source_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Capsule file lacks source_id".to_string())?;
        if !seen_source_ids.insert(source_id.to_string()) {
            return Err("Frozen Publication manifest repeats an included ArtifactVersion".into());
        }
        let path = file
            .get("capsule_path")
            .and_then(Value::as_str)
            .ok_or_else(|| "Capsule file lacks capsule_path".to_string())?;
        if !allowed_artifact_path(path) {
            return Err(format!(
                "Frozen Publication manifest has unsafe Capsule path: {path}"
            ));
        }
        let expected_sha256 = file
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|value| valid_sha256(value))
            .ok_or_else(|| "Included Capsule file lacks a valid SHA-256".to_string())?;
        let expected_size = file
            .get("size_bytes")
            .and_then(Value::as_i64)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| "Included Capsule file lacks a valid size".to_string())?;
        let source_visibility = file
            .get("visibility")
            .and_then(Value::as_str)
            .ok_or_else(|| "Included Capsule file lacks visibility".to_string())
            .and_then(parse_visibility)?;
        if !visibility_allows(visibility, source_visibility) {
            return Err("Capsule cannot include bytes above its target visibility".into());
        }
        let context = store
            .get_artifact_version_context(source_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("ArtifactVersion '{source_id}' no longer exists"))?;
        if context.project_id != publication.project_id
            || context.version.materialization != ArtifactMaterialization::Snapshot
            || context.version.checksum.as_deref() != Some(expected_sha256)
            || context
                .version
                .size_bytes
                .and_then(|size| u64::try_from(size).ok())
                != Some(expected_size)
        {
            return Err(format!(
                "ArtifactVersion '{source_id}' no longer matches the frozen Capsule manifest"
            ));
        }
        let blob_path = safe_snapshot_path(
            &project_root,
            &context.version.storage_path,
            expected_sha256,
        )?;
        let mime = file
            .get("mime")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("application/octet-stream");
        let dependency_roles = file
            .get("dependency_roles")
            .and_then(Value::as_array)
            .map(|roles| {
                roles
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        insert_entry(
            &mut entries,
            CapsuleEntry {
                path: path.into(),
                mime: mime.into(),
                sha256: expected_sha256.into(),
                size_bytes: expected_size,
                source_kind: source_kind.into(),
                source_id: source_id.into(),
                visibility: source_visibility.as_str().into(),
                license: file
                    .get("license")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                producing_run_id: file
                    .get("producing_run_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                dependency_roles,
                body: EntryBody::Blob(BlobSource {
                    path: blob_path,
                    project_root: project_root.clone(),
                    expected_sha256: expected_sha256.into(),
                    expected_size,
                    scan_as_text: text_like(mime, path),
                    public: visibility == EvidenceVisibility::Public,
                }),
            },
        )?;
    }

    let evidence = json!({
        "evidence": manifest_array(&manifest, "evidence"),
        "item_links": manifest_array(&manifest, "item_links"),
        "items": manifest_array(&manifest, "items"),
        "publication_revision_id": revision_id,
        "schema_version": CAPSULE_SCHEMA_VERSION,
        "supersessions": manifest_array(&manifest, "supersessions"),
    });
    let provenance = json!({
        "code": manifest_array(&manifest, "code"),
        "environments": manifest_array(&manifest, "environments"),
        "inputs": manifest_array(&manifest, "inputs"),
        "outputs": manifest_array(&manifest, "outputs"),
        "publication_revision_id": revision_id,
        "runs": manifest_array(&manifest, "runs"),
        "schema_version": CAPSULE_SCHEMA_VERSION,
    });
    let omitted_files = files
        .iter()
        .filter(|file| file.get("include_bytes").and_then(Value::as_bool) != Some(true))
        .cloned()
        .collect::<Vec<_>>();
    let access = json!({
        "external_resources": manifest_array(&manifest, "external_resources"),
        "omissions": manifest_array(&manifest, "omissions"),
        "publication_revision_id": revision_id,
        "reference_only_files": omitted_files,
        "schema_version": CAPSULE_SCHEMA_VERSION,
        "target_visibility": visibility.as_str(),
    });
    let reference_results = json!({
        "files": files,
        "publication_revision_id": revision_id,
        "schema_version": CAPSULE_SCHEMA_VERSION,
    });
    let verification = json!({
        "blockers": manifest_array(&manifest, "blockers"),
        "capability_level": manifest.get("capability_level").cloned().unwrap_or(Value::Null),
        "publication_revision_id": revision_id,
        "schema_version": CAPSULE_SCHEMA_VERSION,
        "verification": manifest_array(&manifest, "verification"),
        "warnings": manifest_array(&manifest, "warnings"),
        "waivers": manifest_array(&manifest, "waivers"),
    });

    for entry in [
        generated_entry(
            scan,
            "README.md",
            "text/markdown",
            render_readme(&manifest, stored_sha256),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "REPRODUCE.md",
            "text/markdown",
            render_reproduce(&manifest),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "CITATION.cff",
            "text/yaml",
            render_citation(&manifest),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "evidence/manifest.json",
            "application/json",
            manifest_json.into(),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "evidence/selection.json",
            "application/json",
            json_document(&evidence),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "provenance/lineage.json",
            "application/json",
            json_document(&provenance),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "data/access.json",
            "application/json",
            json_document(&access),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "reference-results/manifest.json",
            "application/json",
            json_document(&reference_results),
            revision_id,
            visibility,
        )?,
        generated_entry(
            scan,
            "verification/report.json",
            "application/json",
            json_document(&verification),
            revision_id,
            visibility,
        )?,
    ] {
        insert_entry(&mut entries, entry)?;
    }

    let capsule_manifest = json!({
        "capsule_kind": CAPSULE_KIND,
        "capability_level": manifest.get("capability_level").cloned().unwrap_or(Value::Null),
        "entries": entries.values().map(CapsuleEntry::record).collect::<Vec<_>>(),
        "publication": manifest.get("publication").cloned().unwrap_or_else(|| json!({})),
        "publication_revision_id": revision_id,
        "revision_manifest_sha256": stored_sha256,
        "schema_version": CAPSULE_SCHEMA_VERSION,
        "target_visibility": visibility.as_str(),
    });
    insert_entry(
        &mut entries,
        generated_entry(
            scan,
            "capsule.json",
            "application/json",
            json_document(&capsule_manifest),
            revision_id,
            visibility,
        )?,
    )?;

    let checksums = entries
        .values()
        .map(|entry| format!("{}  {}\n", entry.sha256, entry.path))
        .collect::<String>();
    insert_entry(
        &mut entries,
        generated_entry(
            scan,
            "checksums.sha256",
            "text/plain",
            checksums,
            revision_id,
            visibility,
        )?,
    )?;

    Ok(CapsulePlan {
        manifest,
        revision_manifest_sha256: stored_sha256.into(),
        visibility,
        entries,
    })
}

pub fn validate_snapshot_file(source: &BlobSource) -> Result<(), String> {
    let relative = source
        .path
        .strip_prefix(&source.project_root)
        .map_err(|_| "Capsule snapshot escaped the project workspace".to_string())?;
    let mut current = source.project_root.clone();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err("Capsule snapshot path contains an unsafe component".into());
        };
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| format!("Capsule snapshot is unavailable: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Capsule snapshots must not follow symlinks".into());
        }
    }
    let metadata = std::fs::symlink_metadata(&source.path)
        .map_err(|error| format!("Capsule snapshot is unavailable: {error}"))?;
    if !metadata.is_file() || metadata.len() != source.expected_size {
        return Err("Capsule snapshot size no longer matches the frozen manifest".into());
    }
    Ok(())
}

/// The frozen manifest of a revision and the exact snapshots its capsule
/// would copy, keyed by ArtifactVersion: what a clean rerun may read.
pub async fn frozen_reproduction_sources(
    store: &Store,
    revision_id: &str,
    scan: SecurityScan,
) -> Result<(Value, BTreeMap<String, BlobSource>), String> {
    let CapsulePlan {
        manifest, entries, ..
    } = prepare_capsule_plan(store, revision_id, scan).await?;
    let sources = entries
        .into_values()
        .filter_map(|entry| match entry.body {
            EntryBody::Blob(source) => Some((entry.source_id, source)),
            EntryBody::Generated(_) => None,
        })
        .collect();
    Ok((manifest, sources))
}

fn scan_chunk(
    scan: SecurityScan,
    overlap: &mut Vec<u8>,
    chunk: &[u8],
    public: bool,
) -> Result<(), String> {
    let mut scanned = Vec::with_capacity(overlap.len() + chunk.len());
    scanned.extend_from_slice(overlap);
    scanned.extend_from_slice(chunk);
    let text = String::from_utf8_lossy(&scanned);
    let violations = scan(&text, public);
    if !violations.is_empty() {
        return Err(format!(
            "Capsule artifact failed security validation: {}",
            violations.join(", ")
        ));
    }
    let keep = scanned.len().min(SECURITY_SCAN_OVERLAP);
    overlap.clear();
    overlap.extend_from_slice(&scanned[scanned.len() - keep..]);
    Ok(())
}

fn write_blob(
    scan: SecurityScan,
    zip: &mut dyn CapsuleArchive,
    entry: &CapsuleEntry,
    source: &BlobSource,
) -> Result<(), String> {
    validate_snapshot_file(source)?;
    zip.start_file(&entry.path, entry.size_bytes)?;
    let mut input = BufReader::new(
        File::open(&source.path)
            .map_err(|error| format!("Cannot open Capsule snapshot: {error}"))?,
    );
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 128 * 1024];
    let mut overlap = Vec::<u8>::new();
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("Cannot read Capsule snapshot: {error}"))?;
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read];
        if source.scan_as_text {
            scan_chunk(scan, &mut overlap, bytes, source.public)?;
        }
        size = size.saturating_add(read as u64);
        digest.update(bytes);
        zip.write_all(bytes)?;
    }
    let checksum = hex::encode(digest.finalize());
    if size != source.expected_size || checksum != source.expected_sha256 {
        return Err(format!(
            "Capsule snapshot checksum mismatch for ArtifactVersion '{}'",
            entry.source_id
        ));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn write_capsule_archive(
    plan: CapsulePlan,
    destination: &Path,
    host: CapsuleHost,
) -> Result<String, String> {
    if destination.exists() {
        return Err("Capsule destination already exists; choose a new file name".into());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "Capsule destination has no parent directory".to_string())?;
    let parent = dunce::canonicalize(parent)
        .map_err(|error| format!("Capsule destination directory is unavailable: {error}"))?;
    let filename = destination
        .file_name()
        .ok_or_else(|| "Capsule destination has no file name".to_string())?;
    let destination = parent.join(filename);
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        filename.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let output = File::options()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|error| format!("Cannot create Capsule archive: {error}"))?;
        let mut zip = (host.open)(output);
        for entry in plan.entries.values() {
            match &entry.body {
                EntryBody::Generated(bytes) => {
                    zip.start_file(&entry.path, entry.size_bytes)?;
                    zip.write_all(bytes)?;
                }
                EntryBody::Blob(source) => write_blob(host.scan, zip.as_mut(), entry, source)?,
            }
        }
        let output = zip.finish()?;
        output.sync_all().map_err(|error| error.to_string())?;
        let archive_sha256 = hash_file(&temp)?;
        std::fs::rename(&temp, &destination)
            .map_err(|error| format!("Cannot finalize Capsule archive: {error}"))?;
        Ok(archive_sha256)
    })();
    if result.is_err() && temp.exists() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

/// Builds the capsule of a Frozen or Published revision at `destination`,
/// a new file, and records the attempt with the revision.
pub async fn build_publication_capsule_to(
    store: &Store,
    revision_id: &str,
    destination: &Path,
    host: CapsuleHost,
) -> Result<CapsuleBuild, String> {
    if destination.exists() {
        return Err("Capsule destination already exists; choose a new file name".into());
    }
    let plan = prepare_capsule_plan(store, revision_id, host.scan).await?;
    let build_id = uuid::Uuid::new_v4().to_string();
    let destination_text = destination.to_string_lossy().into_owned();
    store
        .start_capsule_build(
            &build_id,
            revision_id,
            "zip",
            plan.visibility,
            &destination_text,
            &plan.revision_manifest_sha256,
        )
        .await
        .map_err(|error| error.to_string())?;
    let destination = destination.to_path_buf();
    let result =
        tokio::task::spawn_blocking(move || write_capsule_archive(plan, &destination, host)).await;
    let result = match result {
        Ok(result) => result,
        Err(error) => Err(format!("Capsule Builder task failed: {error}")),
    };
    match result {
        Ok(archive_sha256) => store
            .complete_capsule_build(&build_id, &archive_sha256)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => {
            let persistence_error = store.fail_capsule_build(&build_id, &error).await.err();
            if let Some(persistence_error) = persistence_error {
                Err(format!(
                    "{error}; failed to persist Capsule failure: {persistence_error}"
                ))
            } else {
                Err(error)
            }
        }
    }
}

/// The file name a capsule is offered under when saving it.
pub fn capsule_filename(title: &str, revision_id: &str) -> String {
    let mut safe = format!("{}-{}", one_line(title), revision_id)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while safe.contains("--") {
        safe = safe.replace("--", "-");
    }
    let safe = safe.trim_matches('-').chars().take(100).collect::<String>();
    format!(
        "wisp-publication-{}.zip",
        if safe.is_empty() { "capsule" } else { &safe }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_paths_reject_traversal_and_reserved_targets() {
        assert!(allowed_artifact_path("figures/version-1-figure.png"));
        assert!(!allowed_artifact_path("../figure.png"));
        assert!(!allowed_artifact_path("figures/../../secret"));
        assert!(!allowed_artifact_path("evidence/manifest.json"));
        assert!(!allowed_artifact_path(r"figures\figure.png"));
        assert!(!allowed_artifact_path("figures/C:figure.png"));
        let checksum = "a".repeat(64);
        let portable = format!(".wisp/artifacts/sha256/aa/{checksum}.txt");
        assert!(safe_snapshot_path(Path::new("workspace"), &portable, &checksum).is_ok());
        assert!(safe_snapshot_path(
            Path::new("workspace"),
            r"C:\Users\Alice\artifact.txt",
            &checksum,
        )
        .is_err());
        assert!(safe_snapshot_path(
            Path::new("workspace"),
            "/Users/alice/artifact.txt",
            &checksum,
        )
        .is_err());
    }

    #[test]
    fn capsule_names_are_portable() {
        assert_eq!(
            capsule_filename("T cells: a study", "revision 1"),
            "wisp-publication-T-cells-a-study-revision-1.zip"
        );
    }
}
