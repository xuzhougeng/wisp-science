//! Publication Freeze preparation and policy evaluation.
//!
//! Filesystem work happens before the final SQLite transaction. A failed
//! preparation may leave an unreferenced content-addressed blob, but the Store
//! either commits every late capture plus the frozen manifest or none of them.

use crate::snapshot_store::{capture_file, SnapshotPolicy, DEFAULT_SNAPSHOT_LIMIT};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use wisp_store::{
    canonical_json, canonical_json_sha256, ArtifactCaptureTiming, ArtifactMaterialization,
    ArtifactVersionContext, EvidenceBinding, EvidenceReproductionState, EvidenceSelectionState,
    EvidenceSourceKind, EvidenceVisibility, ExternalResource, LineageBasis, LineageConfidence,
    PublicationCapabilityLevel, PublicationFreezeCommit, PublicationFreezePolicy,
    PublicationItemKind, PublicationLateCapture, PublicationReadiness, PublicationReadinessFinding,
    PublicationRevision, PublicationRevisionState, PublicationWaiver, RunCodeSnapshot, RunInput,
    RunOutput, RunRecord, RunStatus, Store,
};

const MAX_TEXT_SCAN_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PublicationFreezeOutcome {
    pub frozen: bool,
    pub revision: PublicationRevision,
    pub readiness: PublicationReadiness,
}

struct PreparedFreeze {
    readiness: PublicationReadiness,
    commit: PublicationFreezeCommit,
}

#[derive(Clone)]
struct ResolvedArtifact {
    id: String,
    artifact_id: String,
    version_number: i64,
    filename: String,
    content_type: String,
    storage_path: String,
    size_bytes: Option<i64>,
    checksum: Option<String>,
    producing_run_id: Option<String>,
    env_snapshot_hash: Option<String>,
    materialization: ArtifactMaterialization,
    capture_timing: ArtifactCaptureTiming,
    logical_key: Option<String>,
}

impl ResolvedArtifact {
    fn from_context(context: ArtifactVersionContext) -> Self {
        Self {
            id: context.version.id,
            artifact_id: context.version.artifact_id,
            version_number: context.version.version_number,
            filename: context.filename,
            content_type: context.version.content_type,
            storage_path: context.version.storage_path,
            size_bytes: context.version.size_bytes,
            checksum: context.version.checksum,
            producing_run_id: context.version.producing_run_id,
            env_snapshot_hash: context.version.env_snapshot_hash,
            materialization: context.version.materialization,
            capture_timing: context.version.capture_timing,
            logical_key: context.logical_key,
        }
    }

    fn manifest_value(&self) -> Value {
        json!({
            "artifact_id": self.artifact_id,
            "capture_timing": self.capture_timing.as_str(),
            "content_type": self.content_type,
            "env_snapshot_hash": self.env_snapshot_hash,
            "filename": self.filename,
            "logical_key": self.logical_key,
            "materialization": self.materialization.as_str(),
            "producing_run_id": self.producing_run_id,
            "sha256": self.checksum,
            "size_bytes": self.size_bytes,
            "source_id": self.id,
            "source_kind": "artifact_version",
            "storage_path": self.storage_path,
            "version_number": self.version_number,
        })
    }
}

#[derive(Clone, Copy)]
enum FindingBucket {
    Blocker,
    Warning,
    Omission,
}

#[derive(Default)]
struct Findings {
    blockers: Vec<PublicationReadinessFinding>,
    warnings: Vec<PublicationReadinessFinding>,
    omissions: Vec<PublicationReadinessFinding>,
    seen: BTreeSet<String>,
}

impl Findings {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        bucket: FindingBucket,
        code: &str,
        message: impl Into<String>,
        binding_id: Option<&str>,
        source_kind: Option<EvidenceSourceKind>,
        source_id: Option<&str>,
        waivable: bool,
        details: Value,
    ) {
        let bucket_name = match bucket {
            FindingBucket::Blocker => "blocker",
            FindingBucket::Warning => "warning",
            FindingBucket::Omission => "omission",
        };
        let key = format!(
            "{bucket_name}\0{code}\0{}\0{}",
            binding_id.unwrap_or_default(),
            source_id.unwrap_or_default()
        );
        if !self.seen.insert(key) {
            return;
        }
        let finding = PublicationReadinessFinding {
            code: code.into(),
            message: message.into(),
            binding_id: binding_id.map(str::to_string),
            source_kind,
            source_id: source_id.map(str::to_string),
            waivable,
            waived: false,
            waiver: None,
            details,
        };
        match bucket {
            FindingBucket::Blocker => self.blockers.push(finding),
            FindingBucket::Warning => self.warnings.push(finding),
            FindingBucket::Omission => self.omissions.push(finding),
        }
    }

    fn apply_waivers(&mut self, waivers: &[PublicationWaiver]) {
        let by_code = waivers
            .iter()
            .map(|waiver| (waiver.finding_code.as_str(), waiver))
            .collect::<BTreeMap<_, _>>();
        for finding in self
            .blockers
            .iter_mut()
            .chain(self.warnings.iter_mut())
            .chain(self.omissions.iter_mut())
        {
            if finding.waivable {
                if let Some(waiver) = by_code.get(finding.code.as_str()) {
                    finding.waived = true;
                    finding.waiver = Some((*waiver).clone());
                }
            }
        }
        let sort = |findings: &mut Vec<PublicationReadinessFinding>| {
            findings.sort_by(|left, right| {
                (
                    left.code.as_str(),
                    left.binding_id.as_deref().unwrap_or_default(),
                    left.source_id.as_deref().unwrap_or_default(),
                )
                    .cmp(&(
                        right.code.as_str(),
                        right.binding_id.as_deref().unwrap_or_default(),
                        right.source_id.as_deref().unwrap_or_default(),
                    ))
            });
        };
        sort(&mut self.blockers);
        sort(&mut self.warnings);
        sort(&mut self.omissions);
    }
}

#[derive(Default)]
struct SecurityFlags {
    secret_kinds: Vec<&'static str>,
    machine_path: bool,
    internal_network: bool,
    potential_pii: bool,
}

fn secret_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?ix)
            -----BEGIN\x20(?:RSA\x20|EC\x20|OPENSSH\x20)?PRIVATE\x20KEY-----
            | \bAKIA[0-9A-Z]{16}\b
            | \bsk-[A-Za-z0-9_-]{16,}\b
            | \bBearer\x20+[A-Za-z0-9._~+/=-]{12,}
            | \b(?:api[_-]?key|access[_-]?token|password|passwd|client[_-]?secret)
              \s*[:=]\s*["']?[A-Za-z0-9._~+/=-]{8,}
            | ://[^/\s:@]+:[^/\s@]+@
            "#,
        )
        .expect("valid secret regex")
    })
}

fn private_ip_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid private IP regex"))
}

fn absolute_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?x)
            (?:^|[\s"'=(:,])/[A-Za-z0-9._-]+(?:/[^\s"'<>|/]+)+
            | \b[A-Za-z]:[\\/][^\s"']+
            | (?:^|[\s"'=])\\\\[^\s\\]+\\[^\s\\]+
            "#,
        )
        .expect("valid absolute path regex")
    })
}

fn pii_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?ix)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b
             |\b\d{3}-\d{2}-\d{4}\b
             |\b(?:patient[_ -]?id|medical[_ -]?record|mrn|date[_ -]?of[_ -]?birth)\b",
        )
        .expect("valid PII regex")
    })
}

fn scan_security(text: &str) -> SecurityFlags {
    let mut flags = SecurityFlags::default();
    if secret_regex().is_match(text) {
        flags.secret_kinds.push("credential_pattern");
    }
    let lower = text.to_ascii_lowercase();
    flags.machine_path = absolute_path_regex().is_match(text)
        || looks_absolute_path(text)
        || lower.contains("/home/")
        || lower.contains("/users/")
        || lower.contains("\\users\\")
        || lower.contains("ssh://")
        || lower.contains("known_hosts");
    flags.internal_network = lower.contains("localhost")
        || lower.contains(".internal")
        || lower.contains(".local")
        || private_ip_regex().find_iter(text).any(|value| {
            value
                .as_str()
                .parse::<std::net::Ipv4Addr>()
                .is_ok_and(|address| {
                    address.is_private() || address.is_loopback() || address.is_link_local()
                })
        });
    flags.potential_pii = pii_regex().is_match(text);
    flags
}

pub(crate) fn capsule_security_violations(text: &str, public: bool) -> Vec<String> {
    let flags = scan_security(text);
    let mut violations = flags
        .secret_kinds
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if public && flags.machine_path {
        violations.push("machine_local_detail".into());
    }
    if public && flags.internal_network {
        violations.push("internal_network_detail".into());
    }
    violations.sort();
    violations.dedup();
    violations
}

fn looks_absolute_path(text: &str) -> bool {
    text.starts_with('/')
        || text.starts_with("\\\\")
        || (text.len() >= 3
            && text.as_bytes()[0].is_ascii_alphabetic()
            && text.as_bytes()[1] == b':'
            && matches!(text.as_bytes()[2], b'\\' | b'/'))
}

fn visibility_rank(visibility: EvidenceVisibility) -> u8 {
    match visibility {
        EvidenceVisibility::Public => 0,
        EvidenceVisibility::Restricted => 1,
        EvidenceVisibility::Private => 2,
    }
}

fn most_restrictive(left: EvidenceVisibility, right: EvidenceVisibility) -> EvidenceVisibility {
    if visibility_rank(left) >= visibility_rank(right) {
        left
    } else {
        right
    }
}

fn visibility_allows(target: EvidenceVisibility, source: EvidenceVisibility) -> bool {
    visibility_rank(source) <= visibility_rank(target)
}

fn anchored_snapshot(snapshot_json: &str) -> Option<Value> {
    let Ok(snapshot) = serde_json::from_str::<Value>(snapshot_json) else {
        return None;
    };
    if canonical_json(&snapshot) != snapshot_json {
        return None;
    }
    let anchor = snapshot.get("anchor")?;
    let expected = snapshot.get("anchor_sha256").and_then(Value::as_str)?;
    (canonical_json_sha256(anchor).1 == expected).then(|| anchor.clone())
}

fn valid_anchored_snapshot(snapshot_json: &str) -> bool {
    anchored_snapshot(snapshot_json).is_some()
}

fn safe_component(value: &str) -> String {
    let mut result = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while result.contains("--") {
        result = result.replace("--", "-");
    }
    result.trim_matches(['.', '-']).chars().take(120).collect()
}

fn safe_relative_path(path: &str) -> Option<String> {
    if path.trim().is_empty() || looks_absolute_path(path) {
        return None;
    }
    let path = Path::new(path);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(path.to_string_lossy().replace('\\', "/"))
}

fn artifact_path(project_root: &Path, storage_path: &str) -> Result<PathBuf, String> {
    if looks_absolute_path(storage_path) && !Path::new(storage_path).is_absolute() {
        return Err("artifact path uses a foreign platform absolute path".into());
    }
    let path = PathBuf::from(storage_path);
    Ok(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

fn portable_project_path(project_root: &Path, path: &Path) -> Option<String> {
    let root = dunce::canonicalize(project_root).ok()?;
    let path = dunce::canonicalize(path).ok()?;
    path.strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn late_capture_id(revision_id: &str, old_version_id: &str, checksum: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(revision_id.as_bytes());
    digest.update([0]);
    digest.update(old_version_id.as_bytes());
    digest.update([0]);
    digest.update(checksum.as_bytes());
    format!("publication-late-{}", &hex::encode(digest.finalize())[..32])
}

fn is_text_like(content_type: &str, filename: &str) -> bool {
    content_type.starts_with("text/")
        || matches!(
            content_type,
            "application/json"
                | "application/xml"
                | "application/x-yaml"
                | "application/javascript"
                | "application/sql"
        )
        || Path::new(filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "csv"
                        | "tsv"
                        | "txt"
                        | "md"
                        | "json"
                        | "yaml"
                        | "yml"
                        | "toml"
                        | "py"
                        | "r"
                        | "rs"
                        | "js"
                        | "ts"
                        | "sh"
                )
            })
}

fn executable_binary(filename: &str, content_type: &str) -> bool {
    matches!(
        content_type,
        "application/x-executable"
            | "application/x-msdownload"
            | "application/x-sharedlib"
            | "application/vnd.microsoft.portable-executable"
    ) || Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "dll" | "dylib" | "so"
            )
        })
}

fn read_text_for_scan(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_TEXT_SCAN_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_TEXT_SCAN_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[allow(clippy::too_many_arguments)]
fn add_security_findings(
    findings: &mut Findings,
    flags: &SecurityFlags,
    label: &str,
    binding_id: Option<&str>,
    source_kind: Option<EvidenceSourceKind>,
    source_id: Option<&str>,
    target_visibility: EvidenceVisibility,
) -> bool {
    let mut public_content_allowed = true;
    if !flags.secret_kinds.is_empty() {
        public_content_allowed = false;
        findings.add(
            FindingBucket::Blocker,
            "potential_secret",
            format!("{label} contains a credential-like pattern"),
            binding_id,
            source_kind,
            source_id,
            false,
            json!({"patterns": flags.secret_kinds}),
        );
    }
    if flags.machine_path {
        if target_visibility == EvidenceVisibility::Public {
            public_content_allowed = false;
        }
        findings.add(
            FindingBucket::Warning,
            "machine_local_detail",
            format!("{label} contains a machine-local path or SSH detail"),
            binding_id,
            source_kind,
            source_id,
            true,
            json!({"public_content_allowed": public_content_allowed}),
        );
    }
    if flags.internal_network {
        if target_visibility == EvidenceVisibility::Public {
            public_content_allowed = false;
        }
        findings.add(
            FindingBucket::Warning,
            "internal_network_detail",
            format!("{label} contains a private-network or local-host reference"),
            binding_id,
            source_kind,
            source_id,
            true,
            json!({"public_content_allowed": public_content_allowed}),
        );
    }
    if flags.potential_pii {
        findings.add(
            FindingBucket::Warning,
            "potential_phi_pii",
            format!("{label} contains a potential personal or clinical identifier"),
            binding_id,
            source_kind,
            source_id,
            true,
            json!({}),
        );
    }
    public_content_allowed
}

#[derive(Default)]
struct CapabilityState {
    traceable: bool,
    re_executable: bool,
    has_run: bool,
    reproduced: bool,
}

impl CapabilityState {
    fn new(selected: &[EvidenceBinding]) -> Self {
        Self {
            traceable: !selected.is_empty(),
            re_executable: !selected.is_empty(),
            has_run: false,
            reproduced: !selected.is_empty()
                && selected
                    .iter()
                    .all(|binding| binding.reproduction_state == EvidenceReproductionState::Passed),
        }
    }

    fn level(&self) -> PublicationCapabilityLevel {
        if !self.traceable {
            PublicationCapabilityLevel::Archived
        } else if !self.re_executable || !self.has_run {
            PublicationCapabilityLevel::Traceable
        } else if self.reproduced {
            PublicationCapabilityLevel::Reproduced
        } else {
            PublicationCapabilityLevel::ReExecutable
        }
    }
}

#[derive(Default)]
struct DirectArtifactGroup {
    binding_ids: Vec<String>,
    visibility: Option<EvidenceVisibility>,
    snapshot_bytes: bool,
}

#[tauri::command]
pub(crate) async fn freeze_publication_revision(
    state: tauri::State<'_, crate::AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    revision_id: String,
    policy: PublicationFreezePolicy,
) -> Result<PublicationFreezeOutcome, String> {
    crate::publication_commands::validate_publication_revision(
        &state,
        window.label(),
        &revision_id,
    )
    .await?;
    freeze_publication_revision_in_store(&state.store, &revision_id, policy).await
}

#[tauri::command]
pub(crate) async fn check_publication_revision(
    state: tauri::State<'_, crate::AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
    revision_id: String,
    policy: PublicationFreezePolicy,
) -> Result<PublicationFreezeOutcome, String> {
    crate::publication_commands::validate_publication_revision(
        &state,
        window.label(),
        &revision_id,
    )
    .await?;
    prepare_or_freeze_publication(&state.store, &revision_id, policy, true).await
}

pub(crate) async fn freeze_publication_revision_in_store(
    store: &Store,
    revision_id: &str,
    policy: PublicationFreezePolicy,
) -> Result<PublicationFreezeOutcome, String> {
    prepare_or_freeze_publication(store, revision_id, policy, false).await
}

pub(crate) async fn prepare_or_freeze_publication(
    store: &Store,
    revision_id: &str,
    policy: PublicationFreezePolicy,
    check_only: bool,
) -> Result<PublicationFreezeOutcome, String> {
    let attempt_id = uuid::Uuid::new_v4().to_string();
    store
        .begin_publication_freeze(revision_id, &attempt_id, &policy)
        .await
        .map_err(|error| error.to_string())?;

    let prepared = match prepare_publication_freeze(store, revision_id, &attempt_id, &policy).await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = store
                .abort_publication_freeze(revision_id, &attempt_id)
                .await;
            return Err(error);
        }
    };
    if check_only || !prepared.readiness.can_freeze {
        store
            .abort_publication_freeze(revision_id, &attempt_id)
            .await
            .map_err(|error| error.to_string())?;
        let revision = store
            .get_publication_revision(revision_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Publication revision disappeared after readiness check".to_string())?;
        return Ok(PublicationFreezeOutcome {
            frozen: false,
            revision,
            readiness: prepared.readiness,
        });
    }

    match store.commit_publication_freeze(&prepared.commit).await {
        Ok(revision) => Ok(PublicationFreezeOutcome {
            frozen: true,
            revision,
            readiness: prepared.readiness,
        }),
        Err(error) => {
            let _ = store
                .abort_publication_freeze(revision_id, &attempt_id)
                .await;
            Err(error.to_string())
        }
    }
}

async fn prepare_publication_freeze(
    store: &Store,
    revision_id: &str,
    attempt_id: &str,
    policy: &PublicationFreezePolicy,
) -> Result<PreparedFreeze, String> {
    let revision = store
        .get_publication_revision(revision_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Publication revision not found".to_string())?;
    if revision.state != PublicationRevisionState::Freezing {
        return Err("Publication revision is not held by a freeze attempt".into());
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
    let project_root = (!workspace_dir.trim().is_empty()).then(|| PathBuf::from(&workspace_dir));

    let items = store
        .list_publication_items(revision_id)
        .await
        .map_err(|error| error.to_string())?;
    let item_by_id = items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let all_bindings = store
        .list_evidence_bindings(revision_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut selected = all_bindings
        .iter()
        .filter(|binding| binding.selection_state == EvidenceSelectionState::Selected)
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.id.cmp(&right.id));
    let waivers = store
        .list_publication_waivers(revision_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut findings = Findings::default();
    let mut capability = CapabilityState::new(&selected);

    if selected.is_empty() {
        findings.add(
            FindingBucket::Blocker,
            "no_selected_evidence",
            "The revision has no selected evidence",
            None,
            None,
            None,
            false,
            json!({}),
        );
    }
    if policy.target_visibility == EvidenceVisibility::Public {
        if !policy.phi_pii_reviewed {
            findings.add(
                FindingBucket::Blocker,
                "sensitive_data_review_required",
                "Public freeze requires explicit PHI/PII and human-subject review",
                None,
                None,
                None,
                false,
                json!({}),
            );
        }
        if !policy.redistribution_reviewed {
            findings.add(
                FindingBucket::Blocker,
                "redistribution_review_required",
                "Public freeze requires explicit license and redistribution review",
                None,
                None,
                None,
                false,
                json!({}),
            );
        }
    }
    for item in items.iter().filter(|item| {
        matches!(
            item.kind,
            PublicationItemKind::Figure | PublicationItemKind::Table
        )
    }) {
        if !selected
            .iter()
            .any(|binding| binding.item_id.as_deref() == Some(item.id.as_str()))
        {
            capability.traceable = false;
            capability.re_executable = false;
            findings.add(
                FindingBucket::Blocker,
                "publication_item_missing_evidence",
                format!(
                    "{} '{}' has no selected evidence",
                    item.kind.as_str(),
                    item.title
                ),
                None,
                None,
                Some(&item.id),
                true,
                json!({"item_id": item.id, "item_kind": item.kind.as_str()}),
            );
        }
    }
    let mut version_visibility = BTreeMap::<String, EvidenceVisibility>::new();
    for binding in &all_bindings {
        if binding.source_kind == EvidenceSourceKind::ArtifactVersion {
            version_visibility
                .entry(binding.source_id.clone())
                .and_modify(|visibility| {
                    *visibility = most_restrictive(*visibility, binding.visibility)
                })
                .or_insert(binding.visibility);
        }
    }

    let mut direct_groups = BTreeMap::<String, DirectArtifactGroup>::new();
    for binding in selected
        .iter()
        .filter(|binding| binding.source_kind == EvidenceSourceKind::ArtifactVersion)
    {
        let group = direct_groups.entry(binding.source_id.clone()).or_default();
        group.binding_ids.push(binding.id.clone());
        group.visibility = Some(match group.visibility {
            Some(existing) => most_restrictive(existing, binding.visibility),
            None => binding.visibility,
        });
        group.snapshot_bytes |=
            binding.visibility == EvidenceVisibility::Public || policy.snapshot_restricted_bytes;
    }

    let mut artifacts = BTreeMap::<String, ResolvedArtifact>::new();
    let mut local_paths = BTreeMap::<String, PathBuf>::new();
    let mut replacements = BTreeMap::<String, String>::new();
    let mut replacement_snapshots = BTreeMap::<String, String>::new();
    let mut late_captures = Vec::<PublicationLateCapture>::new();

    for (old_version_id, group) in &direct_groups {
        let context = store
            .get_artifact_version_context(old_version_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("ArtifactVersion '{old_version_id}' no longer exists"))?;
        if context.project_id != publication.project_id {
            return Err("Evidence ArtifactVersion changed project ownership".into());
        }
        let old = context.version.clone();
        if old.materialization == ArtifactMaterialization::External {
            if old.checksum.is_none() || old.size_bytes.is_none() {
                capability.traceable = false;
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "external_artifact_identity_incomplete",
                    "External ArtifactVersion lacks checksum or size",
                    group.binding_ids.first().map(String::as_str),
                    Some(EvidenceSourceKind::ArtifactVersion),
                    Some(old_version_id),
                    true,
                    json!({}),
                );
            }
            artifacts.insert(
                old_version_id.clone(),
                ResolvedArtifact::from_context(context),
            );
            continue;
        }
        let Some(root) = project_root.as_deref() else {
            capability.traceable = false;
            capability.re_executable = false;
            findings.add(
                FindingBucket::Blocker,
                "project_workspace_missing",
                "Local evidence cannot be frozen without a project workspace",
                group.binding_ids.first().map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(old_version_id),
                false,
                json!({}),
            );
            artifacts.insert(
                old_version_id.clone(),
                ResolvedArtifact::from_context(context),
            );
            continue;
        };
        let path = match artifact_path(root, &old.storage_path) {
            Ok(path) => path,
            Err(error) => {
                capability.traceable = false;
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "unsafe_artifact_path",
                    error,
                    group.binding_ids.first().map(String::as_str),
                    Some(EvidenceSourceKind::ArtifactVersion),
                    Some(old_version_id),
                    false,
                    json!({"storage_path": old.storage_path}),
                );
                artifacts.insert(
                    old_version_id.clone(),
                    ResolvedArtifact::from_context(context),
                );
                continue;
            }
        };
        let needs_late_capture = old.checksum.is_none()
            || old.size_bytes.is_none()
            || old.capture_timing == ArtifactCaptureTiming::Unknown;
        if needs_late_capture {
            let capture_policy = if group.snapshot_bytes {
                SnapshotPolicy::UpTo(DEFAULT_SNAPSHOT_LIMIT)
            } else {
                SnapshotPolicy::Reference
            };
            let captured = match capture_file(root, &path, capture_policy) {
                Ok(captured) => captured,
                Err(error) => {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "late_capture_failed",
                        format!("Historical evidence could not be captured safely: {error}"),
                        group.binding_ids.first().map(String::as_str),
                        Some(EvidenceSourceKind::ArtifactVersion),
                        Some(old_version_id),
                        false,
                        json!({}),
                    );
                    artifacts.insert(
                        old_version_id.clone(),
                        ResolvedArtifact::from_context(context),
                    );
                    continue;
                }
            };
            let size_bytes = i64::try_from(captured.size_bytes)
                .map_err(|_| "Artifact exceeds the supported metadata size range".to_string())?;
            let new_version_id = late_capture_id(revision_id, old_version_id, &captured.checksum);
            let latest_number = match context.latest_version_id.as_deref() {
                Some(latest_id) => {
                    store
                        .get_artifact_version(latest_id)
                        .await
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| "Artifact latest version disappeared".to_string())?
                        .version_number
                }
                None => 0,
            };
            let source_snapshot_json = canonical_json(&json!({
                "artifact_id": context.version.artifact_id,
                "capture_timing": "late",
                "checksum": captured.checksum,
                "content_type": context.version.content_type,
                "filename": context.filename,
                "historical_content_unverified": true,
                "materialization": captured.materialization.as_str(),
                "original_source_id": old_version_id,
                "size_bytes": size_bytes,
                "source_id": new_version_id,
                "source_kind": "artifact_version",
                "version_number": latest_number + 1,
            }));
            let resolved = ResolvedArtifact {
                id: new_version_id.clone(),
                artifact_id: context.version.artifact_id.clone(),
                version_number: latest_number + 1,
                filename: context.filename.clone(),
                content_type: context.version.content_type.clone(),
                storage_path: captured.storage_path.clone(),
                size_bytes: Some(size_bytes),
                checksum: Some(captured.checksum.clone()),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: captured.materialization,
                capture_timing: ArtifactCaptureTiming::Late,
                logical_key: context.logical_key.clone(),
            };
            capability.traceable = false;
            capability.re_executable = false;
            findings.add(
                FindingBucket::Warning,
                "historical_content_unverified",
                "Historical bytes were unavailable; evidence was captured at freeze time",
                group.binding_ids.first().map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(&new_version_id),
                true,
                json!({"original_source_id": old_version_id}),
            );
            local_paths.insert(new_version_id.clone(), root.join(&captured.storage_path));
            artifacts.insert(new_version_id.clone(), resolved);
            replacements.insert(old_version_id.clone(), new_version_id.clone());
            replacement_snapshots.insert(old_version_id.clone(), source_snapshot_json.clone());
            late_captures.push(PublicationLateCapture {
                binding_ids: group.binding_ids.clone(),
                old_version_id: old_version_id.clone(),
                new_version_id,
                artifact_id: context.version.artifact_id,
                expected_latest_version_id: context.latest_version_id,
                version_number: latest_number + 1,
                content_type: context.version.content_type,
                storage_path: captured.storage_path,
                size_bytes,
                checksum: captured.checksum,
                materialization: captured.materialization,
                source_snapshot_json,
            });
        } else {
            match capture_file(root, &path, SnapshotPolicy::Reference) {
                Ok(captured)
                    if i64::try_from(captured.size_bytes).ok() == old.size_bytes
                        && Some(captured.checksum.as_str()) == old.checksum.as_deref() =>
                {
                    local_paths.insert(old_version_id.clone(), path);
                }
                Ok(_) => {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "snapshot_checksum_mismatch",
                        "Artifact bytes no longer match the exact ArtifactVersion",
                        group.binding_ids.first().map(String::as_str),
                        Some(EvidenceSourceKind::ArtifactVersion),
                        Some(old_version_id),
                        false,
                        json!({}),
                    );
                }
                Err(error) => {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "snapshot_unavailable",
                        format!("ArtifactVersion could not be verified safely: {error}"),
                        group.binding_ids.first().map(String::as_str),
                        Some(EvidenceSourceKind::ArtifactVersion),
                        Some(old_version_id),
                        false,
                        json!({}),
                    );
                }
            }
            artifacts.insert(
                old_version_id.clone(),
                ResolvedArtifact::from_context(context),
            );
        }
    }

    let mut artifact_queue = VecDeque::<String>::new();
    let mut run_queue = VecDeque::<String>::new();
    let mut direct_external_bindings = BTreeMap::<String, Vec<&EvidenceBinding>>::new();
    let mut artifact_roles = BTreeMap::<String, BTreeSet<String>>::new();
    for binding in &selected {
        match binding.source_kind {
            EvidenceSourceKind::ArtifactVersion => {
                let source_id = replacements
                    .get(&binding.source_id)
                    .unwrap_or(&binding.source_id)
                    .clone();
                artifact_queue.push_back(source_id.clone());
                artifact_roles
                    .entry(source_id)
                    .or_default()
                    .insert("evidence".into());
            }
            EvidenceSourceKind::Run => run_queue.push_back(binding.source_id.clone()),
            EvidenceSourceKind::ExternalResource => {
                if valid_anchored_snapshot(&binding.source_snapshot_json) {
                    direct_external_bindings
                        .entry(binding.source_id.clone())
                        .or_default()
                        .push(binding);
                } else {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "evidence_anchor_hash_invalid",
                        "ExternalResource evidence anchor is not canonical or hash-valid",
                        Some(&binding.id),
                        Some(binding.source_kind),
                        Some(&binding.source_id),
                        false,
                        json!({}),
                    );
                }
            }
            EvidenceSourceKind::ExecutionLog
            | EvidenceSourceKind::MessageSpan
            | EvidenceSourceKind::ToolCall
            | EvidenceSourceKind::CodeCell => {
                if valid_anchored_snapshot(&binding.source_snapshot_json) {
                    continue;
                }
                capability.traceable = false;
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "evidence_anchor_hash_invalid",
                    "Fine-grained evidence anchor is not canonical or hash-valid",
                    Some(&binding.id),
                    Some(binding.source_kind),
                    Some(&binding.source_id),
                    false,
                    json!({}),
                );
            }
        }
    }

    let mut runs = BTreeMap::<String, RunRecord>::new();
    let mut inputs = BTreeMap::<String, RunInput>::new();
    let mut outputs = BTreeMap::<String, RunOutput>::new();
    let mut code = BTreeMap::<String, RunCodeSnapshot>::new();
    let mut environments = BTreeMap::<String, wisp_store::EnvironmentSnapshot>::new();
    let mut external_resources = BTreeMap::<String, ExternalResource>::new();

    for (resource_id, bindings) in direct_external_bindings {
        let Some(resource) = store
            .get_external_resource(&resource_id)
            .await
            .map_err(|error| error.to_string())?
            .filter(|resource| resource.project_id == publication.project_id)
        else {
            capability.traceable = false;
            capability.re_executable = false;
            findings.add(
                FindingBucket::Blocker,
                "external_resource_missing",
                "Evidence refers to a missing or foreign ExternalResource",
                None,
                Some(EvidenceSourceKind::ExternalResource),
                Some(&resource_id),
                false,
                json!({}),
            );
            continue;
        };
        let current_anchor = json!({
            "access_instructions": resource.access_instructions.as_deref(),
            "accessed_at": resource.accessed_at,
            "checksum": resource.checksum.as_deref(),
            "created_at": resource.created_at,
            "kind": resource.kind.as_str(),
            "license": resource.license.as_deref(),
            "size_bytes": resource.size_bytes,
            "source_id": resource.id.as_str(),
            "source_kind": "external_resource",
            "updated_at": resource.updated_at,
            "uri": resource.uri.as_str(),
            "version": resource.version.as_deref(),
            "visibility": resource.visibility.as_str(),
        });
        for binding in bindings {
            if anchored_snapshot(&binding.source_snapshot_json).as_ref() != Some(&current_anchor) {
                capability.traceable = false;
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "external_resource_anchor_drift",
                    "ExternalResource metadata changed after it was selected",
                    Some(&binding.id),
                    Some(EvidenceSourceKind::ExternalResource),
                    Some(&resource_id),
                    false,
                    json!({}),
                );
            }
        }
        if resource.checksum.is_none() || resource.version.is_none() {
            capability.traceable = false;
            capability.re_executable = false;
            findings.add(
                FindingBucket::Blocker,
                "external_resource_identity_incomplete",
                "ExternalResource requires version and checksum for exact evidence",
                None,
                Some(EvidenceSourceKind::ExternalResource),
                Some(&resource_id),
                true,
                json!({}),
            );
        }
        if resource.license.as_deref().is_none_or(str::is_empty) {
            findings.add(
                FindingBucket::Blocker,
                "external_resource_license_missing",
                "ExternalResource license is missing",
                None,
                Some(EvidenceSourceKind::ExternalResource),
                Some(&resource_id),
                true,
                json!({}),
            );
        }
        let resource_visibility = match resource.visibility.as_str() {
            "public" => EvidenceVisibility::Public,
            "restricted" => EvidenceVisibility::Restricted,
            "private" => EvidenceVisibility::Private,
            _ => {
                capability.traceable = false;
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "external_resource_visibility_invalid",
                    "ExternalResource visibility is invalid",
                    None,
                    Some(EvidenceSourceKind::ExternalResource),
                    Some(&resource_id),
                    false,
                    json!({}),
                );
                EvidenceVisibility::Private
            }
        };
        if !visibility_allows(policy.target_visibility, resource_visibility) {
            findings.add(
                FindingBucket::Omission,
                "visibility_dependency_omitted",
                "Restricted ExternalResource bytes are omitted from this target",
                None,
                Some(EvidenceSourceKind::ExternalResource),
                Some(&resource_id),
                false,
                json!({"visibility": resource.visibility}),
            );
            if resource
                .access_instructions
                .as_deref()
                .is_none_or(str::is_empty)
            {
                findings.add(
                    FindingBucket::Blocker,
                    "restricted_dependency_access_missing",
                    "Omitted ExternalResource lacks access instructions",
                    None,
                    Some(EvidenceSourceKind::ExternalResource),
                    Some(&resource_id),
                    true,
                    json!({}),
                );
            }
        }
        let flags = scan_security(&resource.uri);
        add_security_findings(
            &mut findings,
            &flags,
            "ExternalResource URI",
            None,
            Some(EvidenceSourceKind::ExternalResource),
            Some(&resource_id),
            policy.target_visibility,
        );
        external_resources.insert(resource.id.clone(), resource);
    }

    loop {
        if let Some(version_id) = artifact_queue.pop_front() {
            if !artifacts.contains_key(&version_id) {
                let context = match store
                    .get_artifact_version_context(&version_id)
                    .await
                    .map_err(|error| error.to_string())?
                {
                    Some(context) if context.project_id == publication.project_id => context,
                    _ => {
                        capability.traceable = false;
                        capability.re_executable = false;
                        findings.add(
                            FindingBucket::Blocker,
                            "artifact_dependency_missing",
                            "Run lineage references a missing or foreign ArtifactVersion",
                            None,
                            Some(EvidenceSourceKind::ArtifactVersion),
                            Some(&version_id),
                            true,
                            json!({}),
                        );
                        continue;
                    }
                };
                let version = &context.version;
                if version.materialization != ArtifactMaterialization::External {
                    if version.checksum.is_none()
                        || version.size_bytes.is_none()
                        || version.capture_timing == ArtifactCaptureTiming::Unknown
                    {
                        capability.traceable = false;
                        capability.re_executable = false;
                        findings.add(
                            FindingBucket::Blocker,
                            "dependency_historical_content_unverified",
                            "Run lineage contains historical local bytes that were never captured",
                            None,
                            Some(EvidenceSourceKind::ArtifactVersion),
                            Some(&version_id),
                            true,
                            json!({
                                "capture_timing": version.capture_timing.as_str(),
                                "materialization": version.materialization.as_str(),
                            }),
                        );
                    } else if let Some(root) = project_root.as_deref() {
                        match artifact_path(root, &version.storage_path).and_then(|path| {
                            capture_file(root, &path, SnapshotPolicy::Reference)
                                .map(|captured| (path, captured))
                        }) {
                            Ok((path, captured))
                                if i64::try_from(captured.size_bytes).ok()
                                    == version.size_bytes
                                    && Some(captured.checksum.as_str())
                                        == version.checksum.as_deref() =>
                            {
                                local_paths.insert(version_id.clone(), path);
                            }
                            Ok(_) => {
                                capability.traceable = false;
                                capability.re_executable = false;
                                findings.add(
                                    FindingBucket::Blocker,
                                    "dependency_checksum_mismatch",
                                    "A Run dependency no longer matches its ArtifactVersion",
                                    None,
                                    Some(EvidenceSourceKind::ArtifactVersion),
                                    Some(&version_id),
                                    false,
                                    json!({}),
                                );
                            }
                            Err(error) => {
                                capability.traceable = false;
                                capability.re_executable = false;
                                findings.add(
                                    FindingBucket::Blocker,
                                    "dependency_snapshot_unavailable",
                                    format!(
                                        "A Run dependency could not be verified safely: {error}"
                                    ),
                                    None,
                                    Some(EvidenceSourceKind::ArtifactVersion),
                                    Some(&version_id),
                                    false,
                                    json!({}),
                                );
                            }
                        }
                    } else {
                        capability.traceable = false;
                        capability.re_executable = false;
                        findings.add(
                            FindingBucket::Blocker,
                            "project_workspace_missing",
                            "Run ArtifactVersion dependencies require a project workspace",
                            None,
                            Some(EvidenceSourceKind::ArtifactVersion),
                            Some(&version_id),
                            false,
                            json!({}),
                        );
                    }
                } else if version.checksum.is_none() {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "external_artifact_identity_incomplete",
                        "External Run dependency lacks a checksum",
                        None,
                        Some(EvidenceSourceKind::ArtifactVersion),
                        Some(&version_id),
                        true,
                        json!({}),
                    );
                }
                artifacts.insert(version_id.clone(), ResolvedArtifact::from_context(context));
            }
            if let Some(run_id) = artifacts
                .get(&version_id)
                .and_then(|artifact| artifact.producing_run_id.clone())
            {
                run_queue.push_back(run_id);
            }
            continue;
        }

        if let Some(run_id) = run_queue.pop_front() {
            if runs.contains_key(&run_id) {
                continue;
            }
            let run = match store
                .get_run(&run_id)
                .await
                .map_err(|error| error.to_string())?
            {
                Some(run) if run.project_id == publication.project_id => run,
                _ => {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "producing_run_missing",
                        "Evidence refers to a missing or foreign Run",
                        None,
                        Some(EvidenceSourceKind::Run),
                        Some(&run_id),
                        true,
                        json!({}),
                    );
                    continue;
                }
            };
            capability.has_run = true;
            if run.status != RunStatus::Succeeded {
                capability.traceable = false;
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "run_not_succeeded",
                    format!("Run '{}' is {}", run.title, run.status.as_str()),
                    None,
                    Some(EvidenceSourceKind::Run),
                    Some(&run.id),
                    true,
                    json!({"status": run.status.as_str()}),
                );
            }
            if let Some(command) = run.command.as_deref() {
                let flags = scan_security(command);
                add_security_findings(
                    &mut findings,
                    &flags,
                    "Run command",
                    None,
                    Some(EvidenceSourceKind::Run),
                    Some(&run.id),
                    policy.target_visibility,
                );
            }

            let run_inputs = store
                .list_run_inputs(&run.id)
                .await
                .map_err(|error| error.to_string())?;
            for input in run_inputs {
                let has_exact_source =
                    input.artifact_version_id.is_some() || input.external_resource_id.is_some();
                if input.required
                    && (!has_exact_source
                        || input.confidence != LineageConfidence::Exact
                        || input.basis == LineageBasis::Inferred)
                {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "required_run_input_not_exact",
                        format!(
                            "Required Run input '{}' is not exact declared/observed lineage",
                            input.source_ref
                        ),
                        None,
                        Some(EvidenceSourceKind::Run),
                        Some(&run.id),
                        true,
                        json!({
                            "basis": input.basis.as_str(),
                            "confidence": input.confidence.as_str(),
                            "input_id": input.id,
                        }),
                    );
                } else if !input.required && !has_exact_source {
                    findings.add(
                        FindingBucket::Warning,
                        "optional_run_input_unresolved",
                        format!("Optional Run input '{}' is unresolved", input.source_ref),
                        None,
                        Some(EvidenceSourceKind::Run),
                        Some(&run.id),
                        true,
                        json!({"input_id": input.id}),
                    );
                }
                if let Some(version_id) = input.artifact_version_id.as_deref() {
                    artifact_queue.push_back(version_id.to_string());
                    artifact_roles
                        .entry(version_id.to_string())
                        .or_default()
                        .insert(format!("input:{}", input.role));
                    match version_visibility.get(version_id).copied() {
                        Some(visibility)
                            if !visibility_allows(policy.target_visibility, visibility) =>
                        {
                            findings.add(
                                FindingBucket::Omission,
                                "visibility_dependency_omitted",
                                "A dependency is more restricted than the freeze target",
                                None,
                                Some(EvidenceSourceKind::ArtifactVersion),
                                Some(version_id),
                                false,
                                json!({
                                    "dependency_visibility": visibility.as_str(),
                                    "role": input.role,
                                }),
                            );
                        }
                        None if policy.target_visibility != EvidenceVisibility::Private => {
                            capability.re_executable = false;
                            findings.add(
                                FindingBucket::Blocker,
                                "dependency_visibility_unclassified",
                                "Run input visibility has not been classified",
                                None,
                                Some(EvidenceSourceKind::ArtifactVersion),
                                Some(version_id),
                                true,
                                json!({"input_id": input.id, "role": input.role}),
                            );
                        }
                        _ => {}
                    }
                }
                if let Some(resource_id) = input.external_resource_id.as_deref() {
                    let resource = store
                        .get_external_resource(resource_id)
                        .await
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| {
                            format!("Run input ExternalResource '{resource_id}' disappeared")
                        })?;
                    let resource_visibility = match resource.visibility.as_str() {
                        "public" => EvidenceVisibility::Public,
                        "restricted" => EvidenceVisibility::Restricted,
                        "private" => EvidenceVisibility::Private,
                        _ => {
                            capability.traceable = false;
                            capability.re_executable = false;
                            findings.add(
                                FindingBucket::Blocker,
                                "external_resource_visibility_invalid",
                                "ExternalResource visibility is invalid",
                                None,
                                Some(EvidenceSourceKind::ExternalResource),
                                Some(resource_id),
                                false,
                                json!({}),
                            );
                            EvidenceVisibility::Private
                        }
                    };
                    if resource.checksum.is_none() || resource.version.is_none() {
                        capability.traceable = false;
                        capability.re_executable = false;
                        findings.add(
                            FindingBucket::Blocker,
                            "external_resource_identity_incomplete",
                            "ExternalResource requires version and checksum for exact lineage",
                            None,
                            Some(EvidenceSourceKind::ExternalResource),
                            Some(resource_id),
                            true,
                            json!({}),
                        );
                    }
                    if !visibility_allows(policy.target_visibility, resource_visibility) {
                        findings.add(
                            FindingBucket::Omission,
                            "visibility_dependency_omitted",
                            "Restricted ExternalResource bytes are omitted from this target",
                            None,
                            Some(EvidenceSourceKind::ExternalResource),
                            Some(resource_id),
                            false,
                            json!({
                                "dependency_visibility": resource.visibility,
                                "role": input.role,
                            }),
                        );
                        if resource
                            .access_instructions
                            .as_deref()
                            .is_none_or(str::is_empty)
                        {
                            findings.add(
                                FindingBucket::Blocker,
                                "restricted_dependency_access_missing",
                                "Omitted ExternalResource lacks access instructions",
                                None,
                                Some(EvidenceSourceKind::ExternalResource),
                                Some(resource_id),
                                true,
                                json!({}),
                            );
                        }
                    }
                    if resource.license.as_deref().is_none_or(str::is_empty) {
                        findings.add(
                            FindingBucket::Blocker,
                            "external_resource_license_missing",
                            "ExternalResource license is missing",
                            None,
                            Some(EvidenceSourceKind::ExternalResource),
                            Some(resource_id),
                            true,
                            json!({}),
                        );
                    }
                    let flags = scan_security(&resource.uri);
                    add_security_findings(
                        &mut findings,
                        &flags,
                        "ExternalResource URI",
                        None,
                        Some(EvidenceSourceKind::ExternalResource),
                        Some(resource_id),
                        policy.target_visibility,
                    );
                    external_resources.insert(resource.id.clone(), resource);
                }
                inputs.insert(input.id.clone(), input);
            }
            let run_outputs = store
                .list_run_outputs(&run.id)
                .await
                .map_err(|error| error.to_string())?;
            for output in run_outputs {
                artifact_queue.push_back(output.artifact_version_id.clone());
                artifact_roles
                    .entry(output.artifact_version_id.clone())
                    .or_default()
                    .insert(format!("output:{}", output.role));
                outputs.insert(output.id.clone(), output);
            }
            let run_code = store
                .list_run_code_snapshots(&run.id)
                .await
                .map_err(|error| error.to_string())?;
            if !run_code
                .iter()
                .any(|snapshot| !snapshot.source_text.trim().is_empty())
            {
                capability.re_executable = false;
                findings.add(
                    FindingBucket::Blocker,
                    "run_code_missing",
                    "Run has no immutable code snapshot",
                    None,
                    Some(EvidenceSourceKind::Run),
                    Some(&run.id),
                    true,
                    json!({}),
                );
            }
            for snapshot in run_code {
                let checksum = hex::encode(Sha256::digest(snapshot.source_text.as_bytes()));
                if checksum != snapshot.checksum {
                    capability.traceable = false;
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "run_code_checksum_mismatch",
                        "Run code snapshot checksum is invalid",
                        None,
                        Some(EvidenceSourceKind::Run),
                        Some(&run.id),
                        false,
                        json!({"code_snapshot_id": snapshot.id}),
                    );
                }
                code.insert(snapshot.id.clone(), snapshot);
            }
            match store
                .get_run_environment_snapshot(&run.id)
                .await
                .map_err(|error| error.to_string())?
            {
                Some(environment) => {
                    let parsed: Value = serde_json::from_str(&environment.snapshot_json)
                        .map_err(|_| "Run environment snapshot is invalid JSON".to_string())?;
                    let (_, hash) = canonical_json_sha256(&parsed);
                    if environment.hash_algorithm != "sha256" || hash != environment.hash {
                        capability.traceable = false;
                        capability.re_executable = false;
                        findings.add(
                            FindingBucket::Blocker,
                            "run_environment_hash_invalid",
                            "Run environment is not a valid canonical SHA-256 snapshot",
                            None,
                            Some(EvidenceSourceKind::Run),
                            Some(&run.id),
                            true,
                            json!({}),
                        );
                    }
                    let missing_fields = [
                        ("context.id", parsed.pointer("/context/id")),
                        ("context.kind", parsed.pointer("/context/kind")),
                        ("wisp_host.os", parsed.pointer("/wisp_host/os")),
                        ("wisp_host.arch", parsed.pointer("/wisp_host/arch")),
                    ]
                    .into_iter()
                    .filter_map(|(field, value)| {
                        value
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .is_none()
                            .then_some(field)
                    })
                    .collect::<Vec<_>>();
                    if !missing_fields.is_empty() {
                        capability.re_executable = false;
                        findings.add(
                            FindingBucket::Warning,
                            "run_environment_incomplete",
                            "Run environment lacks minimum context or host metadata",
                            None,
                            Some(EvidenceSourceKind::Run),
                            Some(&run.id),
                            true,
                            json!({"missing_fields": missing_fields}),
                        );
                    }
                    environments.insert(run.id.clone(), environment);
                }
                None => {
                    capability.re_executable = false;
                    findings.add(
                        FindingBucket::Blocker,
                        "run_environment_missing",
                        "Run has no environment snapshot",
                        None,
                        Some(EvidenceSourceKind::Run),
                        Some(&run.id),
                        true,
                        json!({}),
                    );
                }
            }
            runs.insert(run.id.clone(), run);
            continue;
        }
        break;
    }

    for binding in selected
        .iter()
        .filter(|binding| binding.source_kind == EvidenceSourceKind::ArtifactVersion)
    {
        let effective_id = replacements
            .get(&binding.source_id)
            .unwrap_or(&binding.source_id);
        let item_kind = binding
            .item_id
            .as_deref()
            .and_then(|item_id| item_by_id.get(item_id))
            .map(|item| item.kind);
        if matches!(
            item_kind,
            Some(PublicationItemKind::Figure | PublicationItemKind::Table)
        ) && artifacts
            .get(effective_id)
            .is_some_and(|artifact| artifact.producing_run_id.is_none())
        {
            capability.re_executable = false;
            findings.add(
                FindingBucket::Blocker,
                "producing_run_missing",
                "Figure/Table evidence has no exact producing Run",
                Some(&binding.id),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(effective_id),
                true,
                json!({}),
            );
        }
    }

    let mut item_content_allowed = BTreeMap::<String, bool>::new();
    for item in &items {
        let flags = scan_security(&format!(
            "{}\n{}\n{}",
            item.title, item.content, item.metadata_json
        ));
        let allowed = add_security_findings(
            &mut findings,
            &flags,
            "Publication item",
            None,
            None,
            Some(&item.id),
            policy.target_visibility,
        );
        item_content_allowed.insert(item.id.clone(), allowed);
    }

    let mut code_values = Vec::<Value>::new();
    for snapshot in code.values() {
        let combined = format!(
            "{}\n{}\n{}\n{}",
            snapshot.source_text,
            snapshot.dirty_patch.as_deref().unwrap_or_default(),
            snapshot.source_path.as_deref().unwrap_or_default(),
            snapshot.storage_path.as_deref().unwrap_or_default()
        );
        let flags = scan_security(&combined);
        let allowed = add_security_findings(
            &mut findings,
            &flags,
            "Run code snapshot",
            None,
            Some(EvidenceSourceKind::Run),
            Some(&snapshot.run_id),
            policy.target_visibility,
        );
        if !allowed && policy.target_visibility == EvidenceVisibility::Public {
            capability.re_executable = false;
            findings.add(
                FindingBucket::Omission,
                "sensitive_code_omitted",
                "Code text is omitted from the Public manifest",
                None,
                Some(EvidenceSourceKind::Run),
                Some(&snapshot.run_id),
                false,
                json!({"code_snapshot_id": snapshot.id}),
            );
        }
        let source_path = snapshot.source_path.as_deref().and_then(|path| {
            safe_relative_path(path).or_else(|| {
                project_root
                    .as_deref()
                    .and_then(|root| portable_project_path(root, Path::new(path)))
            })
        });
        let storage_path = snapshot.storage_path.as_deref().and_then(|path| {
            safe_relative_path(path).or_else(|| {
                project_root
                    .as_deref()
                    .and_then(|root| portable_project_path(root, Path::new(path)))
            })
        });
        code_values.push(json!({
            "checksum": snapshot.checksum,
            "content_included": allowed,
            "dirty_patch": allowed.then_some(snapshot.dirty_patch.as_deref()).flatten(),
            "git_commit": snapshot.git_commit,
            "id": snapshot.id,
            "run_id": snapshot.run_id,
            "source_kind": snapshot.source_kind,
            "source_path": source_path,
            "source_text": allowed.then_some(snapshot.source_text.as_str()),
            "storage_path": storage_path,
        }));
    }

    let mut environment_values = Vec::<Value>::new();
    for (run_id, environment) in &environments {
        let flags = scan_security(&environment.snapshot_json);
        let allowed = add_security_findings(
            &mut findings,
            &flags,
            "Run environment snapshot",
            None,
            Some(EvidenceSourceKind::Run),
            Some(run_id),
            policy.target_visibility,
        );
        if !allowed && policy.target_visibility == EvidenceVisibility::Public {
            capability.re_executable = false;
            findings.add(
                FindingBucket::Omission,
                "sensitive_environment_omitted",
                "Environment details are omitted from the Public manifest",
                None,
                Some(EvidenceSourceKind::Run),
                Some(run_id),
                false,
                json!({}),
            );
        }
        let snapshot: Value =
            serde_json::from_str(&environment.snapshot_json).unwrap_or_else(|_| json!({}));
        environment_values.push(json!({
            "env_name": environment.env_name,
            "hash": environment.hash,
            "hash_algorithm": environment.hash_algorithm,
            "run_id": run_id,
            "snapshot": allowed.then_some(snapshot),
        }));
    }

    let mut external_values = Vec::<Value>::new();
    for resource in external_resources.values() {
        let combined = format!(
            "{}\n{}",
            resource.uri,
            resource.access_instructions.as_deref().unwrap_or_default()
        );
        let flags = scan_security(&combined);
        let allowed = add_security_findings(
            &mut findings,
            &flags,
            "ExternalResource metadata",
            None,
            Some(EvidenceSourceKind::ExternalResource),
            Some(&resource.id),
            policy.target_visibility,
        );
        external_values.push(json!({
            "access_instructions": allowed.then_some(resource.access_instructions.as_deref()).flatten(),
            "checksum": resource.checksum,
            "id": resource.id,
            "kind": resource.kind,
            "license": resource.license,
            "size_bytes": resource.size_bytes,
            "uri": allowed.then_some(resource.uri.as_str()),
            "version": resource.version,
            "visibility": resource.visibility,
        }));
    }

    let mut direct_binding_by_version = BTreeMap::<String, String>::new();
    let mut direct_kind_by_version = BTreeMap::<String, PublicationItemKind>::new();
    let mut license_by_version = BTreeMap::<String, Option<String>>::new();
    for binding in selected
        .iter()
        .filter(|binding| binding.source_kind == EvidenceSourceKind::ArtifactVersion)
    {
        let version_id = replacements
            .get(&binding.source_id)
            .unwrap_or(&binding.source_id)
            .clone();
        direct_binding_by_version
            .entry(version_id.clone())
            .or_insert_with(|| binding.id.clone());
        if let Some(item) = binding
            .item_id
            .as_deref()
            .and_then(|item_id| item_by_id.get(item_id))
        {
            direct_kind_by_version
                .entry(version_id.clone())
                .or_insert(item.kind);
            let metadata: Value =
                serde_json::from_str(&item.metadata_json).unwrap_or_else(|_| json!({}));
            let license = metadata
                .get("license")
                .and_then(Value::as_str)
                .map(str::to_string);
            license_by_version.entry(version_id).or_insert(license);
        }
    }

    let mut file_values = Vec::<Value>::new();
    let mut file_inclusion = BTreeMap::<String, bool>::new();
    for artifact in artifacts.values_mut() {
        let visibility = version_visibility
            .get(&artifact.id)
            .copied()
            .or_else(|| {
                replacements.iter().find_map(|(old, new)| {
                    (new == &artifact.id)
                        .then(|| version_visibility.get(old).copied())
                        .flatten()
                })
            })
            .unwrap_or(EvidenceVisibility::Private);
        let visibility_allowed = visibility_allows(policy.target_visibility, visibility);
        if !visibility_allowed {
            findings.add(
                FindingBucket::Omission,
                "visibility_dependency_omitted",
                "Artifact bytes are more restricted than the freeze target",
                direct_binding_by_version
                    .get(&artifact.id)
                    .map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(&artifact.id),
                false,
                json!({"dependency_visibility": visibility.as_str()}),
            );
        }
        let mut content_allowed = visibility_allowed;
        if executable_binary(&artifact.filename, &artifact.content_type) {
            content_allowed = false;
            findings.add(
                FindingBucket::Omission,
                "executable_binary_omitted",
                "Executable binary bytes are not allowlisted for a capsule",
                direct_binding_by_version
                    .get(&artifact.id)
                    .map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(&artifact.id),
                false,
                json!({"content_type": artifact.content_type}),
            );
        }
        if artifact.content_type.trim().is_empty() {
            findings.add(
                FindingBucket::Warning,
                "file_type_missing",
                "Artifact content type is missing",
                direct_binding_by_version
                    .get(&artifact.id)
                    .map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(&artifact.id),
                true,
                json!({}),
            );
        }
        if policy.target_visibility == EvidenceVisibility::Public
            && artifact
                .size_bytes
                .is_some_and(|size| size > DEFAULT_SNAPSHOT_LIMIT as i64)
        {
            content_allowed = false;
            findings.add(
                FindingBucket::Omission,
                "large_file_reference",
                "Large artifact bytes remain a checksum reference in Public output",
                direct_binding_by_version
                    .get(&artifact.id)
                    .map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(&artifact.id),
                false,
                json!({"size_bytes": artifact.size_bytes}),
            );
        }
        if let Some(path) = local_paths.get(&artifact.id) {
            if let Some(portable) = project_root
                .as_deref()
                .and_then(|root| portable_project_path(root, path))
            {
                artifact.storage_path = portable;
            } else {
                content_allowed = false;
                findings.add(
                    FindingBucket::Blocker,
                    "unsafe_artifact_path",
                    "Artifact path cannot be represented relative to the project",
                    direct_binding_by_version
                        .get(&artifact.id)
                        .map(String::as_str),
                    Some(EvidenceSourceKind::ArtifactVersion),
                    Some(&artifact.id),
                    false,
                    json!({}),
                );
                artifact.storage_path = String::new();
            }
            if content_allowed && is_text_like(&artifact.content_type, &artifact.filename) {
                if let Some(text) = read_text_for_scan(path) {
                    let flags = scan_security(&text);
                    content_allowed &= add_security_findings(
                        &mut findings,
                        &flags,
                        "Artifact text",
                        direct_binding_by_version
                            .get(&artifact.id)
                            .map(String::as_str),
                        Some(EvidenceSourceKind::ArtifactVersion),
                        Some(&artifact.id),
                        policy.target_visibility,
                    );
                } else {
                    content_allowed = false;
                    findings.add(
                        FindingBucket::Omission,
                        "text_security_scan_omitted",
                        "Text bytes are omitted because the bounded security scan could not validate them",
                        direct_binding_by_version
                            .get(&artifact.id)
                            .map(String::as_str),
                        Some(EvidenceSourceKind::ArtifactVersion),
                        Some(&artifact.id),
                        false,
                        json!({"scan_limit_bytes": MAX_TEXT_SCAN_BYTES}),
                    );
                }
            }
        } else if artifact.materialization != ArtifactMaterialization::External {
            content_allowed = false;
            artifact.storage_path = safe_relative_path(&artifact.storage_path).unwrap_or_default();
        }
        let include_bytes =
            content_allowed && artifact.materialization == ArtifactMaterialization::Snapshot;
        if content_allowed && artifact.materialization != ArtifactMaterialization::Snapshot {
            findings.add(
                FindingBucket::Omission,
                "reference_bytes_omitted",
                "Reference-only ArtifactVersion contributes metadata but no capsule bytes",
                direct_binding_by_version
                    .get(&artifact.id)
                    .map(String::as_str),
                Some(EvidenceSourceKind::ArtifactVersion),
                Some(&artifact.id),
                false,
                json!({"materialization": artifact.materialization.as_str()}),
            );
        }
        let filename = {
            let safe = safe_component(&artifact.filename);
            if safe.is_empty() {
                "artifact".to_string()
            } else {
                safe
            }
        };
        let identity = safe_component(&artifact.id);
        let capsule_path = match direct_kind_by_version.get(&artifact.id) {
            Some(PublicationItemKind::Figure) => format!("figures/{identity}-{filename}"),
            Some(PublicationItemKind::Table) => format!("tables/{identity}-{filename}"),
            Some(_) => format!("evidence/{identity}-{filename}"),
            None if artifact_roles
                .get(&artifact.id)
                .is_some_and(|roles| roles.iter().any(|role| role.starts_with("input:"))) =>
            {
                format!("data/{identity}-{filename}")
            }
            None => format!("reference-results/{identity}-{filename}"),
        };
        file_inclusion.insert(artifact.id.clone(), include_bytes);
        file_values.push(json!({
            "capsule_path": capsule_path,
            "dependency_roles": artifact_roles.get(&artifact.id).cloned().unwrap_or_default(),
            "include_bytes": include_bytes,
            "license": license_by_version.get(&artifact.id).cloned().flatten(),
            "mime": artifact.content_type,
            "producing_run_id": artifact.producing_run_id,
            "sha256": artifact.checksum,
            "size_bytes": artifact.size_bytes,
            "source_id": artifact.id,
            "source_kind": "artifact_version",
            "storage_path": artifact.storage_path,
            "visibility": visibility.as_str(),
        }));
    }

    let mut verification_values = Vec::<Value>::new();
    for binding in &selected {
        let flags = scan_security(&format!(
            "{}\n{}",
            binding.purpose, binding.source_snapshot_json
        ));
        add_security_findings(
            &mut findings,
            &flags,
            "Evidence metadata",
            Some(&binding.id),
            Some(binding.source_kind),
            Some(&binding.source_id),
            policy.target_visibility,
        );
        let reviews = store
            .list_evidence_reviews(&binding.id)
            .await
            .map_err(|error| error.to_string())?;
        for review in reviews {
            let combined = format!(
                "{}\n{}\n{}\n{}\n{}",
                review.method,
                review.environment_json,
                review.comparator_json,
                review.tolerance_json,
                review.report_json
            );
            let flags = scan_security(&combined);
            let allowed = add_security_findings(
                &mut findings,
                &flags,
                "Evidence review",
                Some(&binding.id),
                Some(binding.source_kind),
                Some(&binding.source_id),
                policy.target_visibility,
            );
            verification_values.push(json!({
                "binding_id": binding.id,
                "comparator": allowed.then(|| serde_json::from_str::<Value>(&review.comparator_json).unwrap_or_else(|_| json!({}))),
                "environment": allowed.then(|| serde_json::from_str::<Value>(&review.environment_json).unwrap_or_else(|_| json!({}))),
                "method": review.method,
                "report": allowed.then(|| serde_json::from_str::<Value>(&review.report_json).unwrap_or_else(|_| json!({}))),
                "result": review.result,
                "review_id": review.id,
                "reviewer": review.reviewer,
                "tolerance": allowed.then(|| serde_json::from_str::<Value>(&review.tolerance_json).unwrap_or_else(|_| json!({}))),
                "verified_at": review.verified_at,
            }));
        }
    }

    let mut waiver_values = Vec::<Value>::new();
    for waiver in &waivers {
        let flags = scan_security(&format!("{}\n{}", waiver.author, waiver.reason));
        let allowed = add_security_findings(
            &mut findings,
            &flags,
            "Readiness waiver",
            None,
            None,
            Some(&waiver.id),
            policy.target_visibility,
        );
        waiver_values.push(json!({
            "author": waiver.author,
            "created_at": waiver.created_at,
            "finding_code": waiver.finding_code,
            "id": waiver.id,
            "reason": allowed.then_some(waiver.reason.as_str()),
        }));
    }

    findings.apply_waivers(&waivers);
    capability.re_executable &= capability.has_run;
    let capability_level = capability.level();
    let can_freeze = findings.blockers.iter().all(|finding| finding.waived);

    let mut item_values = items
        .iter()
        .map(|item| {
            let content_allowed = item_content_allowed
                .get(&item.id)
                .copied()
                .unwrap_or(true);
            json!({
                "content": content_allowed.then_some(item.content.as_str()),
                "id": item.id,
                "kind": item.kind.as_str(),
                "metadata": content_allowed.then(|| serde_json::from_str::<Value>(&item.metadata_json).unwrap_or_else(|_| json!({}))),
                "ordinal": item.ordinal,
                "parent_item_id": item.parent_item_id,
                "title": item.title,
            })
        })
        .collect::<Vec<_>>();
    item_values.sort_by(|left, right| {
        (
            left.get("parent_item_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            left.get("ordinal")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            left.get("id").and_then(Value::as_str).unwrap_or_default(),
        )
            .cmp(&(
                right
                    .get("parent_item_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                right
                    .get("ordinal")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
                right.get("id").and_then(Value::as_str).unwrap_or_default(),
            ))
    });
    let item_link_values = store
        .list_publication_item_links(revision_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|link| {
            json!({
                "id": link.id,
                "relation": link.relation,
                "source_item_id": link.source_item_id,
                "target_item_id": link.target_item_id,
            })
        })
        .collect::<Vec<_>>();
    let supersession_values = store
        .list_evidence_supersessions(revision_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|supersession| {
            json!({
                "id": supersession.id,
                "new_binding_id": supersession.new_binding_id,
                "old_binding_id": supersession.old_binding_id,
                "reason": supersession.reason,
            })
        })
        .collect::<Vec<_>>();

    let mut evidence_values = Vec::<Value>::new();
    for binding in &selected {
        let source_id = replacements
            .get(&binding.source_id)
            .unwrap_or(&binding.source_id);
        let source_snapshot_json = replacement_snapshots
            .get(&binding.source_id)
            .unwrap_or(&binding.source_snapshot_json);
        let source_snapshot: Value =
            serde_json::from_str(source_snapshot_json).unwrap_or_else(|_| json!({}));
        let item_kind = binding
            .item_id
            .as_deref()
            .and_then(|item_id| item_by_id.get(item_id))
            .map(|item| item.kind.as_str());
        evidence_values.push(json!({
            "binding_id": binding.id,
            "include_bytes": file_inclusion.get(source_id).copied().unwrap_or(false),
            "item_id": binding.item_id,
            "item_kind": item_kind,
            "purpose": binding.purpose,
            "reproduction_state": binding.reproduction_state.as_str(),
            "review_state": binding.review_state.as_str(),
            "selection_state": binding.selection_state.as_str(),
            "source_id": source_id,
            "source_kind": binding.source_kind.as_str(),
            "source_snapshot": source_snapshot,
            "supported_claim_item_id": binding.supported_claim_item_id,
            "visibility": binding.visibility.as_str(),
        }));
    }

    let run_values = runs
        .values()
        .map(|run| {
            let command_sha256 = run
                .command
                .as_deref()
                .map(|command| hex::encode(Sha256::digest(command.as_bytes())));
            let script_path = run.script_path.as_deref().and_then(|path| {
                safe_relative_path(path).or_else(|| {
                    project_root
                        .as_deref()
                        .and_then(|root| portable_project_path(root, Path::new(path)))
                })
            });
            json!({
                "command_sha256": command_sha256,
                "context_id": run.context_id,
                "id": run.id,
                "kind": run.kind,
                "script_path": script_path,
                "status": run.status.as_str(),
                "title": run.title,
            })
        })
        .collect::<Vec<_>>();
    let input_values = inputs
        .values()
        .map(|input| {
            let source = input
                .artifact_version_id
                .as_deref()
                .and_then(|id| artifacts.get(id))
                .map(ResolvedArtifact::manifest_value)
                .or_else(|| {
                    input
                        .external_resource_id
                        .as_deref()
                        .and_then(|id| external_resources.get(id))
                        .map(|resource| {
                            json!({
                                "checksum": resource.checksum,
                                "id": resource.id,
                                "kind": resource.kind,
                                "size_bytes": resource.size_bytes,
                                "source_kind": "external_resource",
                                "version": resource.version,
                                "visibility": resource.visibility,
                            })
                        })
                });
            json!({
                "basis": input.basis.as_str(),
                "confidence": input.confidence.as_str(),
                "id": input.id,
                "required": input.required,
                "role": input.role,
                "run_id": input.run_id,
                "source": source,
                "source_ref": input.source_ref,
            })
        })
        .collect::<Vec<_>>();
    let output_values = outputs
        .values()
        .map(|output| {
            json!({
                "artifact": artifacts.get(&output.artifact_version_id).map(ResolvedArtifact::manifest_value),
                "id": output.id,
                "logical_output_key": output.logical_output_key,
                "role": output.role,
                "run_id": output.run_id,
                "source_path": safe_relative_path(&output.source_path),
            })
        })
        .collect::<Vec<_>>();

    let policy_json =
        canonical_json(&serde_json::to_value(policy).map_err(|error| error.to_string())?);
    let manifest = json!({
        "blockers": findings.blockers,
        "capability_level": capability_level.as_str(),
        "code": code_values,
        "environments": environment_values,
        "evidence": evidence_values,
        "external_resources": external_values,
        "files": file_values,
        "inputs": input_values,
        "item_links": item_link_values,
        "items": item_values,
        "omissions": findings.omissions,
        "outputs": output_values,
        "policy": serde_json::from_str::<Value>(&policy_json).unwrap_or_else(|_| json!({})),
        "publication": {
            "description": publication.description,
            "id": publication.id,
            "title": publication.title,
        },
        "publication_revision_id": revision_id,
        "runs": run_values,
        "schema_version": 1,
        "supersessions": supersession_values,
        "target_visibility": policy.target_visibility.as_str(),
        "verification": verification_values,
        "waivers": waiver_values,
        "warnings": findings.warnings,
    });
    let (manifest_json, manifest_sha256) = canonical_json_sha256(&manifest);
    let readiness = PublicationReadiness {
        revision_id: revision_id.into(),
        target_visibility: policy.target_visibility,
        capability_level,
        blockers: findings.blockers,
        warnings: findings.warnings,
        omissions: findings.omissions,
        manifest_json,
        manifest_sha256,
        can_freeze,
    };
    late_captures.sort_by(|left, right| left.new_version_id.cmp(&right.new_version_id));
    let commit = PublicationFreezeCommit {
        revision_id: revision_id.into(),
        attempt_id: attempt_id.into(),
        policy_json,
        readiness: readiness.clone(),
        late_captures,
    };
    Ok(PreparedFreeze { readiness, commit })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_store::{
        ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionDraft, EvidenceBindingDraft,
        ExternalResource, PublicationItem,
    };

    async fn fixture(name: &str) -> (PathBuf, Store) {
        let root =
            std::env::temp_dir().join(format!("wisp_publication_{name}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "OPERON", "model")
            .await
            .unwrap();
        (root, store)
    }

    async fn publication_with_item(store: &Store, kind: PublicationItemKind) {
        store
            .create_publication("publication", "project", "Paper", "")
            .await
            .unwrap();
        store
            .create_publication_revision("revision", "publication", None, "Submission")
            .await
            .unwrap();
        store
            .save_publication_item(&PublicationItem {
                id: "item".into(),
                revision_id: "revision".into(),
                parent_item_id: None,
                kind,
                title: "Evidence item".into(),
                content: String::new(),
                ordinal: 0,
                metadata_json: "{}".into(),
                created_at: 0,
                updated_at: 0,
            })
            .await
            .unwrap();
    }

    async fn select_evidence(store: &Store, source_kind: EvidenceSourceKind, source_id: &str) {
        store
            .save_evidence_binding(&EvidenceBindingDraft {
                id: "binding".into(),
                revision_id: "revision".into(),
                item_id: Some("item".into()),
                source_kind,
                source_id: source_id.into(),
                purpose: "Paper evidence".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Public,
            })
            .await
            .unwrap();
    }

    fn public_policy() -> PublicationFreezePolicy {
        PublicationFreezePolicy {
            target_visibility: EvidenceVisibility::Public,
            phi_pii_reviewed: true,
            redistribution_reviewed: true,
            snapshot_restricted_bytes: false,
        }
    }

    async fn successful_run(store: &Store, command: &str) {
        let mut run = RunRecord::new("run", "project", "local", "Analysis", "command");
        run.command = Some(command.into());
        run.env_snapshot_json = canonical_json(&json!({
            "context": {"id": "local", "kind": "local"},
            "schema_version": 1,
            "wisp_host": {"arch": std::env::consts::ARCH, "os": std::env::consts::OS},
        }));
        store.create_run(&run).await.unwrap();
        assert!(store
            .activate_run_lifecycle("run", RunStatus::Running, "test", 60)
            .await
            .unwrap());
        assert!(store
            .finish_active_run_owned("run", "test", RunStatus::Succeeded, Some(0))
            .await
            .unwrap());
    }

    #[test]
    fn security_scan_finds_embedded_platform_paths_without_flagging_urls() {
        assert!(scan_security("python /tmp/project/analysis.py").machine_path);
        assert!(scan_security(r#"Rscript C:\Study\analysis.R"#).machine_path);
        assert!(!scan_security("https://example.org/data/release-1").machine_path);
    }

    #[tokio::test]
    async fn exact_snapshot_freezes_deterministically_and_reports_drift() {
        let (root, store) = fixture("deterministic").await;
        std::fs::create_dir_all(root.join("results")).unwrap();
        let source = root.join("results/supplement.txt");
        std::fs::write(&source, b"stable result\n").unwrap();
        let captured = capture_file(&root, &source, SnapshotPolicy::Always).unwrap();
        let version_id = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("version-1".into()),
                artifact_id: "artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "supplement.txt".into(),
                content_type: "text/plain".into(),
                storage_path: captured.storage_path.clone(),
                logical_key: Some("supplement".into()),
                size_bytes: Some(captured.size_bytes as i64),
                checksum: Some(captured.checksum.clone()),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        publication_with_item(&store, PublicationItemKind::Supplement).await;
        select_evidence(&store, EvidenceSourceKind::ArtifactVersion, &version_id).await;

        let policy = public_policy();
        store
            .begin_publication_freeze("revision", "determinism", &policy)
            .await
            .unwrap();
        let first = prepare_publication_freeze(&store, "revision", "determinism", &policy)
            .await
            .unwrap();
        let second = prepare_publication_freeze(&store, "revision", "determinism", &policy)
            .await
            .unwrap();
        assert!(first.readiness.can_freeze);
        assert_eq!(
            first.readiness.manifest_sha256,
            second.readiness.manifest_sha256
        );
        assert_eq!(
            first.readiness.manifest_json,
            second.readiness.manifest_json
        );
        assert!(store
            .abort_publication_freeze("revision", "determinism")
            .await
            .unwrap());

        let outcome = freeze_publication_revision_in_store(&store, "revision", policy.clone())
            .await
            .unwrap();
        assert!(outcome.frozen);
        assert_eq!(
            outcome.readiness.manifest_sha256,
            first.readiness.manifest_sha256
        );
        assert_eq!(
            outcome.revision.manifest_sha256.as_deref(),
            Some(first.readiness.manifest_sha256.as_str())
        );
        let report = store
            .get_publication_readiness_report("revision")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            report.policy_json,
            canonical_json(&serde_json::to_value(policy).unwrap())
        );

        std::fs::write(&source, b"changed live file\n").unwrap();
        assert_eq!(
            std::fs::read(root.join(&captured.storage_path)).unwrap(),
            b"stable result\n"
        );
        let changed = capture_file(&root, &source, SnapshotPolicy::Always).unwrap();
        store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("version-2".into()),
                artifact_id: "artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "supplement.txt".into(),
                content_type: "text/plain".into(),
                storage_path: changed.storage_path,
                logical_key: Some("supplement".into()),
                size_bytes: Some(changed.size_bytes as i64),
                checksum: Some(changed.checksum),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let drift = store
            .list_publication_evidence_drift("revision")
            .await
            .unwrap();
        assert_eq!(drift.len(), 1);
        assert!(drift[0].has_drift);
        assert_eq!(drift[0].bound_version_id, version_id);
        assert_eq!(drift[0].latest_version_id, "version-2");

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn historical_live_file_is_late_captured_without_rewriting_history() {
        let (root, store) = fixture("late_capture").await;
        std::fs::create_dir_all(root.join("results")).unwrap();
        let source = root.join("results/legacy.txt");
        std::fs::write(&source, b"legacy bytes\n").unwrap();
        let old_version_id = store
            .save_artifact(
                "artifact",
                "project",
                "frame",
                "legacy.txt",
                "text/plain",
                "results/legacy.txt",
            )
            .await
            .unwrap();
        publication_with_item(&store, PublicationItemKind::Supplement).await;
        select_evidence(&store, EvidenceSourceKind::ArtifactVersion, &old_version_id).await;

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(outcome.frozen);
        assert_eq!(
            outcome.readiness.capability_level,
            PublicationCapabilityLevel::Archived
        );
        assert!(outcome
            .readiness
            .warnings
            .iter()
            .any(|finding| finding.code == "historical_content_unverified"));
        let binding = store
            .get_evidence_binding("binding")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(binding.source_id, old_version_id);
        let old = store
            .get_artifact_version(&old_version_id)
            .await
            .unwrap()
            .unwrap();
        assert!(old.checksum.is_none());
        assert_eq!(old.capture_timing, ArtifactCaptureTiming::Unknown);
        let captured = store
            .get_artifact_version(&binding.source_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(captured.capture_timing, ArtifactCaptureTiming::Late);
        assert_eq!(captured.materialization, ArtifactMaterialization::Snapshot);
        std::fs::write(&source, b"new live bytes\n").unwrap();
        assert_eq!(
            std::fs::read(root.join(&captured.storage_path)).unwrap(),
            b"legacy bytes\n"
        );

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn readiness_check_does_not_lock_revision_and_confirm_rechecks_evidence() {
        let (root, store) = fixture("check_before_freeze").await;
        store
            .append_message("frame", 1, &wisp_llm::Message::user("stable evidence"))
            .await
            .unwrap();
        publication_with_item(&store, PublicationItemKind::Methods).await;
        let locator = canonical_json(
            &json!({"byte_end":15,"byte_start":0,"frame_id":"frame","message_seq":1}),
        );
        select_evidence(&store, EvidenceSourceKind::MessageSpan, &locator).await;
        let checked = prepare_or_freeze_publication(&store, "revision", public_policy(), true)
            .await
            .unwrap();
        assert!(
            checked.readiness.can_freeze,
            "{:?}",
            checked.readiness.blockers
        );
        assert!(!checked.frozen);
        assert_eq!(
            checked.revision.state,
            wisp_store::PublicationRevisionState::Draft
        );
        assert!(checked.revision.manifest_sha256.is_none());
        // Check is repeatable and does not leave a freezing lease behind.
        assert!(
            !prepare_or_freeze_publication(&store, "revision", public_policy(), true)
                .await
                .unwrap()
                .frozen
        );
        store
            .update_evidence_binding_selection(
                "binding",
                EvidenceSelectionState::Rejected,
                EvidenceVisibility::Public,
            )
            .await
            .unwrap();
        let rejected = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(!rejected.frozen);
        store
            .update_evidence_binding_selection(
                "binding",
                EvidenceSelectionState::Selected,
                EvidenceVisibility::Public,
            )
            .await
            .unwrap();
        assert!(
            freeze_publication_revision_in_store(&store, "revision", public_policy())
                .await
                .unwrap()
                .frozen
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn message_span_freezes_after_its_session_is_deleted() {
        let (root, store) = fixture("message_anchor").await;
        store
            .append_message(
                "frame",
                1,
                &wisp_llm::Message::user("prefix stable evidence suffix"),
            )
            .await
            .unwrap();
        publication_with_item(&store, PublicationItemKind::Methods).await;
        let locator = canonical_json(&json!({
            "byte_end": 22,
            "byte_start": 7,
            "frame_id": "frame",
            "message_seq": 1,
        }));
        select_evidence(&store, EvidenceSourceKind::MessageSpan, &locator).await;
        store.delete_session("frame", "project").await.unwrap();

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(outcome.frozen, "{:?}", outcome.readiness.blockers);
        let manifest: Value = serde_json::from_str(&outcome.readiness.manifest_json).unwrap();
        assert_eq!(
            manifest["evidence"][0]["source_snapshot"]["anchor"]["text_snapshot"],
            "stable evidence"
        );

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn changed_external_resource_cannot_reinterpret_selected_evidence() {
        let (root, store) = fixture("external_anchor_drift").await;
        let mut resource = ExternalResource {
            id: "dataset".into(),
            project_id: "project".into(),
            kind: "dataset".into(),
            uri: "doi:10.1234/example".into(),
            version: Some("v1".into()),
            checksum: Some("d".repeat(64)),
            size_bytes: Some(1_000),
            license: Some("CC-BY-4.0".into()),
            visibility: "public".into(),
            access_instructions: Some("Resolve the DOI".into()),
            accessed_at: Some(1),
            created_at: 1,
            updated_at: 1,
        };
        store.save_external_resource(&resource).await.unwrap();
        publication_with_item(&store, PublicationItemKind::Methods).await;
        select_evidence(&store, EvidenceSourceKind::ExternalResource, "dataset").await;
        resource.version = Some("v2".into());
        resource.updated_at = 2;
        store.save_external_resource(&resource).await.unwrap();

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(!outcome.frozen);
        assert!(outcome
            .readiness
            .blockers
            .iter()
            .any(|finding| finding.code == "external_resource_anchor_drift"));

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn public_freeze_omits_restricted_external_input_bytes() {
        let (root, store) = fixture("restricted").await;
        successful_run(&store, "python analysis.py").await;
        store
            .save_external_resource(&ExternalResource {
                id: "dataset".into(),
                project_id: "project".into(),
                kind: "dataset".into(),
                uri: "doi:10.1234/restricted-dataset".into(),
                version: Some("release-3".into()),
                checksum: Some("d".repeat(64)),
                size_bytes: Some(1_000),
                license: Some("DUA".into()),
                visibility: "restricted".into(),
                access_instructions: Some("Apply through the data access committee".into()),
                accessed_at: Some(1),
                created_at: 1,
                updated_at: 1,
            })
            .await
            .unwrap();
        store
            .save_run_input(&RunInput {
                id: "input".into(),
                run_id: "run".into(),
                artifact_version_id: None,
                external_resource_id: Some("dataset".into()),
                source_ref: "restricted cohort".into(),
                role: "primary_data".into(),
                required: true,
                basis: LineageBasis::Declared,
                confidence: LineageConfidence::Exact,
                created_at: 1,
            })
            .await
            .unwrap();
        publication_with_item(&store, PublicationItemKind::Methods).await;
        select_evidence(&store, EvidenceSourceKind::Run, "run").await;

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(outcome.frozen);
        assert_eq!(
            outcome.readiness.capability_level,
            PublicationCapabilityLevel::ReExecutable
        );
        assert!(outcome.readiness.omissions.iter().any(|finding| {
            finding.code == "visibility_dependency_omitted"
                && finding.source_id.as_deref() == Some("dataset")
        }));
        let manifest: Value = serde_json::from_str(&outcome.readiness.manifest_json).unwrap();
        let resource = &manifest["external_resources"][0];
        assert_eq!(resource["id"], "dataset");
        assert_eq!(
            resource["access_instructions"],
            "Apply through the data access committee"
        );
        assert!(manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["source_id"] != "dataset"));

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn text_larger_than_security_scan_limit_is_manifest_only() {
        let (root, store) = fixture("large_text").await;
        std::fs::create_dir_all(root.join("results")).unwrap();
        let source = root.join("results/large.txt");
        std::fs::write(&source, vec![b'a'; MAX_TEXT_SCAN_BYTES as usize + 1]).unwrap();
        let captured = capture_file(&root, &source, SnapshotPolicy::Always).unwrap();
        let version_id = store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("large-version".into()),
                artifact_id: "large-artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "large.txt".into(),
                content_type: "text/plain".into(),
                storage_path: captured.storage_path,
                logical_key: Some("large-text".into()),
                size_bytes: Some(captured.size_bytes as i64),
                checksum: Some(captured.checksum),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        publication_with_item(&store, PublicationItemKind::Supplement).await;
        select_evidence(&store, EvidenceSourceKind::ArtifactVersion, &version_id).await;

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(outcome.frozen);
        assert!(outcome
            .readiness
            .omissions
            .iter()
            .any(|finding| finding.code == "text_security_scan_omitted"));
        let manifest: Value = serde_json::from_str(&outcome.readiness.manifest_json).unwrap();
        assert_eq!(manifest["files"][0]["include_bytes"], false);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn incomplete_environment_downgrades_run_to_traceable() {
        let (root, store) = fixture("environment").await;
        let mut run = RunRecord::new("run", "project", "local", "Analysis", "command");
        run.command = Some("python analysis.py".into());
        store.create_run(&run).await.unwrap();
        assert!(store
            .activate_run_lifecycle("run", RunStatus::Running, "test", 60)
            .await
            .unwrap());
        assert!(store
            .finish_active_run_owned("run", "test", RunStatus::Succeeded, Some(0))
            .await
            .unwrap());
        publication_with_item(&store, PublicationItemKind::Methods).await;
        select_evidence(&store, EvidenceSourceKind::Run, "run").await;

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(outcome.frozen);
        assert_eq!(
            outcome.readiness.capability_level,
            PublicationCapabilityLevel::Traceable
        );
        assert!(outcome
            .readiness
            .warnings
            .iter()
            .any(|finding| finding.code == "run_environment_incomplete"));

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn empty_command_snapshot_blocks_run_evidence() {
        let (root, store) = fixture("missing_code").await;
        let mut run = RunRecord::new("run", "project", "local", "Analysis", "command");
        run.env_snapshot_json = canonical_json(&json!({
            "context": {"id": "local", "kind": "local"},
            "schema_version": 1,
            "wisp_host": {"arch": std::env::consts::ARCH, "os": std::env::consts::OS},
        }));
        store.create_run(&run).await.unwrap();
        assert!(store
            .activate_run_lifecycle("run", RunStatus::Running, "test", 60)
            .await
            .unwrap());
        assert!(store
            .finish_active_run_owned("run", "test", RunStatus::Succeeded, Some(0))
            .await
            .unwrap());
        publication_with_item(&store, PublicationItemKind::Methods).await;
        select_evidence(&store, EvidenceSourceKind::Run, "run").await;

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(!outcome.frozen);
        assert!(outcome
            .readiness
            .blockers
            .iter()
            .any(|finding| finding.code == "run_code_missing"));

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn secret_blocks_freeze_without_persisting_partial_state() {
        let (root, store) = fixture("secret").await;
        successful_run(&store, "python analysis.py --api_key=supersecret123").await;
        publication_with_item(&store, PublicationItemKind::Methods).await;
        select_evidence(&store, EvidenceSourceKind::Run, "run").await;

        let outcome = freeze_publication_revision_in_store(&store, "revision", public_policy())
            .await
            .unwrap();
        assert!(!outcome.frozen);
        assert_eq!(outcome.revision.state, PublicationRevisionState::Draft);
        assert!(outcome
            .readiness
            .blockers
            .iter()
            .any(|finding| finding.code == "potential_secret" && !finding.waivable));
        assert!(store
            .get_publication_readiness_report("revision")
            .await
            .unwrap()
            .is_none());

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
