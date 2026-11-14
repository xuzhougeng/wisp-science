//! Operations for the desktop process that embeds this host: conversation
//! management and the model it runs on. They are typed methods rather than
//! dispatch commands, so nothing here can be reached through the remote
//! allowlist.
pub use crate::output::{tool_question, TokenUsage};
use crate::{
    config::{default_api_url, ModelConfig},
    Server,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use wisp_dto::SessionSearchInfo;
use wisp_llm::ProviderKind;
use wisp_store::Store;

const MODEL_SETTING: &str = "desktop.model";
/// Names this database's key in the secret store. Keys are per data
/// directory, like the settings they belong to.
const MODEL_KEY_ID: &str = "desktop.model.key_id";
const TITLE_LIMIT: usize = 200;

/// Where API keys live. The key never enters SQLite or a snapshot.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
}

/// The OS keyring path shared with the rest of Wisp.
pub struct Keyring;

impl SecretStore for Keyring {
    fn get(&self, name: &str) -> Option<String> {
        wisp_store::secrets::Secret::get(name)
            .ok()
            .filter(|value| !value.is_empty())
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        wisp_store::secrets::Secret::set(name, value)
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct MemorySecrets(std::sync::Mutex<std::collections::HashMap<String, String>>);

#[cfg(test)]
impl SecretStore for MemorySecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.0.lock().unwrap().get(name).cloned()
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(name.into(), value.into());
        Ok(())
    }
}

/// The non-secret model fields, as typed in the settings form. Blank limits
/// mean "use the catalog"; a blank URL means the provider's default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelFields {
    pub provider: String,
    pub api_url: String,
    pub model: String,
    pub max_context: String,
    pub max_tokens: String,
    pub reasoning_effort: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSource {
    /// Nothing usable is configured yet.
    None,
    /// `WISP_*` variables; they take precedence and cannot be edited here.
    Environment,
    Saved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSettings {
    pub fields: ModelFields,
    pub has_api_key: bool,
    pub source: ModelSource,
    /// Why the saved settings are not in use, if they are not.
    pub problem: Option<String>,
}

pub struct ModelInput {
    pub fields: ModelFields,
    /// `None` keeps the stored key.
    pub api_key: Option<String>,
}

pub(crate) struct ModelState {
    pub active: Option<ModelConfig>,
    source: ModelSource,
    problem: Option<String>,
}

fn resolve(fields: &ModelFields, key: &str) -> Result<ModelConfig> {
    ensure!(!fields.model.trim().is_empty(), "Enter a model ID");
    ensure!(!key.trim().is_empty(), "Enter an API key");
    ModelConfig::resolve(|name| {
        let value = match name {
            "WISP_PROVIDER" => &fields.provider,
            "WISP_API_URL" => &fields.api_url,
            "WISP_MODEL" => &fields.model,
            "WISP_API_KEY" => key,
            "WISP_REASONING_EFFORT" => &fields.reasoning_effort,
            "WISP_MAX_CONTEXT" => &fields.max_context,
            "WISP_MAX_TOKENS" => &fields.max_tokens,
            _ => return None,
        };
        Some(value.trim().to_owned())
    })
    .map_err(|error| {
        // The shared validation names the server's environment variables.
        let text = format!("{error:#}");
        if text.contains("exact-ID catalog") {
            anyhow::anyhow!(
                "This model is not in the catalog; enter its context window and output limit"
            )
        } else if text.contains("WISP_MAX_CONTEXT") {
            anyhow::anyhow!("Context window must be a positive number")
        } else if text.contains("WISP_MAX_TOKENS") {
            anyhow::anyhow!("Output limit must be a positive number")
        } else {
            error
        }
    })
}

fn provider_name(kind: &ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenAiResponses => "openai_responses",
        _ => "openai",
    }
}

impl ModelState {
    pub fn environment(model: ModelConfig) -> Self {
        Self {
            active: Some(model),
            source: ModelSource::Environment,
            problem: None,
        }
    }

    /// Saved settings that no longer validate leave the host unconfigured and
    /// say why, instead of failing startup.
    pub async fn saved(store: &Store, secrets: &dyn SecretStore) -> Self {
        let unconfigured = |problem| Self {
            active: None,
            source: ModelSource::None,
            problem,
        };
        let fields = match saved_fields(store).await {
            Ok(Some(fields)) => fields,
            Ok(None) => return unconfigured(None),
            Err(error) => return unconfigured(Some(format!("{error:#}"))),
        };
        let Some(key) = saved_key(store, secrets).await else {
            return unconfigured(Some("The saved API key is missing; enter it again".into()));
        };
        match resolve(&fields, &key) {
            Ok(model) => Self {
                active: Some(model),
                source: ModelSource::Saved,
                problem: None,
            },
            Err(error) => unconfigured(Some(format!("{error:#}"))),
        }
    }
}

async fn saved_fields(store: &Store) -> Result<Option<ModelFields>> {
    match store.get_setting(MODEL_SETTING).await? {
        Some(raw) => Ok(Some(
            serde_json::from_str(&raw).context("Saved model settings are unreadable")?,
        )),
        None => Ok(None),
    }
}

fn key_name(id: &str) -> String {
    format!("desktop.model.api_key.{id}")
}

async fn saved_key(store: &Store, secrets: &dyn SecretStore) -> Option<String> {
    let id = store.get_setting(MODEL_KEY_ID).await.ok()??;
    secrets.get(&key_name(&id))
}

/// Conversations of one project for the sidebar.
#[derive(Clone)]
pub struct ConversationList {
    pub rows: Vec<SessionSearchInfo>,
    /// IDs the user pinned, to be shown first.
    pub pinned: Vec<String>,
}

impl Server {
    pub async fn model_settings(&self) -> Result<ModelSettings> {
        let (source, problem, active) = {
            let state = self.0.model.read().unwrap();
            (state.source, state.problem.clone(), state.active.clone())
        };
        if let (ModelSource::Environment, Some(model)) = (source, &active) {
            let provider = provider_name(&model.provider.kind);
            return Ok(ModelSettings {
                fields: ModelFields {
                    provider: provider.into(),
                    api_url: model.provider.base_url.clone(),
                    model: model.provider.model.clone(),
                    max_context: model.max_context.to_string(),
                    max_tokens: model.provider.max_tokens.to_string(),
                    reasoning_effort: model.provider.reasoning_effort.clone().unwrap_or_default(),
                },
                has_api_key: true,
                source,
                problem: None,
            });
        }
        let mut fields = saved_fields(&self.0.store).await?.unwrap_or_default();
        if fields.provider.is_empty() {
            fields.provider = "openai".into();
        }
        if fields.api_url.is_empty() {
            fields.api_url = default_api_url(&fields.provider).into();
        }
        Ok(ModelSettings {
            fields,
            has_api_key: saved_key(&self.0.store, self.0.secrets.as_ref())
                .await
                .is_some(),
            source,
            problem,
        })
    }

    /// Validates first, so a rejected form never replaces working settings.
    /// Turns already running keep the model they started with.
    pub async fn save_model_settings(&self, input: ModelInput) -> Result<ModelSettings> {
        ensure!(
            self.0.model.read().unwrap().source != ModelSource::Environment,
            "The model is set by WISP_* environment variables; unset them to edit it here"
        );
        // Report the first empty field first, in the order the form shows them.
        ensure!(!input.fields.model.trim().is_empty(), "Enter a model ID");
        let typed = input
            .api_key
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty());
        let key = match &typed {
            Some(key) => key.clone(),
            None => saved_key(&self.0.store, self.0.secrets.as_ref())
                .await
                .context("Enter an API key")?,
        };
        let model = resolve(&input.fields, &key)?;
        if let Some(key) = &typed {
            let id = match self.0.store.get_setting(MODEL_KEY_ID).await? {
                Some(id) => id,
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    self.0.store.set_setting(MODEL_KEY_ID, &id).await?;
                    id
                }
            };
            self.0.secrets.set(&key_name(&id), key)?;
        }
        self.0
            .store
            .set_setting(MODEL_SETTING, &serde_json::to_string(&input.fields)?)
            .await?;
        *self.0.model.write().unwrap() = ModelState {
            active: Some(model),
            source: ModelSource::Saved,
            problem: None,
        };
        self.model_settings().await
    }

    /// Newest conversations across every project, for the home page.
    pub async fn recent_conversations(&self, limit: usize) -> Result<Vec<SessionSearchInfo>> {
        let mut rows = self.session_rows(None, "", false).await?;
        rows.truncate(limit);
        Ok(rows)
    }

    /// Conversations in any project whose title or messages match `query`,
    /// archived ones included. An empty query lists the newest instead.
    pub async fn find_conversations(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchInfo>> {
        let query = query.trim();
        let mut rows = self.session_rows(None, query, !query.is_empty()).await?;
        rows.truncate(limit);
        Ok(rows)
    }

    /// Files by name, in one project or in every project whose folder is
    /// still there. At most `limit` matches per project.
    pub async fn find_files(
        &self,
        project: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FileMatch>> {
        let mut matches = Vec::new();
        for (id, name, root, ..) in self.0.store.list_projects().await? {
            if project.is_some_and(|wanted| wanted != id) {
                continue;
            }
            // A project whose folder was moved away simply has no matches.
            let Ok(paths) =
                crate::files::search(std::path::Path::new(&root), query, limit, SEARCH_BUDGET)
                    .await
            else {
                continue;
            };
            matches.extend(paths.into_iter().map(|path| FileMatch {
                project_id: id.clone(),
                project_name: name.clone(),
                path,
            }));
        }
        Ok(matches)
    }

    async fn project_files(
        &self,
        project: &str,
        command: &str,
        path: &str,
    ) -> Result<serde_json::Value> {
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        let (_, root) = self
            .0
            .store
            .get_project(project)
            .await?
            .context("Project not found")?;
        let request =
            serde_json::from_value(serde_json::json!({"session_id": null, "path": path}))?;
        crate::files::read(project, &root, command, request).await
    }

    /// One folder of the project. Unlike the remote file commands this needs
    /// no conversation: the desktop user owns the folder they opened.
    pub async fn project_directory(
        &self,
        project: &str,
        path: &str,
    ) -> Result<wisp_dto::native_files::Directory> {
        let value = self
            .project_files(project, "native_project_file_directory", path)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    /// The first megabyte of a project file, under the same confinement.
    pub async fn project_file(
        &self,
        project: &str,
        path: &str,
    ) -> Result<wisp_dto::native_files::Preview> {
        let value = self
            .project_files(project, "native_project_file_read", path)
            .await?;
        Ok(serde_json::from_value(value)?)
    }

    async fn known_project(&self, project: &str) -> Result<()> {
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        ensure!(
            self.0.store.get_project(project).await?.is_some(),
            "Project not found"
        );
        Ok(())
    }

    /// A starred project stays first in the project list.
    pub async fn star_project(&self, project: &str, starred: bool) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        wisp_app::projects::set_project_starred(&self.0.store, project, starred).await
    }

    /// Removes the project and its conversations from Wisp. The folder and
    /// every file in it are left exactly as they are.
    pub async fn remove_project(&self, project: &str) -> Result<()> {
        // Exclusive: no turn may be admitted while the project's rows go away.
        let _admission = self.0.admission.write().await;
        self.known_project(project).await?;
        ensure!(
            removable(project),
            "The built-in workspace cannot be removed"
        );
        let frames = self.0.store.list_project_frame_ids(project).await?;
        let busy = {
            let sessions = self.0.sessions.lock().unwrap();
            frames.iter().any(|id| {
                sessions
                    .get(id)
                    .is_some_and(|session| session.live.lock().unwrap().running)
            })
        };
        ensure!(
            !busy,
            "Stop the running turns in this project before you remove it"
        );
        self.0.store.delete_project(project).await?;
        let closed: Vec<_> = {
            let mut sessions = self.0.sessions.lock().unwrap();
            frames.iter().filter_map(|id| sessions.remove(id)).collect()
        };
        for session in closed {
            let runtime = session.runtime.lock().unwrap().take();
            if let Some(runtime) = runtime {
                runtime.shutdown_all().await;
            }
        }
        Ok(())
    }

    async fn own_conversation(&self, project: &str, session: &str) -> Result<()> {
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        ensure!(
            self.0
                .store
                .live_frame_project_id(session)
                .await?
                .as_deref()
                == Some(project),
            "Conversation does not belong to the selected project"
        );
        Ok(())
    }

    /// Recent conversations, keyword matches (titles and messages, archived
    /// ones included), or the archive itself.
    pub async fn conversations(
        &self,
        project: &str,
        query: &str,
        archived: bool,
    ) -> Result<ConversationList> {
        let query = query.trim();
        let rows = if archived {
            self.0
                .store
                .list_sessions_page_with_visibility(project, None, 100, Some(true), query)
                .await?
                .into_iter()
                .map(|(id, title, ts, ..)| SessionSearchInfo {
                    id,
                    project_id: project.into(),
                    project_name: String::new(),
                    title,
                    ts,
                    activity_at: ts,
                    status: "complete".into(),
                    shelved: true,
                })
                .collect()
        } else {
            self.session_rows(Some(project), query, !query.is_empty())
                .await?
        };
        let pinned = self
            .0
            .store
            .list_pinned_sessions(project)
            .await?
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        Ok(ConversationList { rows, pinned })
    }

    pub async fn rename_conversation(
        &self,
        project: &str,
        session: &str,
        title: &str,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        let title = title.trim();
        ensure!(!title.is_empty(), "Enter a title");
        ensure!(
            title.chars().count() <= TITLE_LIMIT,
            "Titles are limited to {TITLE_LIMIT} characters"
        );
        self.0.store.rename_session(session, project, title).await
    }

    pub async fn pin_conversation(&self, project: &str, session: &str, pinned: bool) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.0
            .store
            .set_session_pinned(session, project, pinned)
            .await
    }

    fn ensure_idle(&self, session: &str, action: &str) -> Result<()> {
        if self.session(session).live.lock().unwrap().running {
            bail!("Stop the running turn before you {action} this conversation");
        }
        Ok(())
    }

    /// Archived conversations leave everyday lists but stay searchable.
    pub async fn archive_conversation(
        &self,
        project: &str,
        session: &str,
        archived: bool,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        if archived {
            self.ensure_idle(session, "archive")?;
        }
        self.0
            .store
            .set_session_shelved(session, project, archived)
            .await
    }

    /// What the next turn of this conversation will run with.
    pub(crate) async fn stored_options(&self, session: &str) -> Result<ConversationOptions> {
        Ok(ConversationOptions {
            full_access: self
                .0
                .store
                .get_setting(&full_access_key(session))
                .await?
                .as_deref()
                == Some("1"),
            reasoning_effort: self
                .0
                .store
                .frame_reasoning_effort(session)
                .await?
                .unwrap_or_default(),
        })
    }

    /// The conversation's options, what its latest turn used, and the tools
    /// it may currently run without asking.
    pub async fn conversation_status(
        &self,
        project: &str,
        session: &str,
    ) -> Result<ConversationStatus> {
        self.own_conversation(project, session).await?;
        let options = self.stored_options(session).await?;
        let session = self.session(session);
        let live = session.live.lock().unwrap();
        let mut allowed_tools: Vec<_> = live.allowed_tools.iter().cloned().collect();
        allowed_tools.sort();
        Ok(ConversationStatus {
            options,
            usage: live.usage.clone(),
            allowed_tools,
        })
    }

    /// Saves the conversation's options. They apply from its next turn; a
    /// turn that is already running keeps what it started with.
    pub async fn set_conversation_options(
        &self,
        project: &str,
        session: &str,
        options: ConversationOptions,
    ) -> Result<ConversationOptions> {
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        let effort = options.reasoning_effort.trim();
        ensure!(
            EFFORTS.contains(&effort),
            "Choose one of the listed reasoning efforts"
        );
        self.0
            .store
            .set_frame_reasoning_effort(session, project, (!effort.is_empty()).then_some(effort))
            .await?;
        let key = full_access_key(session);
        if options.full_access {
            self.0.store.set_setting(&key, "1").await?;
        } else {
            self.0.store.delete_setting(&key).await?;
        }
        self.stored_options(session).await
    }

    /// Answers a pending approval. With `for_conversation` the same tool
    /// also runs without asking for the rest of this conversation; only the
    /// registry's standard question can be answered that way.
    pub async fn answer_approval(
        &self,
        project: &str,
        session: &str,
        approval: &str,
        approved: bool,
        for_conversation: bool,
    ) -> Result<()> {
        self.own_conversation(project, session).await?;
        let session = self.session(session);
        let mut live = session.live.lock().unwrap();
        let (pending, sender) = live
            .approvals
            .remove(approval)
            .context("Approval expired or already answered")?;
        if for_conversation {
            if !approved || pending.message != tool_question(&pending.tool) {
                live.approvals.insert(approval.into(), (pending, sender));
                bail!("This request can only be answered once");
            }
            live.allowed_tools.insert(pending.tool.clone());
        }
        sender
            .send(if approved {
                wisp_tools::ConfirmDecision::Approved
            } else {
                wisp_tools::ConfirmDecision::Denied { feedback: None }
            })
            .map_err(|_| anyhow::anyhow!("Approval expired"))
    }

    /// Goes back to asking before every tool that changes something.
    pub async fn revoke_allowed_tools(&self, project: &str, session: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        self.session(session)
            .live
            .lock()
            .unwrap()
            .allowed_tools
            .clear();
        Ok(())
    }

    /// Permanently removes the conversation and its messages. Project files
    /// the conversation produced are left in place.
    pub async fn delete_conversation(&self, project: &str, session: &str) -> Result<()> {
        let live = self.session(session);
        // Acceptance takes the same lock, so no turn can start mid-deletion.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.ensure_idle(session, "delete")?;
        self.0.store.delete_session(session, project).await?;
        self.0
            .store
            .delete_setting(&full_access_key(session))
            .await?;
        self.0.sessions.lock().unwrap().remove(session);
        let runtime = live.runtime.lock().unwrap().take();
        if let Some(runtime) = runtime {
            runtime.shutdown_all().await;
        }
        Ok(())
    }
}

/// Reasoning efforts a conversation may ask for. Empty is the model's default.
pub const EFFORTS: &[&str] = &[
    "", "none", "minimal", "low", "medium", "high", "xhigh", "max",
];

/// Choices that belong to one conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationOptions {
    /// Run tools that change things without asking first. Dangerous shell
    /// commands still ask.
    pub full_access: bool,
    /// One of [`EFFORTS`].
    pub reasoning_effort: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationStatus {
    pub options: ConversationOptions,
    /// The running or latest turn since the app started; `None` before one.
    pub usage: Option<TokenUsage>,
    /// Tools allowed for the rest of this conversation, sorted.
    pub allowed_tools: Vec<String>,
}

fn full_access_key(session: &str) -> String {
    format!("desktop.conversation.full_access.{session}")
}

/// Entries one file-name search may look at in a project.
const SEARCH_BUDGET: usize = 20_000;

/// A file found by name, with the project it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatch {
    pub project_id: String,
    pub project_name: String,
    /// Project-relative, with `/` separators on every platform.
    pub path: String,
}

/// Whether a project can leave Wisp. The workspace the host was started on
/// is its anchor and stays.
pub fn removable(project: &str) -> bool {
    project != crate::PROJECT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(model: &str) -> ModelFields {
        ModelFields {
            provider: "openai".into(),
            model: model.into(),
            ..Default::default()
        }
    }

    #[test]
    fn form_validation_speaks_in_form_terms() {
        let error = |fields: &ModelFields, key: &str| {
            resolve(fields, key)
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default()
        };
        assert_eq!(error(&fields(" "), "k"), "Enter a model ID");
        assert_eq!(error(&fields("gpt-4o"), " "), "Enter an API key");
        assert!(error(&fields("private-model"), "k").contains("not in the catalog"));
        let mut custom = fields("private-model");
        custom.max_context = "many".into();
        custom.max_tokens = "4000".into();
        assert_eq!(
            error(&custom, "k"),
            "Context window must be a positive number"
        );
        custom.max_context = "32000".into();
        custom.max_tokens = "0".into();
        assert_eq!(
            error(&custom, "k"),
            "Output limit must be a positive number"
        );
        custom.max_tokens = "64000".into();
        assert!(error(&custom, "k").contains("Context must exceed"));
        custom.provider = "typo".into();
        assert!(error(&custom, "k").contains("API-key providers"));
    }

    #[test]
    fn blank_fields_fall_back_to_provider_defaults_and_catalog_limits() {
        let known = resolve(&fields("gpt-4o"), " key ").unwrap();
        assert_eq!(known.provider.api_key, "key");
        assert_eq!(known.provider.base_url, "https://api.openai.com/v1");
        assert!(known.max_context > known.provider.max_tokens as usize);
        assert!(known.provider.reasoning_effort.is_none());

        let mut custom = fields("private-model");
        custom.provider = "anthropic".into();
        custom.max_context = " 32000 ".into();
        custom.max_tokens = "4000".into();
        custom.reasoning_effort = "high".into();
        let model = resolve(&custom, "key").unwrap();
        assert_eq!(provider_name(&model.provider.kind), "anthropic");
        assert_eq!(model.provider.base_url, "https://api.anthropic.com");
        assert_eq!(
            (model.max_context, model.provider.max_tokens),
            (32000, 4000)
        );
        assert_eq!(model.provider.reasoning_effort.as_deref(), Some("high"));
    }
}
