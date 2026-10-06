//! On-disk state of one `map_items` run under `.wisp/map-runs/<run_id>/`:
//!
//! - `manifest.json`: the frozen item list, the frozen spec, run status.
//! - `rows.jsonl`: append-only, one line per attempt; the last line for an
//!   item wins, so a retry never rewrites history.
//! - `results.csv`: flat table derived from the rows (re-rendered each call).
//! - `reduce.md`: the optional reduce output.
//!
//! Nothing here is kept in memory between calls, so a crash, a Stop or an app
//! restart loses at most the in-flight items.

use super::decide::{verdict, Thresholds, Verdict};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use wisp_llm::systemone::Question;

/// Frozen-item ceiling per run. A tool call blocks its turn, so a bigger list
/// finishes over several `resume` calls instead of one unbounded call.
pub const MAX_ITEMS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Project-relative path (or absolute, for a file outside the root), the
    /// row `id` of a JSONL source, or a previous run's item id.
    pub id: String,
    /// Frozen structured input. `None` means "a file: read `id`".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSpec {
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Read-only subset of `read`/`grep`/`search`; empty = one plain model call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    pub max_iterations: usize,
    /// Model id on the session's endpoint, e.g. a cheaper extraction model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecideSpec {
    pub questions: BTreeMap<String, Question>,
    pub thresholds: Thresholds,
    /// How much of the item text Jev sees when there is no worker.
    pub text_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReduceSpec {
    pub instruction: String,
    /// Which verdicts feed the reduce; rows with no verdict always do.
    pub verdicts: Vec<Verdict>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<WorkerSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decide: Option<DecideSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce: Option<ReduceSpec>,
    pub concurrency: usize,
    /// Characters of one item's text handed to the model (head + tail kept).
    pub max_chars: usize,
    pub max_minutes: u64,
    /// Per-call model-token ceiling; `None` = derived from the pending work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    /// Items are still pending: Stop, a budget, or a failing service.
    Paused,
    /// Every item has a final row (failed ones can still be retried).
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub run_id: String,
    pub created: String,
    pub status: RunStatus,
    #[serde(default)]
    pub note: String,
    pub items: Vec<Item>,
    pub spec: Spec,
    /// The Jev version that answered first; later calls and resumes pin it so
    /// one run never mixes model versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowStatus {
    Ok,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub item: String,
    pub status: RowStatus,
    /// The worker's structured output, kept even when a later step failed so a
    /// retry only redoes that step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Raw yes-probabilities of the noul questions; verdicts derive from them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scores: BTreeMap<String, f64>,
    /// Choice/score answers, recorded but never part of the verdict.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub answers: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub chars: usize,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev: Option<String>,
    pub ts: String,
}

impl Row {
    pub fn verdict(&self, spec: &Spec) -> Option<Verdict> {
        let decide = spec.decide.as_ref()?;
        (self.status == RowStatus::Ok)
            .then(|| verdict(&self.scores, &decide.questions, decide.thresholds))
            .flatten()
    }
}

pub struct RunDir {
    pub dir: PathBuf,
}

/// Run ids end up in paths: allow only what `new_run_id` produces.
pub fn valid_run_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn new_run_id() -> String {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    format!(
        "{stamp}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    )
}

pub fn runs_dir(root: &Path) -> PathBuf {
    root.join(".wisp").join("map-runs")
}

impl RunDir {
    pub fn create(root: &Path, manifest: &Manifest) -> std::io::Result<Self> {
        let dir = runs_dir(root).join(&manifest.run_id);
        std::fs::create_dir_all(&dir)?;
        let run = Self { dir };
        run.save_manifest(manifest)?;
        Ok(run)
    }

    pub fn open(root: &Path, run_id: &str) -> Result<(Self, Manifest), String> {
        if !valid_run_id(run_id) {
            return Err(format!("invalid run id '{run_id}'"));
        }
        let dir = runs_dir(root).join(run_id);
        let text = std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|e| format!("no map_items run '{run_id}' in this project: {e}"))?;
        let manifest = serde_json::from_str(&text)
            .map_err(|e| format!("run '{run_id}' has an unreadable manifest: {e}"))?;
        Ok((Self { dir }, manifest))
    }

    pub fn save_manifest(&self, manifest: &Manifest) -> std::io::Result<()> {
        let tmp = self.dir.join("manifest.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(manifest)?)?;
        std::fs::rename(tmp, self.dir.join("manifest.json"))
    }

    pub fn append_row(&self, row: &Row) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("rows.jsonl"))?;
        let mut line = serde_json::to_vec(row)?;
        line.push(b'\n');
        file.write_all(&line)
    }

    /// Latest row per item. A line cut short by a crash is skipped, so that
    /// item is simply pending again.
    pub fn load_rows(&self) -> HashMap<String, Row> {
        let text = std::fs::read_to_string(self.dir.join("rows.jsonl")).unwrap_or_default();
        text.lines()
            .filter_map(|line| serde_json::from_str::<Row>(line).ok())
            .map(|row| (row.item.clone(), row))
            .collect()
    }

    pub fn write_reduce(&self, text: &str) -> std::io::Result<()> {
        std::fs::write(self.dir.join("reduce.md"), text)
    }

    pub fn write_results_csv(
        &self,
        manifest: &Manifest,
        rows: &HashMap<String, Row>,
    ) -> std::io::Result<()> {
        std::fs::write(self.dir.join("results.csv"), results_csv(manifest, rows))
    }
}

fn csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Text from a paper or a model can open in a spreadsheet: neutralize a leading
/// formula character, but leave plain numbers alone.
fn csv_value(value: &Value) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => return csv_cell(&other.to_string()),
    };
    let formula =
        text.starts_with(['=', '+', '-', '@', '\t', '\r']) && text.parse::<f64>().is_err();
    csv_cell(&if formula { format!("'{text}") } else { text })
}

/// One line per item in manifest order: item, status, verdict, the union of the
/// worker's top-level fields, one `p_<question>` column per noul question, error.
pub fn results_csv(manifest: &Manifest, rows: &HashMap<String, Row>) -> String {
    let mut fields: Vec<String> = Vec::new();
    for row in manifest.items.iter().filter_map(|item| rows.get(&item.id)) {
        if let Some(Value::Object(map)) = &row.data {
            for key in map.keys() {
                if !fields.contains(key) {
                    fields.push(key.clone());
                }
            }
        }
    }
    let questions: Vec<&String> = manifest
        .spec
        .decide
        .iter()
        .flat_map(|d| d.questions.iter())
        .filter(|(_, q)| matches!(q, Question::Noul { .. }))
        .map(|(id, _)| id)
        .collect();

    let mut head = vec!["item".to_string(), "status".into(), "verdict".into()];
    head.extend(fields.iter().map(|f| csv_cell(f)));
    head.extend(questions.iter().map(|q| csv_cell(&format!("p_{q}"))));
    head.push("error".into());
    let mut out = head.join(",");
    out.push('\n');

    for item in &manifest.items {
        let Some(row) = rows.get(&item.id) else {
            let mut cells = vec![csv_cell(&item.id), "pending".into()];
            cells.resize(head.len(), String::new());
            out.push_str(&cells.join(","));
            out.push('\n');
            continue;
        };
        let mut cells = vec![
            csv_cell(&item.id),
            match row.status {
                RowStatus::Ok => "ok".into(),
                RowStatus::Failed => "failed".into(),
            },
            row.verdict(&manifest.spec)
                .map(|v| v.as_str().to_string())
                .unwrap_or_default(),
        ];
        for field in &fields {
            cells.push(match &row.data {
                Some(Value::Object(map)) => map.get(field).map(csv_value).unwrap_or_default(),
                _ => String::new(),
            });
        }
        for question in &questions {
            cells.push(
                row.scores
                    .get(*question)
                    .map(|p| p.to_string())
                    .unwrap_or_default(),
            );
        }
        cells.push(
            row.error
                .as_deref()
                .map(|e| csv_value(&Value::from(e)))
                .unwrap_or_default(),
        );
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn csv_quotes_and_defuses_spreadsheet_formulas() {
        assert_eq!(csv_value(&json!("plain")), "plain");
        assert_eq!(csv_value(&json!("a,b \"c\"")), "\"a,b \"\"c\"\"\"");
        assert_eq!(
            csv_value(&json!("=HYPERLINK(\"x\")")),
            "\"'=HYPERLINK(\"\"x\"\")\""
        );
        assert_eq!(csv_value(&json!("-0.5")), "-0.5", "numbers stay numbers");
        assert_eq!(csv_value(&json!("@cmd")), "'@cmd");
        assert_eq!(csv_value(&json!(["a", 1])), "\"[\"\"a\"\",1]\"");
        assert_eq!(csv_value(&Value::Null), "");
    }

    #[test]
    fn run_ids_cannot_escape_the_runs_directory() {
        assert!(valid_run_id(&new_run_id()));
        for bad in ["", "..", "../x", "a/b", "a\\b", &"x".repeat(65)] {
            assert!(!valid_run_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_torn_last_line_only_makes_that_item_pending() {
        let root = std::env::temp_dir().join(format!("wisp-map-{}", uuid::Uuid::new_v4()));
        let manifest = Manifest {
            run_id: new_run_id(),
            created: String::new(),
            status: RunStatus::Running,
            note: String::new(),
            items: vec![],
            spec: Spec {
                worker: None,
                decide: None,
                reduce: None,
                concurrency: 1,
                max_chars: 1000,
                max_minutes: 1,
                max_tokens: None,
            },
            jev_model: None,
        };
        let dir = RunDir::create(&root, &manifest).unwrap();
        let row = |item: &str| Row {
            item: item.into(),
            status: RowStatus::Ok,
            data: None,
            scores: BTreeMap::new(),
            answers: BTreeMap::new(),
            error: None,
            chars: 0,
            truncated: false,
            tokens_in: 0,
            tokens_out: 0,
            ms: 0,
            jev: None,
            ts: String::new(),
        };
        dir.append_row(&row("a")).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.dir.join("rows.jsonl"))
            .unwrap();
        file.write_all(b"{\"item\":\"b\",\"stat").unwrap();
        let rows = dir.load_rows();
        assert!(rows.contains_key("a") && !rows.contains_key("b"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
