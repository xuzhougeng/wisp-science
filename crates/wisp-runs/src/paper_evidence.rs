//! `paper_evidence`: set a project file, a registered result or a Run as
//! evidence of a paper from the conversation, so nobody has to look the
//! source up by hand. The paper's figures, tables and source data are kept
//! in one place: the paper's draft version.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};
use wisp_llm::ToolSchema;
use wisp_store::{
    ArtifactCaptureTiming, ArtifactVersionDraft, EvidenceBinding, EvidenceBindingDraft,
    EvidenceSelectionState, EvidenceSourceKind, EvidenceVisibility, Publication, PublicationItem,
    PublicationItemKind, PublicationRevision, PublicationRevisionState, StateScope, Store,
};
use wisp_tools::{Tool, ToolEnv, ToolResult};

use crate::snapshot_store::{capture_file, SnapshotPolicy, DEFAULT_SNAPSHOT_LIMIT};

/// Label of the version a paper started from the conversation begins with.
const FIRST_VERSION: &str = "Initial submission";
const TITLE_LIMIT: usize = 200;
const PURPOSE_LIMIT: usize = 4_000;

pub struct PaperEvidenceTool {
    store: Store,
    scope: StateScope,
    frame_id: String,
}

impl PaperEvidenceTool {
    /// `frame_id` is the conversation a newly registered file is recorded
    /// under; `scope` is that conversation's state scope.
    pub fn new_in_scope(store: Store, scope: StateScope, frame_id: impl Into<String>) -> Self {
        Self {
            store,
            scope,
            frame_id: frame_id.into(),
        }
    }

    fn project(&self) -> &str {
        self.scope.project_id()
    }

    /// Papers are kept by the project itself, not by an exploration.
    fn require_mainline(&self) -> Result<()> {
        ensure!(
            matches!(self.scope, StateScope::Mainline { .. }),
            "Paper evidence is set from the project's own conversations. Keep the exploration's \
             results first, then set them as evidence."
        );
        Ok(())
    }

    /// Every paper of the project at its newest version, with what each
    /// item rests on.
    async fn list(&self) -> Result<Value> {
        let mut papers = Vec::new();
        for paper in self.store.list_publications(self.project()).await? {
            let Some(revision) = newest_revision(&self.store, &paper.id).await? else {
                papers.push(json!({ "id": paper.id, "title": paper.title, "items": [] }));
                continue;
            };
            let bindings = self.store.list_evidence_bindings(&revision.id).await?;
            let items: Vec<Value> = self
                .store
                .list_publication_items(&revision.id)
                .await?
                .iter()
                .map(|item| {
                    json!({
                        "id": item.id,
                        "kind": item.kind.as_str(),
                        "title": item.title,
                        "evidence": bindings
                            .iter()
                            .filter(|binding| binding.item_id.as_deref() == Some(&item.id))
                            .map(evidence_row)
                            .collect::<Vec<_>>(),
                    })
                })
                .collect();
            let unassigned: Vec<Value> = bindings
                .iter()
                .filter(|binding| binding.item_id.is_none())
                .map(evidence_row)
                .collect();
            papers.push(json!({
                "id": paper.id,
                "title": paper.title,
                "version": {
                    "id": revision.id,
                    "label": revision.label,
                    "state": revision.state.as_str(),
                    "editable": revision.state == PublicationRevisionState::Draft,
                },
                "items": items,
                "unassigned": unassigned,
            }));
        }
        Ok(json!({ "papers": papers }))
    }

    /// The paper named by id or title, without changing anything. With
    /// `wanted` unset the project's only paper is meant; a title no paper
    /// has is a paper to start.
    async fn find_paper(&self, wanted: Option<&str>) -> Result<PaperChoice> {
        let papers = self.store.list_publications(self.project()).await?;
        let Some(wanted) = wanted else {
            let mut papers = papers.into_iter();
            return match (papers.next(), papers.next()) {
                (Some(only), None) => Ok(PaperChoice::Existing(only)),
                (None, _) => bail!(
                    "This project has no paper yet. Pass `paper` with the paper's title to \
                     start one."
                ),
                (Some(first), Some(second)) => {
                    let titles: Vec<String> = [first, second]
                        .into_iter()
                        .chain(papers)
                        .map(|paper| format!("“{}”", paper.title))
                        .collect();
                    bail!(
                        "This project has several papers ({}). Pass `paper` with the title of \
                         the one meant.",
                        titles.join(", ")
                    )
                }
            };
        };
        if let Some(found) = papers.iter().find(|paper| paper.id == wanted) {
            return Ok(PaperChoice::Existing(found.clone()));
        }
        let same: Vec<&Publication> = papers
            .iter()
            .filter(|paper| same_title(&paper.title, wanted))
            .collect();
        match same.as_slice() {
            [found] => Ok(PaperChoice::Existing((*found).clone())),
            [] => Ok(PaperChoice::New(title(wanted)?)),
            _ => bail!("Several papers are titled “{wanted}”. Pass `paper` with the paper's id."),
        }
    }

    async fn start_paper(&self, title: &str) -> Result<Publication> {
        let id = uuid::Uuid::new_v4().to_string();
        self.store
            .create_publication(&id, self.project(), title, "")
            .await?;
        self.store
            .get_publication(&id)
            .await?
            .context("The new paper could not be read back")
    }

    /// The paper's draft: its newest version when that is a draft, or its
    /// first version when it has none.
    async fn draft_of(&self, paper: &Publication) -> Result<PublicationRevision> {
        match ensure_draft(&self.store, paper).await? {
            Some(draft) => Ok(draft),
            None => {
                self.store
                    .create_publication_revision(
                        &uuid::Uuid::new_v4().to_string(),
                        &paper.id,
                        None,
                        FIRST_VERSION,
                    )
                    .await
            }
        }
    }

    /// The figure, table or section named by id or title; a title the
    /// draft does not have adds it at the top level, as `kind`.
    async fn item_for(
        &self,
        revision: &str,
        wanted: &str,
        kind: Option<PublicationItemKind>,
    ) -> Result<(PublicationItem, bool)> {
        let items = self.store.list_publication_items(revision).await?;
        if let Some(found) = items.iter().find(|item| item.id == wanted) {
            return Ok((found.clone(), false));
        }
        let same: Vec<&PublicationItem> = items
            .iter()
            .filter(|item| same_title(&item.title, wanted))
            .collect();
        // Two items may share a title across kinds ("Results" section and table).
        let found = match kind {
            Some(kind) => same.iter().find(|item| item.kind == kind).or(same.first()),
            None => same.first(),
        };
        if let Some(found) = found {
            return Ok(((*found).clone(), false));
        }
        let item = PublicationItem {
            id: uuid::Uuid::new_v4().to_string(),
            revision_id: revision.to_owned(),
            parent_item_id: None,
            kind: kind.unwrap_or(PublicationItemKind::Figure),
            title: title(wanted)?,
            content: String::new(),
            // After every item, so it never takes a sibling's place.
            ordinal: items
                .iter()
                .map(|item| item.ordinal)
                .max()
                .map_or(0, |last| last + 1),
            metadata_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        };
        self.store.save_publication_item(&item).await?;
        Ok((item, true))
    }

    /// Registers the project file at `path` as a result, at the content it
    /// has now, and returns the exact version to bind. A file that has not
    /// changed since it was last registered keeps that version.
    async fn register_file(&self, root: &Path, path: &str) -> Result<RegisteredFile> {
        let root = dunce::canonicalize(root).context("The project folder is not available")?;
        let typed = PathBuf::from(path.trim());
        let source = if typed.is_absolute() {
            typed
        } else {
            root.join(typed)
        };
        let real = dunce::canonicalize(&source).map_err(|_| anyhow!("No file at {path}"))?;
        let inside = real.strip_prefix(&root).map_err(|_| {
            anyhow!(
                "{path} is outside the project folder. Copy the file into the project, or \
                 harvest it from its run, then set it as evidence."
            )
        })?;
        ensure!(real.is_file(), "{path} is not a file");
        let relative = inside
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        ensure!(
            !relative.is_empty()
                && !relative
                    .split('/')
                    .any(|part| part == ".wisp" || part == ".git"),
            "{path} is not a project file that can be evidence"
        );
        let capture_root = root.clone();
        let capture_source = real.clone();
        let captured = tokio::task::spawn_blocking(move || {
            capture_file(
                &capture_root,
                &capture_source,
                SnapshotPolicy::UpTo(DEFAULT_SNAPSHOT_LIMIT),
            )
        })
        .await
        .context("The file could not be read")?
        .map_err(|error| anyhow!("{path} could not be registered: {error}"))?;
        let logical_key = format!("path:{relative}");
        let artifact_id = wisp_store::scoped_logical_artifact_id(
            self.project(),
            self.scope.scope_key(),
            &logical_key,
        );
        let name = real
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_owned();
        if let Some(latest) = self
            .store
            .get_latest_artifact_version_context(&artifact_id)
            .await?
        {
            if latest.version.checksum.as_deref() == Some(captured.checksum.as_str()) {
                return Ok(RegisteredFile {
                    version_id: latest.version.id,
                    artifact_id,
                    name,
                    path: relative,
                    registered: false,
                });
            }
        }
        let version_id = self
            .store
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: None,
                artifact_id: artifact_id.clone(),
                project_id: self.project().to_owned(),
                root_frame_id: self.frame_id.clone(),
                filename: name.clone(),
                content_type: crate::mime::mime_for_path(&real).into(),
                storage_path: captured.storage_path,
                logical_key: Some(logical_key),
                size_bytes: i64::try_from(captured.size_bytes).ok(),
                checksum: Some(captured.checksum),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: captured.materialization,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await?;
        Ok(RegisteredFile {
            version_id,
            artifact_id,
            name,
            path: relative,
            registered: true,
        })
    }

    async fn set(&self, args: &Value, root: &Path) -> Result<Value> {
        self.require_mainline()?;
        let path = text_arg(args, "path");
        let artifact = text_arg(args, "artifact_id");
        let run = text_arg(args, "run_id");
        ensure!(
            [path, artifact, run].iter().flatten().count() == 1,
            "Pass exactly one of `path`, `artifact_id` or `run_id` as the evidence"
        );
        let wanted_item = text_arg(args, "item")
            .context("Pass `item`: the figure, table or section it is for")?;
        let kind = match text_arg(args, "item_kind") {
            Some(kind) => Some(item_kind(kind)?),
            None => None,
        };
        let purpose = text_arg(args, "purpose").unwrap_or_default();
        ensure!(
            purpose.chars().count() <= PURPOSE_LIMIT,
            "`purpose` is at most {PURPOSE_LIMIT} characters"
        );
        // Everything is checked before anything is written, so a refused
        // call leaves no empty paper and no stray result behind.
        let choice = self.find_paper(text_arg(args, "paper")).await?;
        if let PaperChoice::Existing(paper) = &choice {
            ensure_draft(&self.store, paper).await?;
        }
        let source = match (path, artifact, run) {
            (Some(path), _, _) => {
                let file = self.register_file(root, path).await?;
                Source {
                    kind: EvidenceSourceKind::ArtifactVersion,
                    id: file.version_id,
                    artifact: Some(file.artifact_id),
                    name: file.name,
                    path: Some(file.path),
                    registered: file.registered,
                }
            }
            (_, Some(artifact), _) => {
                let found = self
                    .store
                    .get_latest_artifact_version_context(artifact)
                    .await?
                    .filter(|found| found.project_id == self.project())
                    .context("That result is not in this project")?;
                Source {
                    kind: EvidenceSourceKind::ArtifactVersion,
                    id: found.version.id,
                    artifact: Some(found.version.artifact_id),
                    name: found.filename,
                    path: found
                        .logical_key
                        .as_deref()
                        .and_then(|key| key.strip_prefix("path:"))
                        .map(str::to_owned),
                    registered: false,
                }
            }
            (_, _, Some(run)) => {
                let found = self
                    .store
                    .get_run(run)
                    .await?
                    .filter(|found| found.project_id == self.project())
                    .context("That run is not in this project")?;
                Source {
                    kind: EvidenceSourceKind::Run,
                    id: found.id,
                    artifact: None,
                    name: found.title,
                    path: None,
                    registered: false,
                }
            }
            _ => unreachable!("exactly one source was checked above"),
        };
        let (paper, paper_created) = match choice {
            PaperChoice::Existing(paper) => (paper, false),
            PaperChoice::New(title) => (self.start_paper(&title).await?, true),
        };
        let revision = self.draft_of(&paper).await?;
        let (item, item_created) = self.item_for(&revision.id, wanted_item, kind).await?;

        let bound: Vec<EvidenceBinding> = self
            .store
            .list_evidence_bindings(&revision.id)
            .await?
            .into_iter()
            .filter(|binding| binding.item_id.as_deref() == Some(&item.id))
            .collect();
        let same = bound
            .iter()
            .find(|binding| binding.source_kind == source.kind && binding.source_id == source.id);
        // The same file bound at an older content: this call supersedes it.
        let older: Vec<&EvidenceBinding> = bound
            .iter()
            .filter(|binding| {
                binding.source_kind == EvidenceSourceKind::ArtifactVersion
                    && binding.source_id != source.id
                    && source.artifact.is_some()
                    && snapshot_text(binding, "artifact_id") == source.artifact
            })
            .collect();
        // A reason given earlier stays when this call gives none.
        let purpose = if purpose.is_empty() {
            same.or(older.first().copied())
                .map(|binding| binding.purpose.clone())
                .unwrap_or_default()
        } else {
            purpose.to_owned()
        };
        let binding = self
            .store
            .save_evidence_binding(&EvidenceBindingDraft {
                id: same
                    .map(|binding| binding.id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                revision_id: revision.id.clone(),
                item_id: Some(item.id.clone()),
                source_kind: source.kind,
                source_id: source.id.clone(),
                purpose,
                supported_claim_item_id: same
                    .and_then(|binding| binding.supported_claim_item_id.clone()),
                selection_state: same.map_or(EvidenceSelectionState::Selected, |binding| {
                    binding.selection_state
                }),
                visibility: same.map_or(EvidenceVisibility::Private, |binding| binding.visibility),
            })
            .await?;
        for binding in &older {
            self.store.delete_evidence_binding(&binding.id).await?;
        }
        Ok(json!({
            "paper": { "id": paper.id, "title": paper.title, "created": paper_created },
            "version": { "id": revision.id, "label": revision.label },
            "item": {
                "id": item.id,
                "kind": item.kind.as_str(),
                "title": item.title,
                "created": item_created,
            },
            "evidence": {
                "binding_id": binding.id,
                "kind": source.kind.as_str(),
                "name": source.name,
                "path": source.path,
                "purpose": binding.purpose,
                "already_set": same.is_some(),
                "registered_new_version": source.registered,
                "replaced_older_versions": older.len(),
            },
        }))
    }

    async fn remove(&self, args: &Value) -> Result<Value> {
        self.require_mainline()?;
        let id = text_arg(args, "binding_id")
            .context("Pass `binding_id` (from `list`) of the evidence to remove")?;
        let binding = self
            .store
            .get_evidence_binding(id)
            .await?
            .context("That evidence is not set")?;
        let revision = self
            .store
            .get_publication_revision(&binding.revision_id)
            .await?
            .context("That evidence is not set")?;
        self.store
            .get_publication(&revision.publication_id)
            .await?
            .filter(|paper| paper.project_id == self.project())
            .context("That evidence is not in this project")?;
        ensure!(
            revision.state == PublicationRevisionState::Draft,
            "Evidence of a finalized version cannot be removed"
        );
        self.store.delete_evidence_binding(id).await?;
        Ok(json!({ "removed": id }))
    }
}

enum PaperChoice {
    Existing(Publication),
    /// The title of a paper to start.
    New(String),
}

/// The paper's newest version when it is a draft, `None` when the paper
/// has no version yet; a finalized newest version is refused.
async fn ensure_draft(store: &Store, paper: &Publication) -> Result<Option<PublicationRevision>> {
    match newest_revision(store, &paper.id).await? {
        None => Ok(None),
        Some(revision) if revision.state == PublicationRevisionState::Draft => Ok(Some(revision)),
        Some(revision) => bail!(
            "The newest version of “{}” ({}) is finalized and cannot be changed. Start a new \
             version on the paper's page first.",
            paper.title,
            revision.label
        ),
    }
}

struct RegisteredFile {
    version_id: String,
    artifact_id: String,
    name: String,
    path: String,
    registered: bool,
}

struct Source {
    kind: EvidenceSourceKind,
    /// The exact artifact version, or the run.
    id: String,
    artifact: Option<String>,
    name: String,
    path: Option<String>,
    registered: bool,
}

async fn newest_revision(store: &Store, paper: &str) -> Result<Option<PublicationRevision>> {
    Ok(store
        .list_publication_revisions(paper)
        .await?
        .into_iter()
        .max_by_key(|revision| revision.revision_number))
}

fn same_title(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn title(text: &str) -> Result<String> {
    let text = text.trim();
    ensure!(!text.is_empty(), "A title is needed");
    ensure!(
        text.chars().count() <= TITLE_LIMIT,
        "A title is at most {TITLE_LIMIT} characters"
    );
    Ok(text.to_owned())
}

fn item_kind(kind: &str) -> Result<PublicationItemKind> {
    Ok(match kind {
        "section" => PublicationItemKind::Section,
        "claim" => PublicationItemKind::Claim,
        "figure" => PublicationItemKind::Figure,
        "table" => PublicationItemKind::Table,
        "methods" => PublicationItemKind::Methods,
        "supplement" => PublicationItemKind::Supplement,
        other => bail!(
            "Unknown item_kind “{other}”; use figure, table, supplement, section, claim or methods"
        ),
    })
}

/// A non-empty string argument, trimmed.
fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// A text field of what the binding kept of its source when it was set.
fn snapshot_text(binding: &EvidenceBinding, key: &str) -> Option<String> {
    serde_json::from_str::<Value>(&binding.source_snapshot_json)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

fn evidence_row(binding: &EvidenceBinding) -> Value {
    json!({
        "binding_id": binding.id,
        "kind": binding.source_kind.as_str(),
        "name": snapshot_text(binding, "filename").or_else(|| snapshot_text(binding, "title")),
        "path": snapshot_text(binding, "logical_key")
            .and_then(|key| key.strip_prefix("path:").map(str::to_owned)),
        "purpose": binding.purpose,
        "selection": binding.selection_state.as_str(),
    })
}

#[async_trait::async_trait]
impl Tool for PaperEvidenceTool {
    fn name(&self) -> &str {
        "paper_evidence"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "paper_evidence",
            "Manage what the project's paper rests on: its figures, tables and their source \
             data. Use `set` when the user asks to set, mark or keep a file, figure, table, \
             source data or run as evidence or material of their paper: it registers the \
             project file at `path` at the content it has now (or takes an already registered \
             result by `artifact_id`, or a run by `run_id`) and binds it to `item`, the figure, \
             table or section of the paper it is for, adding the paper or the item when the \
             title is new. Setting the same file again after it changed replaces the older \
             content. `list` shows every paper with its items and evidence; `remove` unbinds \
             one by `binding_id`. Call `list` first when unsure which paper or item is meant; \
             never ask the user to look a file up on the paper's page.",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list", "set", "remove"] },
                    "path": {
                        "type": "string",
                        "description": "A file in the project folder, relative to it or absolute."
                    },
                    "artifact_id": {
                        "type": "string",
                        "description": "A registered result, instead of `path`."
                    },
                    "run_id": {
                        "type": "string",
                        "description": "A run, instead of `path`."
                    },
                    "paper": {
                        "type": "string",
                        "description": "The paper's title or id. Optional when the project has one paper; a new title starts that paper."
                    },
                    "item": {
                        "type": "string",
                        "description": "Title or id of the figure, table or section the evidence is for, e.g. \"Figure 1\". Added when the draft does not have it."
                    },
                    "item_kind": {
                        "type": "string",
                        "enum": ["figure", "table", "supplement", "section", "claim", "methods"],
                        "description": "Kind of a newly added item; figure when omitted."
                    },
                    "purpose": {
                        "type": "string",
                        "description": "What the evidence is there, and why it was chosen, e.g. \"Panel B image\" or \"Source data of panel B\"."
                    },
                    "binding_id": {
                        "type": "string",
                        "description": "For `remove`: the evidence's binding_id from `list`."
                    }
                },
                "required": ["action"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        let action = text_arg(args, "action").unwrap_or_default();
        let source = ["path", "artifact_id", "run_id", "binding_id"]
            .into_iter()
            .find_map(|key| text_arg(args, key));
        match (source, text_arg(args, "item")) {
            (Some(source), Some(item)) => format!("{action} {source} → {item}"),
            (Some(source), None) => format!("{action} {source}"),
            _ => action.to_owned(),
        }
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let result = match text_arg(args, "action") {
            Some("list") => self.list().await,
            Some("set") => self.set(args, env.project_root()).await,
            Some("remove") => self.remove(args).await,
            _ => Err(anyhow!(
                "paper_evidence requires action: list, set or remove"
            )),
        };
        match result {
            Ok(value) => ToolResult::ok(
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            ),
            Err(error) => ToolResult::fail(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_tools::ToolEvent;

    struct TestEnv(PathBuf);

    #[async_trait::async_trait]
    impl ToolEnv for TestEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    struct Fixture {
        store: Store,
        root: PathBuf,
        tool: PaperEvidenceTool,
        env: TestEnv,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    async fn fixture() -> Fixture {
        let root =
            std::env::temp_dir().join(format!("wisp_paper_evidence_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("figures")).unwrap();
        let store = Store::open(Path::new(":memory:")).await.unwrap();
        store
            .create_project("p1", "p1", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("f1", "p1", "main", "model")
            .await
            .unwrap();
        let tool = PaperEvidenceTool::new_in_scope(store.clone(), StateScope::mainline("p1"), "f1");
        Fixture {
            store,
            env: TestEnv(root.clone()),
            root,
            tool,
        }
    }

    async fn call(fixture: &Fixture, args: Value) -> (bool, Value) {
        let result = fixture.tool.run(&args, &fixture.env).await;
        let value = serde_json::from_str(&result.content)
            .unwrap_or_else(|_| Value::String(result.content.clone()));
        (result.success, value)
    }

    async fn only_draft(fixture: &Fixture) -> PublicationRevision {
        let papers = fixture.store.list_publications("p1").await.unwrap();
        assert_eq!(papers.len(), 1);
        let revisions = fixture
            .store
            .list_publication_revisions(&papers[0].id)
            .await
            .unwrap();
        assert_eq!(revisions.len(), 1);
        revisions.into_iter().next().unwrap()
    }

    #[tokio::test]
    async fn a_file_path_becomes_evidence_of_a_new_paper_and_figure() {
        let fixture = fixture().await;
        std::fs::write(fixture.root.join("figures/fig1.png"), b"png-bytes").unwrap();

        let (ok, set) = call(
            &fixture,
            json!({
                "action": "set",
                "path": "figures/fig1.png",
                "paper": "Estrogen paper",
                "item": "Figure 1",
                "purpose": "Panel A image",
            }),
        )
        .await;

        assert!(ok, "{set}");
        assert_eq!(set["paper"]["created"], true);
        assert_eq!(set["item"]["created"], true);
        assert_eq!(set["item"]["kind"], "figure");
        assert_eq!(set["evidence"]["path"], "figures/fig1.png");
        assert_eq!(set["evidence"]["registered_new_version"], true);
        let revision = only_draft(&fixture).await;
        let bindings = fixture
            .store
            .list_evidence_bindings(&revision.id)
            .await
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].source_kind, EvidenceSourceKind::ArtifactVersion);
        assert_eq!(bindings[0].purpose, "Panel A image");
        assert_eq!(
            bindings[0].selection_state,
            EvidenceSelectionState::Selected
        );

        let (ok, listed) = call(&fixture, json!({ "action": "list" })).await;
        assert!(ok);
        let evidence = &listed["papers"][0]["items"][0]["evidence"][0];
        assert_eq!(evidence["name"], "fig1.png");
        assert_eq!(evidence["path"], "figures/fig1.png");
    }

    #[tokio::test]
    async fn setting_an_unchanged_file_again_keeps_one_binding_and_its_reason() {
        let fixture = fixture().await;
        std::fs::write(fixture.root.join("figures/fig1.png"), b"png-bytes").unwrap();
        let args = json!({
            "action": "set",
            "path": "figures/fig1.png",
            "paper": "Paper",
            "item": "Figure 1",
            "purpose": "Panel A image",
        });
        let (_, first) = call(&fixture, args.clone()).await;

        // The only paper is meant, the item is found whatever its case, and
        // the reason given before stays.
        let (ok, second) = call(
            &fixture,
            json!({ "action": "set", "path": "figures/fig1.png", "item": "figure 1" }),
        )
        .await;

        assert!(ok, "{second}");
        assert_eq!(second["evidence"]["already_set"], true);
        assert_eq!(second["evidence"]["registered_new_version"], false);
        assert_eq!(
            second["evidence"]["binding_id"],
            first["evidence"]["binding_id"]
        );
        assert_eq!(second["evidence"]["purpose"], "Panel A image");
        assert_eq!(second["item"]["created"], false);
        let revision = only_draft(&fixture).await;
        assert_eq!(
            fixture
                .store
                .list_publication_items(&revision.id)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn a_changed_file_replaces_its_older_content_on_the_same_item() {
        let fixture = fixture().await;
        let file = fixture.root.join("figures/fig1.png");
        std::fs::write(&file, b"first").unwrap();
        let args = json!({
            "action": "set",
            "path": "figures/fig1.png",
            "paper": "Paper",
            "item": "Figure 1",
            "purpose": "Panel A image",
        });
        let (_, first) = call(&fixture, args.clone()).await;
        std::fs::write(&file, b"second, regenerated").unwrap();

        let (ok, second) = call(
            &fixture,
            json!({ "action": "set", "path": "figures/fig1.png", "item": "Figure 1" }),
        )
        .await;

        assert!(ok, "{second}");
        assert_eq!(second["evidence"]["registered_new_version"], true);
        assert_eq!(second["evidence"]["replaced_older_versions"], 1);
        assert_eq!(second["evidence"]["purpose"], "Panel A image");
        assert_ne!(
            second["evidence"]["binding_id"],
            first["evidence"]["binding_id"]
        );
        let revision = only_draft(&fixture).await;
        let bindings = fixture
            .store
            .list_evidence_bindings(&revision.id)
            .await
            .unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            snapshot_text(&bindings[0], "filename").as_deref(),
            Some("fig1.png")
        );
    }

    #[tokio::test]
    async fn source_data_joins_the_figure_it_belongs_to() {
        let fixture = fixture().await;
        std::fs::write(fixture.root.join("figures/fig1.png"), b"png").unwrap();
        std::fs::write(fixture.root.join("figures/fig1_source.csv"), b"a,b\n1,2\n").unwrap();
        call(
            &fixture,
            json!({ "action": "set", "path": "figures/fig1.png", "paper": "Paper", "item": "Figure 1" }),
        )
        .await;

        let (ok, set) = call(
            &fixture,
            json!({
                "action": "set",
                "path": "figures/fig1_source.csv",
                "item": "Figure 1",
                "purpose": "Source data of panel A",
            }),
        )
        .await;

        assert!(ok, "{set}");
        let (_, listed) = call(&fixture, json!({ "action": "list" })).await;
        let evidence = listed["papers"][0]["items"][0]["evidence"]
            .as_array()
            .unwrap();
        assert_eq!(evidence.len(), 2);
        assert!(evidence
            .iter()
            .any(|row| row["path"] == "figures/fig1_source.csv"
                && row["purpose"] == "Source data of panel A"));
    }

    #[tokio::test]
    async fn a_refused_file_leaves_no_paper_behind() {
        let fixture = fixture().await;
        let outside = std::env::temp_dir().join(format!("wisp_outside_{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, b"elsewhere").unwrap();

        let (missing, _) = call(
            &fixture,
            json!({ "action": "set", "path": "figures/none.png", "paper": "Paper", "item": "Figure 1" }),
        )
        .await;
        let (elsewhere, refused) = call(
            &fixture,
            json!({
                "action": "set",
                "path": outside.to_string_lossy(),
                "paper": "Paper",
                "item": "Figure 1",
            }),
        )
        .await;
        std::fs::remove_file(&outside).ok();

        assert!(!missing);
        assert!(!elsewhere);
        assert!(refused
            .as_str()
            .unwrap()
            .contains("outside the project folder"));
        assert!(fixture
            .store
            .list_publications("p1")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn the_paper_must_be_named_when_there_is_none_or_several() {
        let fixture = fixture().await;
        std::fs::write(fixture.root.join("figures/fig1.png"), b"png").unwrap();
        let unnamed = json!({ "action": "set", "path": "figures/fig1.png", "item": "Figure 1" });

        let (none, _) = call(&fixture, unnamed.clone()).await;
        for name in ["First", "Second"] {
            let (ok, _) = call(
                &fixture,
                json!({ "action": "set", "path": "figures/fig1.png", "paper": name, "item": "Figure 1" }),
            )
            .await;
            assert!(ok);
        }
        let (several, refused) = call(&fixture, unnamed).await;

        assert!(!none);
        assert!(!several);
        assert!(refused.as_str().unwrap().contains("“First”"));
        assert_eq!(
            fixture.store.list_publications("p1").await.unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn evidence_is_removed_by_its_binding_and_only_in_this_project() {
        let fixture = fixture().await;
        std::fs::write(fixture.root.join("figures/fig1.png"), b"png").unwrap();
        let (_, set) = call(
            &fixture,
            json!({ "action": "set", "path": "figures/fig1.png", "paper": "Paper", "item": "Figure 1" }),
        )
        .await;
        let binding = set["evidence"]["binding_id"].as_str().unwrap().to_owned();
        fixture
            .store
            .create_project("p2", "p2", &fixture.root.to_string_lossy())
            .await
            .unwrap();
        let other = PaperEvidenceTool::new_in_scope(
            fixture.store.clone(),
            StateScope::mainline("p2"),
            "f1",
        );

        let foreign = other
            .run(
                &json!({ "action": "remove", "binding_id": binding }),
                &fixture.env,
            )
            .await;
        let (removed, _) = call(
            &fixture,
            json!({ "action": "remove", "binding_id": binding }),
        )
        .await;

        assert!(!foreign.success);
        assert!(removed);
        let revision = only_draft(&fixture).await;
        assert!(fixture
            .store
            .list_evidence_bindings(&revision.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn an_exploration_cannot_set_paper_evidence() {
        let fixture = fixture().await;
        std::fs::write(fixture.root.join("figures/fig1.png"), b"png").unwrap();
        let tool = PaperEvidenceTool::new_in_scope(
            fixture.store.clone(),
            StateScope::exploration("p1", "e1"),
            "f1",
        );

        let result = tool
            .run(
                &json!({ "action": "set", "path": "figures/fig1.png", "paper": "Paper", "item": "Figure 1" }),
                &fixture.env,
            )
            .await;

        assert!(!result.success);
        assert!(fixture
            .store
            .list_publications("p1")
            .await
            .unwrap()
            .is_empty());
    }
}
