//! Operations for the desktop process that embeds this host: conversation
//! management and the model it runs on. They are typed methods rather than
//! dispatch commands, so nothing here can be reached through the remote
//! allowlist.
pub use crate::output::{tool_question, TokenUsage};
use crate::{config::ModelConfig, Server};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
pub use wisp_dto::native_conversations::ReferenceOption;
use wisp_dto::SessionSearchInfo;
use wisp_llm::ProviderKind;
use wisp_store::Store;

/// The saved model profiles: `{"profiles": [...], "default": "<id>"}`.
const MODELS_SETTING: &str = "desktop.models";
/// The single model earlier builds saved. It is read as a profile with
/// [`LEGACY_PROFILE`] as its id, so nothing has to be migrated.
const LEGACY_SETTING: &str = "desktop.model";
const LEGACY_PROFILE: &str = "default";
/// Id of the read-only profile shown while `WISP_*` variables set the model.
const ENVIRONMENT_PROFILE: &str = "env";
/// Names this database's keys in the secret store. Keys are per data
/// directory, like the settings they belong to.
const MODEL_KEY_ID: &str = "desktop.model.key_id";
const NAME_LIMIT: usize = 60;
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

/// One saved model, as the settings page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub fields: ModelFields,
    pub has_api_key: bool,
    /// New conversations, and conversations that chose nothing, use this one.
    pub is_default: bool,
    /// Why this profile cannot be used as it is saved, if it cannot.
    pub problem: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSettings {
    pub profiles: Vec<ModelProfile>,
    pub source: ModelSource,
}

pub struct ModelInput {
    /// `None` adds a profile; `Some` replaces that one.
    pub id: Option<String>,
    /// Blank means "call it by its model ID".
    pub name: String,
    pub fields: ModelFields,
    /// `None` keeps the stored key.
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct SavedProfile {
    id: String,
    name: String,
    fields: ModelFields,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SavedModels {
    profiles: Vec<SavedProfile>,
    default: String,
}

struct Loaded {
    profile: SavedProfile,
    has_api_key: bool,
    model: std::result::Result<ModelConfig, String>,
}

pub(crate) struct ModelState {
    source: ModelSource,
    profiles: Vec<Loaded>,
    default: String,
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
        let fields = ModelFields {
            provider: provider_name(&model.provider.kind).into(),
            api_url: model.provider.base_url.clone(),
            model: model.provider.model.clone(),
            max_context: model.max_context.to_string(),
            max_tokens: model.provider.max_tokens.to_string(),
            reasoning_effort: model.provider.reasoning_effort.clone().unwrap_or_default(),
        };
        Self {
            source: ModelSource::Environment,
            profiles: vec![Loaded {
                profile: SavedProfile {
                    id: ENVIRONMENT_PROFILE.into(),
                    name: fields.model.clone(),
                    fields,
                },
                has_api_key: true,
                model: Ok(model),
            }],
            default: ENVIRONMENT_PROFILE.into(),
        }
    }

    /// A profile that no longer validates stays listed with its problem; it
    /// never fails startup.
    pub async fn saved(store: &Store, secrets: &dyn SecretStore) -> Self {
        let saved = match saved_models(store).await {
            Ok(saved) => saved,
            Err(error) => {
                tracing::warn!(%error, "Saved model profiles are unreadable");
                SavedModels::default()
            }
        };
        let key_id = store.get_setting(MODEL_KEY_ID).await.ok().flatten();
        let profiles: Vec<_> = saved
            .profiles
            .into_iter()
            .map(|profile| {
                let key = key_id
                    .as_deref()
                    .and_then(|id| secrets.get(&key_name(id, &profile.id)));
                let model = match &key {
                    Some(key) => resolve(&profile.fields, key).map_err(|e| format!("{e:#}")),
                    None => Err("The saved API key is missing; enter it again".into()),
                };
                Loaded {
                    profile,
                    has_api_key: key.is_some(),
                    model,
                }
            })
            .collect();
        let state = Self {
            source: ModelSource::Saved,
            profiles,
            default: saved.default,
        };
        Self {
            source: if state.pick("").is_ok() {
                ModelSource::Saved
            } else {
                ModelSource::None
            },
            ..state
        }
    }

    fn default_profile(&self) -> Option<&Loaded> {
        self.profiles
            .iter()
            .find(|loaded| loaded.profile.id == self.default)
            .or_else(|| self.profiles.first())
    }

    fn profile(&self, id: &str) -> Option<&Loaded> {
        if id.is_empty() {
            self.default_profile()
        } else {
            self.profiles.iter().find(|loaded| loaded.profile.id == id)
        }
    }

    /// The model for a conversation that chose profile `id`; an empty id is
    /// the default. A choice that was deleted since falls back to the
    /// default, but a broken one is reported rather than silently replaced.
    pub fn pick(&self, id: &str) -> Result<ModelConfig> {
        let loaded = self
            .profile(id)
            .or_else(|| self.default_profile())
            .context("Configure a model in Settings before sending a message")?;
        loaded.model.clone().map_err(|problem| {
            anyhow::anyhow!(
                "The model \"{}\" cannot be used: {problem}",
                loaded.profile.name
            )
        })
    }

    /// The model ID a conversation that chose profile `id` runs on, or
    /// empty when nothing usable is configured.
    pub fn model_id(&self, id: &str) -> String {
        self.pick(id)
            .map(|model| model.provider.model)
            .unwrap_or_default()
    }

    fn settings(&self) -> ModelSettings {
        let default = self
            .default_profile()
            .map(|loaded| loaded.profile.id.clone());
        ModelSettings {
            source: self.source,
            profiles: self
                .profiles
                .iter()
                .map(|loaded| ModelProfile {
                    id: loaded.profile.id.clone(),
                    name: loaded.profile.name.clone(),
                    fields: loaded.profile.fields.clone(),
                    has_api_key: loaded.has_api_key,
                    is_default: Some(&loaded.profile.id) == default.as_ref(),
                    problem: loaded.model.as_ref().err().cloned(),
                })
                .collect(),
        }
    }
}

async fn saved_models(store: &Store) -> Result<SavedModels> {
    if let Some(raw) = store.get_setting(MODELS_SETTING).await? {
        return serde_json::from_str(&raw).context("Saved model profiles are unreadable");
    }
    Ok(match store.get_setting(LEGACY_SETTING).await? {
        Some(raw) => {
            let fields: ModelFields =
                serde_json::from_str(&raw).context("Saved model settings are unreadable")?;
            SavedModels {
                profiles: vec![SavedProfile {
                    id: LEGACY_PROFILE.into(),
                    name: fields.model.trim().to_owned(),
                    fields,
                }],
                default: LEGACY_PROFILE.into(),
            }
        }
        None => SavedModels::default(),
    })
}

/// The earlier single model keeps the secret name it was saved under.
fn key_name(key_id: &str, profile: &str) -> String {
    if profile == LEGACY_PROFILE {
        format!("desktop.model.api_key.{key_id}")
    } else {
        format!("desktop.model.api_key.{key_id}.{profile}")
    }
}

/// Conversations of one project for the sidebar.
#[derive(Clone)]
pub struct ConversationList {
    pub rows: Vec<SessionSearchInfo>,
    /// IDs the user pinned, to be shown first.
    pub pinned: Vec<String>,
    /// The project's groups, in the order the user gave them.
    pub folders: Vec<Folder>,
    /// Conversation id to the id of the group it is in.
    pub grouped: HashMap<String, String>,
    /// Branch id to the id of the conversation it was branched from.
    pub branched: HashMap<String, String>,
}

/// Names one user message of a conversation without trusting a row number:
/// how many user messages come after it, and what it says. A reference that
/// does not match the saved conversation is refused, never resolved to the
/// nearest guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnRef {
    /// 0 is the last user message.
    pub from_end: usize,
    /// The message as the transcript shows it.
    pub text: String,
}

/// Said when a [`TurnRef`] names no saved message. The WebView desktop
/// refuses the same case in the same words.
pub const TURN_NOT_FOUND: &str =
    "That message could not be found in the saved conversation. Nothing was changed.";
/// User messages a [`TurnRef`] may count back over.
const TURN_LIMIT: usize = 5_000;

/// A named group of conversations in one project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: String,
    pub name: String,
}

/// What a batch does to each chosen conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchAction {
    Archive,
    Restore,
    Delete,
    /// Into the group with this id, or out of every group.
    Group(Option<String>),
}

/// How a batch went. One conversation's refusal does not stop the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub done: Vec<String>,
    /// `(conversation id, reason)`.
    pub refused: Vec<(String, String)>,
}

/// Conversations one batch may name.
const BATCH_LIMIT: usize = 500;

impl Server {
    pub async fn model_settings(&self) -> Result<ModelSettings> {
        Ok(self.0.model.read().unwrap().settings())
    }

    fn editable_models(&self) -> Result<()> {
        ensure!(
            self.0.model.read().unwrap().source != ModelSource::Environment,
            "The model is set by WISP_* environment variables; unset them to edit it here"
        );
        Ok(())
    }

    async fn key_id(&self) -> Result<String> {
        Ok(match self.0.store.get_setting(MODEL_KEY_ID).await? {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                self.0.store.set_setting(MODEL_KEY_ID, &id).await?;
                id
            }
        })
    }

    /// Validates a form against the key typed into it, or the one stored for
    /// the profile it edits.
    async fn resolve_input(&self, input: &ModelInput) -> Result<(ModelConfig, Option<String>)> {
        // Report the first empty field first, in the order the form shows them.
        ensure!(!input.fields.model.trim().is_empty(), "Enter a model ID");
        let typed = input
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        let key = match (&typed, &input.id) {
            (Some(key), _) => key.clone(),
            (None, Some(id)) => {
                let key_id = self.0.store.get_setting(MODEL_KEY_ID).await?;
                key_id
                    .and_then(|key_id| self.0.secrets.get(&key_name(&key_id, id)))
                    .context("Enter an API key")?
            }
            (None, None) => bail!("Enter an API key"),
        };
        Ok((resolve(&input.fields, &key)?, typed))
    }

    async fn store_models(&self, saved: &SavedModels) -> Result<ModelSettings> {
        self.0
            .store
            .set_setting(MODELS_SETTING, &serde_json::to_string(saved)?)
            .await?;
        let state = ModelState::saved(&self.0.store, self.0.secrets.as_ref()).await;
        let settings = state.settings();
        *self.0.model.write().unwrap() = state;
        Ok(settings)
    }

    /// Adds a profile or replaces one. Validates first, so a rejected form
    /// never replaces working settings. Turns already running keep the model
    /// they started with. The first profile saved becomes the default.
    pub async fn save_model_profile(&self, input: ModelInput) -> Result<ModelSettings> {
        self.editable_models()?;
        let (_, typed) = self.resolve_input(&input).await?;
        let name = match input.name.trim() {
            "" => input.fields.model.trim(),
            name => name,
        };
        ensure!(
            name.chars().count() <= NAME_LIMIT,
            "Keep the name within {NAME_LIMIT} characters"
        );
        let mut saved = saved_models(&self.0.store).await?;
        let id = match &input.id {
            Some(id) => {
                ensure!(
                    saved.profiles.iter().any(|profile| &profile.id == id),
                    "That model was removed; add it again"
                );
                id.clone()
            }
            None => uuid::Uuid::new_v4().to_string(),
        };
        if let Some(key) = &typed {
            self.0
                .secrets
                .set(&key_name(&self.key_id().await?, &id), key)?;
        }
        let profile = SavedProfile {
            id: id.clone(),
            name: name.to_owned(),
            fields: input.fields,
        };
        match saved.profiles.iter_mut().find(|profile| profile.id == id) {
            Some(existing) => *existing = profile,
            None => saved.profiles.push(profile),
        }
        if !saved.profiles.iter().any(|p| p.id == saved.default) {
            saved.default = id;
        }
        self.store_models(&saved).await
    }

    /// Conversations that chose nothing, and new ones, use this profile.
    pub async fn set_default_model(&self, id: &str) -> Result<ModelSettings> {
        self.editable_models()?;
        let mut saved = saved_models(&self.0.store).await?;
        ensure!(
            saved.profiles.iter().any(|profile| profile.id == id),
            "That model was removed"
        );
        saved.default = id.into();
        self.store_models(&saved).await
    }

    /// Removes a profile and its key. Conversations that had chosen it go
    /// back to the default.
    pub async fn delete_model_profile(&self, id: &str) -> Result<ModelSettings> {
        self.editable_models()?;
        let mut saved = saved_models(&self.0.store).await?;
        let before = saved.profiles.len();
        saved.profiles.retain(|profile| profile.id != id);
        ensure!(saved.profiles.len() < before, "That model was removed");
        if saved.default == id {
            saved.default = saved
                .profiles
                .first()
                .map(|profile| profile.id.clone())
                .unwrap_or_default();
        }
        if let Some(key_id) = self.0.store.get_setting(MODEL_KEY_ID).await? {
            // The secret store has no delete; an empty value reads as absent.
            self.0.secrets.set(&key_name(&key_id, id), "")?;
        }
        self.store_models(&saved).await
    }

    /// Sends one short request with the form as typed, without saving it,
    /// and returns what the model said.
    pub async fn test_model(&self, input: ModelInput) -> Result<String> {
        let (model, _) = self.resolve_input(&input).await?;
        let provider = (self.0.models)(model.provider);
        let ask = [wisp_llm::Message::user("Reply with the single word: ready")];
        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            provider.complete(&ask, &[]),
        )
        .await
        .context("The model did not answer within 30 seconds")?
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        // One line is enough to show that it answered.
        let text: String = reply
            .content
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect();
        Ok(if text.is_empty() {
            "The model answered.".into()
        } else {
            text
        })
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
        let mut grouped = HashMap::new();
        let mut branched = HashMap::new();
        let rows: Vec<SessionSearchInfo> = if archived {
            self.0
                .store
                .list_sessions_page_with_visibility(project, None, 100, Some(true), query)
                .await?
                .into_iter()
                .map(|(id, title, ts, folder, source)| {
                    if let Some(folder) = folder {
                        grouped.insert(id.clone(), folder);
                    }
                    if let Some(source) = source {
                        branched.insert(id.clone(), source);
                    }
                    SessionSearchInfo {
                        id,
                        project_id: project.into(),
                        project_name: String::new(),
                        title,
                        ts,
                        activity_at: ts,
                        status: "complete".into(),
                        shelved: true,
                    }
                })
                .collect()
        } else {
            let rows = self
                .session_rows(Some(project), query, !query.is_empty())
                .await?;
            let listed: HashSet<&str> = rows.iter().map(|row| row.id.as_str()).collect();
            for (id, _, _, folder, source) in self.0.store.list_sessions(project).await? {
                if !listed.contains(id.as_str()) {
                    continue;
                }
                if let Some(folder) = folder {
                    grouped.insert(id.clone(), folder);
                }
                if let Some(source) = source {
                    branched.insert(id, source);
                }
            }
            rows
        };
        let folders = self
            .0
            .store
            .list_folders(project)
            .await?
            .into_iter()
            .map(|(id, name, _)| Folder { id, name })
            .collect();
        let pinned = self
            .0
            .store
            .list_pinned_sessions(project)
            .await?
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        Ok(ConversationList {
            rows,
            pinned,
            folders,
            grouped,
            branched,
        })
    }

    /// A choice about how the desktop looks, such as how the sidebar is
    /// arranged. `None` until one was saved.
    pub async fn preference(&self, name: &str) -> Result<Option<String>> {
        self.0.store.get_setting(&preference_key(name)?).await
    }

    pub async fn set_preference(&self, name: &str, value: &str) -> Result<()> {
        ensure!(
            value.len() <= PREFERENCE_LIMIT,
            "Preferences are limited to {PREFERENCE_LIMIT} bytes"
        );
        self.0
            .store
            .set_setting(&preference_key(name)?, value)
            .await
    }

    /// A group name that is not empty, not too long and not already taken in
    /// the project. `except` is the group being renamed.
    async fn folder_name(&self, project: &str, name: &str, except: Option<&str>) -> Result<String> {
        let name = name.trim();
        ensure!(!name.is_empty(), "Enter a group name");
        ensure!(
            name.chars().count() <= NAME_LIMIT,
            "Group names are limited to {NAME_LIMIT} characters"
        );
        let taken = self
            .0
            .store
            .list_folders(project)
            .await?
            .into_iter()
            .any(|(id, other, _)| {
                Some(id.as_str()) != except && other.to_lowercase() == name.to_lowercase()
            });
        ensure!(!taken, "This project already has a group with that name");
        Ok(name.to_owned())
    }

    /// Adds an empty group after the project's existing ones.
    pub async fn create_folder(&self, project: &str, name: &str) -> Result<Folder> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        let name = self.folder_name(project, name, None).await?;
        let id = uuid::Uuid::new_v4().to_string();
        self.0.store.create_folder(&id, project, &name).await?;
        Ok(Folder { id, name })
    }

    pub async fn rename_folder(&self, project: &str, folder: &str, name: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        let name = self.folder_name(project, name, Some(folder)).await?;
        self.0.store.rename_folder(folder, project, &name).await
    }

    /// Removes the group. Its conversations stay, outside any group.
    pub async fn delete_folder(&self, project: &str, folder: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        self.0.store.delete_folder(folder, project).await
    }

    /// Saves the order of the project's groups, first to last. Groups left
    /// out keep their place.
    pub async fn reorder_folders(&self, project: &str, folders: &[String]) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        self.0.store.reorder_folders(project, folders).await
    }

    /// Puts the conversation, with its branches, into a group, or with
    /// `None` takes it out of the one it is in.
    pub async fn group_conversation(
        &self,
        project: &str,
        session: &str,
        folder: Option<&str>,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.0
            .store
            .move_session_to_folder(session, project, folder)
            .await
    }

    /// Applies one action to several conversations of a project. Each is
    /// held to the same rules as when it is done alone.
    pub async fn batch(
        &self,
        project: &str,
        sessions: &[String],
        action: BatchAction,
    ) -> Result<BatchReport> {
        self.known_project(project).await?;
        ensure!(
            sessions.len() <= BATCH_LIMIT,
            "Choose at most {BATCH_LIMIT} conversations at a time"
        );
        if let BatchAction::Group(Some(folder)) = &action {
            let known = self.0.store.list_folders(project).await?;
            ensure!(known.iter().any(|(id, ..)| id == folder), "Group not found");
        }
        let mut report = BatchReport::default();
        let mut seen = HashSet::new();
        for session in sessions.iter().filter(|id| seen.insert(id.as_str())) {
            let result = match &action {
                BatchAction::Archive => self.archive_conversation(project, session, true).await,
                BatchAction::Restore => self.archive_conversation(project, session, false).await,
                BatchAction::Delete => self.delete_conversation(project, session).await,
                BatchAction::Group(folder) => {
                    self.group_conversation(project, session, folder.as_deref())
                        .await
                }
            };
            match result {
                Ok(()) => report.done.push(session.clone()),
                Err(error) => report.refused.push((session.clone(), error.to_string())),
            }
        }
        Ok(report)
    }

    /// Copies the conversation's transcript into another project, or with
    /// `keep` off moves it there. Files, runs and artifacts stay where they
    /// are. Returns the id the conversation has in the target project.
    ///
    /// The conversation keeps its model and plan-first choices; it goes back
    /// to asking before it changes anything, because the files are not the
    /// ones full access was given for.
    pub async fn transfer_conversation(
        &self,
        project: &str,
        session: &str,
        target: &str,
        keep: bool,
    ) -> Result<String> {
        ensure!(project != target, "Choose a different project");
        let live = self.session(session);
        // Acceptance takes the same lock, so no turn can start mid-transfer.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.known_project(target).await?;
        self.ensure_idle(session, if keep { "copy" } else { "move" })?;
        let options = self.stored_options(session).await?;
        let id = uuid::Uuid::new_v4().to_string();
        if keep {
            self.0
                .store
                .copy_session_to_project(session, project, target, &id)
                .await?;
        } else {
            self.without_branches(project, session, "move").await?;
            self.0
                .store
                .move_session_to_project(session, project, target, &id)
                .await?;
        }
        // The user sent it there themselves; it does not arrive unread.
        self.0.store.mark_frame_seen(&id).await?;
        if !options.model.is_empty() {
            self.0
                .store
                .set_setting(&model_choice_key(&id), &options.model)
                .await?;
        }
        if options.plan_first {
            self.0
                .store
                .set_setting(&plan_mode_key(&id), "true")
                .await?;
        }
        if !keep {
            self.forget(session, &live).await?;
        }
        Ok(id)
    }

    /// Finds the user message `turn` names: its sequence number and that of
    /// the next user message, if there is one.
    async fn locate_turn(&self, session: &str, turn: &TurnRef) -> Result<(i64, Option<i64>)> {
        ensure!(turn.from_end < TURN_LIMIT, TURN_NOT_FOUND);
        // The last `from_end + 1` turns, starting at the one that is meant.
        let page = self
            .0
            .store
            .load_session_transcript_page(session, None, turn.from_end + 1)
            .await?;
        let users: Vec<_> = page
            .messages
            .iter()
            .filter(|(_, message)| message.role == wisp_llm::Role::User)
            .collect();
        ensure!(users.len() == turn.from_end + 1, TURN_NOT_FOUND);
        let (seq, message) = users[0];
        ensure!(
            crate::runtime::bounded(&message.content.as_text()) == turn.text,
            TURN_NOT_FOUND
        );
        Ok((*seq, users.get(1).map(|(seq, _)| *seq)))
    }

    /// Removes the user message `turn` names and everything after it, so it
    /// can be written again. Returns its text without the list of attached
    /// files. Files the removed turns wrote, their runs and their artifacts
    /// stay as they are.
    pub async fn rewind_conversation(
        &self,
        project: &str,
        session: &str,
        turn: &TurnRef,
    ) -> Result<String> {
        let live = self.session(session);
        // Acceptance takes the same lock, so no turn can start mid-rewind.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.ensure_idle(session, "rewind")?;
        let store = &self.0.store;
        store.require_unarchived_session(session).await?;
        ensure!(
            !matches!(
                store.session_branch_state(session).await?,
                Some("merged" | "orphaned")
            ),
            "A branch that was merged or lost its source cannot be rewound"
        );
        let (seq, _) = self.locate_turn(session, turn).await?;
        let keep = seq - 1;
        // The row before the message decides which context the conversation
        // returns to. A missing row is refused: assuming the first context
        // would delete every later one.
        let epoch = if keep <= 0 {
            0
        } else {
            store
                .resolve_message_epoch(session, keep)
                .await?
                .context(TURN_NOT_FOUND)?
        };
        store.rewind_to_seq(session, epoch, keep).await?;
        {
            // What the last turn left behind describes rows that are gone.
            let mut live = live.live.lock().unwrap();
            live.generation += 1;
            live.messages.clear();
            live.outcomes.clear();
            live.usage = None;
            live.error = None;
        }
        Ok(message_body(&turn.text).to_owned())
    }

    /// Starts a new conversation in the same project from a copy of this
    /// one: all of it, or with `upto` everything through that message's
    /// reply. The branch is listed under its source, in the same group, with
    /// the same model, effort, Plan first and Full access choices. Returns
    /// its id.
    pub async fn branch_conversation(
        &self,
        project: &str,
        session: &str,
        upto: Option<&TurnRef>,
    ) -> Result<String> {
        let live = self.session(session);
        // The copy must not race a turn that is still writing its rows.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.ensure_idle(session, "branch")?;
        let store = &self.0.store;
        let cut = match upto {
            Some(turn) => self.locate_turn(session, turn).await?.1,
            None => None,
        };
        let messages: Vec<wisp_llm::Message> = match cut {
            // Through the last turn: the conversation as the model has it.
            None => store.load_messages(session).await?,
            Some(next) => {
                let keep = next - 1;
                let epoch = store
                    .resolve_message_epoch(session, keep)
                    .await?
                    .context(TURN_NOT_FOUND)?;
                store
                    .load_messages_in_epoch(session, epoch)
                    .await?
                    .into_iter()
                    .filter(|(seq, _)| *seq <= keep)
                    .map(|(_, message)| message)
                    .collect()
            }
        };
        ensure!(
            messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::User),
            "There is nothing to branch from yet"
        );
        let options = self.stored_options(session).await?;
        let title = store.session_title(session).await?.unwrap_or_default();
        let folder = store
            .list_sessions(project)
            .await?
            .into_iter()
            .find(|(id, ..)| id == session)
            .and_then(|(_, _, _, folder, _)| folder);
        let id = uuid::Uuid::new_v4().to_string();
        store
            .create_frame(&id, project, "main", &self.model_id(&options.model))
            .await?;
        let filled = async {
            for (index, message) in messages.iter().enumerate() {
                store.append_message(&id, index as i64 + 1, message).await?;
            }
            store
                .rename_session(&id, project, &branch_title(&title))
                .await?;
            store.set_session_branched_from(&id, session).await?;
            if let Some(folder) = &folder {
                store
                    .move_session_to_folder(&id, project, Some(folder))
                    .await?;
            }
            if !options.reasoning_effort.is_empty() {
                store
                    .set_frame_reasoning_effort(&id, project, Some(&options.reasoning_effort))
                    .await?;
            }
            for (key, value) in [
                (model_choice_key(&id), options.model.as_str()),
                (
                    plan_mode_key(&id),
                    if options.plan_first { "true" } else { "" },
                ),
                (
                    full_access_key(&id),
                    if options.full_access { "1" } else { "" },
                ),
            ] {
                if !value.is_empty() {
                    store.set_setting(&key, value).await?;
                }
            }
            // Its messages were read where they were written.
            store.mark_frame_seen(&id).await
        }
        .await;
        if let Err(error) = filled {
            // Half a branch is worse than none.
            let _ = store.delete_session(&id, project).await;
            return Err(error);
        }
        Ok(id)
    }

    /// Drops everything the host holds for a conversation that has left the
    /// project: its saved choices, its live state and its interpreters.
    async fn forget(&self, session: &str, live: &crate::output::Session) -> Result<()> {
        for key in [
            full_access_key(session),
            model_choice_key(session),
            plan_mode_key(session),
        ] {
            self.0.store.delete_setting(&key).await?;
        }
        self.0.sessions.lock().unwrap().remove(session);
        let runtime = live.runtime.lock().unwrap().take();
        if let Some(runtime) = runtime {
            runtime.shutdown_all().await;
        }
        Ok(())
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

    /// A conversation with branches under it stays until they are gone.
    async fn without_branches(&self, project: &str, session: &str, action: &str) -> Result<()> {
        ensure!(
            !self
                .0
                .store
                .session_has_conversation_branches(session, project)
                .await?,
            "Delete this conversation's branches before you {action} it"
        );
        Ok(())
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

    async fn project_root(&self, project: &str) -> Result<String> {
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        Ok(self
            .0
            .store
            .get_project(project)
            .await?
            .context("Project not found")?
            .1)
    }

    /// Sends a message that may carry project files and references. The
    /// remote send command stays text-only.
    pub async fn send(
        &self,
        project: &str,
        request: wisp_dto::native_conversations::SendRequest,
    ) -> Result<wisp_dto::native_conversations::SendAccepted> {
        let root = self.project_root(project).await?;
        self.own_conversation(project, &request.session_id).await?;
        Ok(serde_json::from_value(
            self.start(project, &root, request).await?,
        )?)
    }

    /// Makes files chosen in the file picker attachable: copies those from
    /// outside the project into its `uploads` folder and returns every
    /// file's project-relative path.
    pub async fn import_attachments(
        &self,
        project: &str,
        sources: &[std::path::PathBuf],
    ) -> Result<Vec<String>> {
        let root = self.project_root(project).await?;
        crate::files::import(std::path::Path::new(&root), sources).await
    }

    /// Skills a conversation in this project can be pointed at, filtered by
    /// the words of `query`.
    pub async fn skill_options(
        &self,
        project: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ReferenceOption>> {
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let config = self.0.config.clone();
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        Ok(tokio::task::spawn_blocking(move || {
            crate::runtime::skill_index(&config, &root)
                .all()
                .iter()
                .filter(|skill| {
                    let text = format!("{} {}", skill.name, skill.description).to_lowercase();
                    words.iter().all(|word| text.contains(word))
                })
                .take(limit)
                .map(|skill| ReferenceOption {
                    reference: wisp_dto::ComposerReferenceArg::Skill {
                        name: skill.name.clone(),
                    },
                    label: skill.name.clone(),
                    detail: skill.description.clone(),
                })
                .collect()
        })
        .await?)
    }

    /// Files the project's conversations produced, newest first; those of
    /// one conversation when `session` is given. `query` filters by name.
    pub async fn artifacts(
        &self,
        project: &str,
        session: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ArtifactRow>> {
        self.project_root(project).await?;
        let found = self
            .0
            .store
            .search_artifacts(Some(project), query.trim(), 500, None)
            .await?;
        Ok(found
            .into_iter()
            .filter(|artifact| session.is_none_or(|session| artifact.session_id == session))
            .take(limit)
            .map(|artifact| ArtifactRow {
                id: artifact.id,
                name: artifact.name,
                kind: artifact.kind,
                path: artifact.logical_path,
                created_at: artifact.ts,
                size_bytes: artifact
                    .size_bytes
                    .and_then(|size| u64::try_from(size).ok()),
                session_id: artifact.session_id,
                session_title: artifact.session_title,
            })
            .collect())
    }

    /// Asks a run that has not finished to stop.
    pub async fn cancel_run(&self, project: &str, run: &str) -> Result<()> {
        self.project_root(project).await?;
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        self.0
            .runs
            .cancel(&self.0.store, run)
            .await
            .map_err(anyhow::Error::msg)
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
            model: self
                .0
                .store
                .get_setting(&model_choice_key(session))
                .await?
                .unwrap_or_default(),
            plan_first: self
                .0
                .store
                .get_setting(&plan_mode_key(session))
                .await?
                .as_deref()
                == Some("true"),
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
            reasoning: live.reasoning.clone(),
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
        let key = plan_mode_key(session);
        if options.plan_first {
            self.0.store.set_setting(&key, "true").await?;
        } else {
            self.0.store.delete_setting(&key).await?;
        }
        let key = model_choice_key(session);
        if options.model.is_empty() {
            self.0.store.delete_setting(&key).await?;
        } else {
            ensure!(
                self.0
                    .model
                    .read()
                    .unwrap()
                    .settings()
                    .profiles
                    .iter()
                    .any(|profile| profile.id == options.model),
                "That model was removed; choose another"
            );
            self.0.store.set_setting(&key, &options.model).await?;
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
        self.without_branches(project, session, "delete").await?;
        self.0.store.delete_session(session, project).await?;
        self.forget(session, &live).await
    }
}

/// A file a conversation produced, as the Artifacts list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactRow {
    pub id: String,
    pub name: String,
    /// MIME type.
    pub kind: String,
    /// Where the file is in the project, when it is a project file.
    pub path: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
    pub size_bytes: Option<u64>,
    pub session_id: String,
    pub session_title: String,
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
    /// Id of the model profile this conversation runs on. Empty follows the
    /// default profile, whichever that is at the time.
    pub model: String,
    /// Investigate and propose a plan instead of carrying the task out.
    /// Tools that change things refuse to run until this is turned off.
    pub plan_first: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversationStatus {
    pub options: ConversationOptions,
    /// The running or latest turn since the app started; `None` before one.
    pub usage: Option<TokenUsage>,
    /// Tools allowed for the rest of this conversation, sorted.
    pub allowed_tools: Vec<String>,
    /// What the model is thinking right now; empty when it is not.
    pub reasoning: String,
}

/// A user message without the list of files that was sent with it.
fn message_body(text: &str) -> &str {
    const FILES: &str = "Uploaded files: ";
    match text.rsplit_once("\n\n") {
        Some((body, last)) if last.starts_with(FILES) => body,
        None if text.starts_with(FILES) => "",
        _ => text,
    }
}

/// What a branch of the conversation titled `source` is called.
fn branch_title(source: &str) -> String {
    const SUFFIX: &str = " (branch)";
    let source = source.trim();
    let source = if source.is_empty() {
        "Conversation"
    } else {
        source
    };
    let room = TITLE_LIMIT - SUFFIX.chars().count();
    let mut title: String = source.chars().take(room).collect();
    title.push_str(SUFFIX);
    title
}

const PREFERENCE_LIMIT: usize = 2_000;

/// Preferences have their own corner of the settings table, so a name can
/// never reach a model, a key id or another conversation's choices.
fn preference_key(name: &str) -> Result<String> {
    ensure!(
        !name.is_empty()
            && name.len() <= 40
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "Preference names are lowercase letters, digits and underscores"
    );
    Ok(format!("desktop.ui.{name}"))
}

/// The key the WebView desktop uses for the same switch.
fn plan_mode_key(session: &str) -> String {
    format!("frame_plan_mode:{session}")
}

fn model_choice_key(session: &str) -> String {
    format!("desktop.conversation.model.{session}")
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
    fn a_rewound_message_comes_back_without_its_file_list() {
        assert_eq!(message_body("plain"), "plain");
        assert_eq!(message_body("two\n\nparagraphs"), "two\n\nparagraphs");
        assert_eq!(
            message_body("look at this\n\nUploaded files: a.csv, b.png"),
            "look at this"
        );
        assert_eq!(message_body("Uploaded files: a.csv"), "");
        assert_eq!(
            message_body("one\n\ntwo\n\nUploaded files: a.csv"),
            "one\n\ntwo"
        );
    }

    #[test]
    fn a_branch_is_named_after_its_source_within_the_title_limit() {
        assert_eq!(branch_title(" Kinase screen "), "Kinase screen (branch)");
        assert_eq!(branch_title(""), "Conversation (branch)");
        let long = branch_title(&"基".repeat(300));
        assert_eq!(long.chars().count(), TITLE_LIMIT);
        assert!(long.ends_with(" (branch)"));
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
