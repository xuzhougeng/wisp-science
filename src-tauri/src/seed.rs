//! Bundled demo loader — reads the upstream `seed/manifest_*.json` session
//! recordings and presents each as a pre-baked transcript the UI can open.
//! Full operation history lives in `output_data.items` (UiItem-shaped rows).
//! Figure/data files live in paired `assets_*.tar.gz` archives and are extracted
//! into the workspace when a demo is opened.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::State;
use uuid::Uuid;
use wisp_llm::Message;
use wisp_store::Store;

use crate::resource_refs;
use crate::AppState;

const MAX_SEED_REPEAT: usize = 200;
const MAX_SEED_PAD: usize = 64;

/// Bundled demo manifests (`seed/`).
pub fn bundled_dir() -> Option<PathBuf> {
    wisp_paths::seed_dir()
}

#[derive(Serialize, Clone)]
pub struct DemoInfo {
    pub id: String,
    pub title: String,
}

/// One transcript row returned to the UI (same shape as session `UiItem`).
#[derive(Serialize, Clone, Deserialize)]
pub struct DemoUiItem {
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<resource_refs::UiMessageResource>,
}

#[derive(Serialize, Clone)]
pub struct Demo {
    pub id: String,
    pub title: String,
    pub request: String,
    pub response: String,
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DemoUiItem>,
}

#[tauri::command(rename = "list_demos")]
pub(super) fn list_demos_cmd() -> Vec<DemoInfo> {
    list_demos()
}

#[tauri::command(rename = "load_demo")]
pub(super) fn load_demo_cmd(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Demo, String> {
    let ap = state.require_active(window.label())?;
    extract_demo_assets(&id, &ap.root)?;
    load_demo(&id).ok_or_else(|| format!("demo '{id}' not found"))
}

#[tauri::command(rename = "copy_demo_to_project")]
pub(super) async fn copy_demo_to_project_cmd(
    state: State<'_, AppState>,
    id: String,
    target_project_id: String,
) -> Result<String, String> {
    let (_, workspace_dir) = state
        .store
        .get_project(&target_project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Target project not found.".to_string())?;
    if workspace_dir.trim().is_empty() {
        return Err("Target project has no workspace.".into());
    }
    let _activity = state.begin_project_activity(&target_project_id)?;
    let model_id = crate::models::active_profile_id(&state.store).await;
    copy_demo_into_project(
        &state.store,
        &id,
        &target_project_id,
        Path::new(&workspace_dir),
        &model_id,
    )
    .await
}

#[derive(Deserialize)]
struct DemoSeedTurn {
    role: String,
    #[serde(default)]
    text: String,
    #[serde(default = "default_seed_repeat")]
    repeat: usize,
    /// Repeat `text` inside one message so a compact seed file still expands
    /// into enough tokens for a real `/compact` to fold the opening answer.
    #[serde(default = "default_seed_repeat")]
    pad: usize,
}

fn default_seed_repeat() -> usize {
    1
}

fn seed_item(role: &str, text: &str) -> DemoUiItem {
    DemoUiItem {
        role: role.to_string(),
        text: text.to_string(),
        tool_name: None,
        ok: None,
        duration_ms: None,
        input: None,
        model_name: None,
        call_id: None,
        kind: None,
        status: None,
        locations: None,
        resources: Vec::new(),
    }
}

fn expand_seed_turns(turns: Vec<DemoSeedTurn>) -> Vec<DemoUiItem> {
    let mut out = Vec::new();
    for turn in turns {
        let n = turn.repeat.clamp(1, MAX_SEED_REPEAT);
        let pad = turn.pad.clamp(1, MAX_SEED_PAD);
        let text = turn.text.repeat(pad);
        for _ in 0..n {
            out.push(seed_item(&turn.role, &text));
        }
    }
    out
}

fn clean(text: &str) -> String {
    static IMG: OnceLock<Regex> = OnceLock::new();
    static ART: OnceLock<Regex> = OnceLock::new();
    let img = IMG.get_or_init(|| Regex::new(r"!\[([^\]]*)\]\(\{\{artifact:[^}]+\}\}\)").unwrap());
    let art = ART.get_or_init(|| Regex::new(r"\{\{artifact:[^}]+\}\}").unwrap());
    let s = img.replace_all(text, "[$1 (figure)]").to_string();
    art.replace_all(&s, "(artifact)").to_string()
}

fn read_title(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let req = v
        .pointer("/root_frame/input_data/request")
        .and_then(|x| x.as_str())?;
    let first = req.split('.').next().unwrap_or(req).trim();
    Some(first.chars().take(70).collect())
}

/// Enumerate `manifest_*.json` in the bundled seed dir.
pub fn list_demos() -> Vec<DemoInfo> {
    let Some(dir) = bundled_dir() else {
        return vec![];
    };
    let mut out = vec![];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if !stem.starts_with("manifest_") {
                continue;
            }
            let title =
                read_title(&p).unwrap_or_else(|| stem.trim_start_matches("manifest_").to_string());
            out.push(DemoInfo { id: stem, title });
        }
    }
    // Numeric id prefixes (manifest_esr1_01_…) keep the research narrative order.
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn assets_tarball(id: &str) -> Option<PathBuf> {
    let dir = bundled_dir()?;
    let suffix = id.strip_prefix("manifest_")?;
    let path = dir.join(format!("assets_{suffix}.tar.gz"));
    path.is_file().then_some(path)
}

/// Extract bundled demo files into `dest` (workspace root), flattening the
/// `example_*` folder inside each tarball so transcript filenames resolve.
/// Demos without an assets archive are a no-op.
pub fn extract_demo_assets(id: &str, dest: &Path) -> Result<(), String> {
    let Some(tar_path) = assets_tarball(id) else {
        return Ok(());
    };
    std::fs::create_dir_all(dest).map_err(|e| format!("create demo dest: {e}"))?;
    let file = File::open(&tar_path).map_err(|e| format!("open {}: {e}", tar_path.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in archive.entries().map_err(|e| format!("read tar: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let path = entry.path().map_err(|e| format!("tar path: {e}"))?;
        let Some(name) = path.file_name() else {
            continue;
        };
        let out = dest.join(name);
        entry
            .unpack(&out)
            .map_err(|e| format!("unpack {}: {e}", out.display()))?;
    }
    Ok(())
}

fn clean_item(mut item: DemoUiItem) -> DemoUiItem {
    item.text = clean(&item.text);
    if let Some(input) = item.input.as_mut() {
        *input = clean(input);
    }
    item
}

struct DemoManifest {
    demo: Demo,
    workspace_files: BTreeMap<String, String>,
}

fn load_demo_manifest(id: &str) -> Option<DemoManifest> {
    let dir = bundled_dir()?;
    let path = dir.join(format!("{id}.json"));
    let text = std::fs::read_to_string(&path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let req = v
        .pointer("/root_frame/input_data/request")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let resp = v
        .pointer("/root_frame/output_data/response")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let thinking = v
        .pointer("/root_frame/output_data/thinking")
        .and_then(|x| x.as_str())
        .map(String::from);
    let mut items = v
        .pointer("/root_frame/output_data/context_seed")
        .and_then(|x| serde_json::from_value::<Vec<DemoSeedTurn>>(x.clone()).ok())
        .map(expand_seed_turns)
        .unwrap_or_default();
    items.extend(
        v.pointer("/root_frame/output_data/items")
            .and_then(|x| serde_json::from_value::<Vec<DemoUiItem>>(x.clone()).ok())
            .unwrap_or_default(),
    );
    let items = items.into_iter().map(clean_item).collect();
    let workspace_files = v
        .pointer("/root_frame/output_data/workspace_files")
        .and_then(|x| serde_json::from_value::<BTreeMap<String, String>>(x.clone()).ok())
        .unwrap_or_default();
    let title = read_title(&path).unwrap_or_else(|| id.trim_start_matches("manifest_").to_string());
    Some(DemoManifest {
        demo: Demo {
            id: id.to_string(),
            title,
            request: clean(&req),
            response: clean(&resp),
            thinking: thinking.map(|t| clean(&t)),
            items,
        },
        workspace_files,
    })
}

/// Load one demo by id (the manifest file stem, e.g. `manifest_esr1_03_rnaseq`).
pub fn load_demo(id: &str) -> Option<Demo> {
    load_demo_manifest(id).map(|manifest| manifest.demo)
}

fn safe_workspace_path(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(format!("unsafe demo path: {rel}"));
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(seg) => out.push(seg),
            _ => return Err(format!("unsafe demo path: {rel}")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("unsafe demo path: {rel}"));
    }
    Ok(root.join(out))
}

fn write_workspace_files(root: &Path, files: &BTreeMap<String, String>) -> Result<(), String> {
    for (rel, content) in files {
        let dest = safe_workspace_path(root, rel)?;
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        std::fs::write(&dest, content).map_err(|e| format!("write {}: {e}", dest.display()))?;
    }
    Ok(())
}

fn demo_items_to_messages(items: &[DemoUiItem]) -> Vec<Message> {
    let mut out = Vec::new();
    let mut pending_reasoning = None;
    for (idx, item) in items.iter().enumerate() {
        match item.role.as_str() {
            "reasoning" => pending_reasoning = Some(item.text.clone()),
            "user" => {
                pending_reasoning = None;
                out.push(Message::user(&item.text));
            }
            "assistant" => {
                let mut message = Message::assistant(&item.text);
                message.reasoning = pending_reasoning.take();
                message.model_name = item.model_name.clone();
                out.push(message);
            }
            "tool" => {
                let name = item.tool_name.clone().unwrap_or_else(|| "tool".to_string());
                let call_id = item
                    .call_id
                    .clone()
                    .unwrap_or_else(|| format!("demo-tool-{idx}"));
                out.push(Message::tool(call_id, name, &item.text));
            }
            _ => {}
        }
    }
    out
}

pub async fn copy_demo_into_project(
    store: &Store,
    demo_id: &str,
    project_id: &str,
    workspace: &Path,
    model_id: &str,
) -> Result<String, String> {
    let manifest =
        load_demo_manifest(demo_id).ok_or_else(|| format!("demo '{demo_id}' not found"))?;
    let messages = demo_items_to_messages(&manifest.demo.items);
    if messages.is_empty() {
        return Err(format!("demo '{demo_id}' has no conversation to copy"));
    }
    write_workspace_files(workspace, &manifest.workspace_files)?;
    extract_demo_assets(demo_id, workspace)?;
    let frame_id = Uuid::new_v4().to_string();
    store
        .create_frame(&frame_id, project_id, "OPERON", model_id)
        .await
        .map_err(|e| e.to_string())?;
    store
        .replace_messages(&frame_id, &messages)
        .await
        .map_err(|e| e.to_string())?;
    let title = manifest.demo.title.trim();
    if !title.is_empty() {
        let _ = store.rename_session(&frame_id, project_id, title).await;
    }
    Ok(frame_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_esr1_demo_assets() {
        let tmp = std::env::temp_dir().join(format!("wisp-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        extract_demo_assets("manifest_esr1_03_rnaseq", &tmp).expect("extract rnaseq assets");
        assert!(tmp.join("GSE153250_counts_matrix.tsv").is_file());
        assert!(tmp.join("GSE153250_sample_groups.txt").is_file());
        assert!(tmp.join("GSE153250_featureCounts_summary.txt").is_file());

        let down = tmp.join("downstream");
        std::fs::create_dir_all(&down).unwrap();
        extract_demo_assets("manifest_esr1_04_downstream", &down)
            .expect("extract downstream assets");
        assert!(down.join("DESeq2_top200.csv").is_file());
        assert!(down.join("research_projects.md").is_file());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn lists_and_loads_bundled_demos() {
        let demos = list_demos();
        assert_eq!(
            demos.len(),
            6,
            "bundled seed should ship the five ESR1 demos plus the long-context memory demo"
        );
        assert_eq!(
            demos.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            [
                "manifest_esr1_01_datasets",
                "manifest_esr1_02_samples",
                "manifest_esr1_03_rnaseq",
                "manifest_esr1_04_downstream",
                "manifest_esr1_05_hypotheses",
                "manifest_memory_01_long_context",
            ]
        );
        for info in &demos {
            let demo = load_demo(&info.id).expect("load demo");
            assert!(!demo.request.is_empty());
            assert!(!demo.request.contains("English reply"));
            assert!(!demo.request.to_ascii_lowercase().contains("guotosky"));
            assert!(
                !demo.items.is_empty(),
                "{} should ship transcript items",
                info.id
            );
            let is_esr1 = info.id.starts_with("manifest_esr1_");
            if is_esr1 {
                assert!(
                    demo.items.iter().any(|i| i.role == "tool"),
                    "{} should include tool operation records",
                    info.id
                );
            }
            let blob = serde_json::to_string(&demo).unwrap();
            assert!(!blob.to_ascii_lowercase().contains("guotosky"));
            assert!(!blob.contains("10.10.10."));
            assert!(!blob.contains(":7897"));
            assert!(!blob.to_ascii_lowercase().contains("proxy configured"));
            assert!(!blob.to_ascii_lowercase().contains("proxy settings"));
            assert!(!blob.to_ascii_lowercase().contains("bashrc"));
            assert!(!blob.contains("kimi-k3"));
            assert!(!blob.contains("{{artifact:"));
            if is_esr1 {
                assert!(
                    demo.items
                        .iter()
                        .filter_map(|i| i.model_name.as_deref())
                        .all(|m| m == "deepseek-v4-pro"),
                    "{} should use deepseek-v4-pro for all model labels",
                    info.id
                );
            }
        }

        let datasets = load_demo("manifest_esr1_01_datasets").expect("datasets demo");
        assert!(
            datasets.request.contains("MCF7") || datasets.request.contains("ESR1"),
            "datasets demo request should mention ESR1/MCF7"
        );

        let samples = load_demo("manifest_esr1_02_samples").expect("samples demo");
        assert!(
            samples.request.contains("GSE153250"),
            "samples demo request should mention GSE153250"
        );

        let rnaseq = load_demo("manifest_esr1_03_rnaseq").expect("rnaseq demo");
        assert!(
            rnaseq
                .items
                .iter()
                .any(|i| i.tool_name.as_deref() == Some("monitor_run")),
            "rnaseq demo should include SSH/run monitor cards"
        );
        assert!(
            rnaseq.response.contains("GSE153250") || rnaseq.response.contains("siESR1"),
            "rnaseq response should mention the study"
        );

        let downstream = load_demo("manifest_esr1_04_downstream").expect("downstream demo");
        assert!(
            downstream.request.contains("differential")
                || downstream.request.contains("GSEA")
                || downstream.request.contains("Enrichr"),
            "downstream demo request should mention enrichment/DEG"
        );

        let hypotheses = load_demo("manifest_esr1_05_hypotheses").expect("hypotheses demo");
        assert!(
            hypotheses.request.contains("research projects")
                || hypotheses.request.contains("scientific"),
            "hypotheses demo request should ask for research projects"
        );

        let memory = load_demo("manifest_memory_01_long_context").expect("memory demo");
        assert!(
            memory.items.len() > 100,
            "memory demo should expand into a long transcript, got {}",
            memory.items.len()
        );
        assert_eq!(memory.items[0].role, "user");
        assert!(memory.items[0].text.contains("GSE153250"));
        assert_eq!(memory.items[1].role, "assistant");
        assert!(memory.items[1].text.contains("GENE_FILTER="));
        assert!(memory.items[1].text.contains("PRIMARY_CONTRAST="));
        assert!(memory.items[1].text.contains("FDR_CUTOFF=0.05"));
        // The recorded conversation legitimately reuses the locked values when
        // applying them (turn 2) and in the proposed memory note (turn 6), so
        // they are not exclusive to the opening turn. What must hold: the
        // exact GENE_FILTER phrasing stays out of the protected recent tail,
        // so a full post-compact recall has to come from the checkpoint.
        let gene_filter = "GENE_FILTER=keep genes with CPM > 1 in at least 6 samples";
        assert!(memory.items[1].text.contains(gene_filter));
        for item in &memory.items[memory.items.len().saturating_sub(20)..] {
            assert!(
                !item.text.contains(gene_filter),
                "the exact GENE_FILTER phrasing must not survive in the recent tail"
            );
        }
        let last_user = memory
            .items
            .iter()
            .rev()
            .find(|item| item.role == "user")
            .expect("a user item");
        assert!(last_user
            .text
            .contains("do not restate the opening locked decision"));
        let chars: usize = memory.items.iter().map(|item| item.text.len()).sum();
        assert!(
            chars > 300_000,
            "expanded transcript should carry the full recorded session, got {chars} chars"
        );
        let tokens: usize = demo_items_to_messages(&memory.items)
            .iter()
            .map(wisp_core::ContextManager::estimated_tokens)
            .sum();
        // The transcript is a real recorded session (~104K estimated tokens,
        // ~70K after safe pruning), so a manual /compact installs the semantic
        // checkpoint once the configured window is ~110K or smaller (the fold
        // gate is 60% of the window).
        assert!(
            tokens > 80_000,
            "estimated tokens should exceed a ~110K-class 60% fold gate, got {tokens}"
        );
        assert!(memory.request.contains("Long-context memory demo"));
    }

    #[tokio::test]
    async fn copies_long_context_demo_into_a_project_workspace() {
        let tmp = std::env::temp_dir().join(format!("wisp-copy-demo-{}", Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let store = Store::open(&tmp.join("wisp.sqlite")).await.unwrap();
        let workspace = tmp.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        store
            .create_project("p", "Demo Target", workspace.to_str().unwrap())
            .await
            .unwrap();

        let session_id = copy_demo_into_project(
            &store,
            "manifest_memory_01_long_context",
            "p",
            &workspace,
            "test-model",
        )
        .await
        .expect("copy demo");

        let messages = store.load_messages(&session_id).await.unwrap();
        assert!(messages.len() > 100);
        assert!(messages[0].content.as_text().contains("GSE153250"));
        assert!(messages[1].content.as_text().contains("GENE_FILTER="));
        assert!(messages[1].content.as_text().contains("FDR_CUTOFF=0.05"));
        let tokens: usize = messages
            .iter()
            .map(wisp_core::ContextManager::estimated_tokens)
            .sum();
        assert!(
            tokens > 80_000,
            "copied session too short to exercise a real fold: {tokens}"
        );
        assert!(workspace.join(".wisp/memory/2026-08-13.md").is_file());
        assert!(workspace.join(".wisp/memory/2025-05-20.md").is_file());
        assert!(workspace.join("AGENTS.md").is_file());
        assert!(workspace.join(".wisp/WISP.md").is_file());
        let sessions = store.list_sessions("p").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].1.contains("Long-context memory demo"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rejects_unsafe_demo_workspace_paths() {
        let root = PathBuf::from("/tmp/wisp-demo-root");
        assert!(safe_workspace_path(&root, "../etc/passwd").is_err());
        assert!(safe_workspace_path(&root, "/etc/passwd").is_err());
        assert!(safe_workspace_path(&root, "").is_err());
        assert_eq!(
            safe_workspace_path(&root, ".wisp/memory/note.md").unwrap(),
            root.join(".wisp/memory/note.md")
        );
    }
}
