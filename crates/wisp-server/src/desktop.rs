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
pub use wisp_dto::{ProjectMemo, ProjectMemoPatch};
use wisp_llm::ProviderKind;
pub use wisp_store::GlobalMemory;
use wisp_store::Store;
pub use wisp_store::{LibraryItem, LibraryItemSummary};

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
/// Set once the first-run guide is finished or put away; the WebView's key.
const ONBOARDING_DONE: &str = "onboarding_done";
/// What the first-run guide sets up, as the WebView does.
const DEEPSEEK_URL: &str = "https://api.deepseek.com";
const DEEPSEEK_FLASH: &str = "deepseek-v4-flash";
const DEEPSEEK_PRO: &str = "deepseek-v4-pro";
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

    pub(crate) async fn key_id(&self) -> Result<String> {
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

    /// Whether the first-run guide was finished or put away, in this
    /// desktop or the WebView one (the same setting).
    pub async fn onboarding_done(&self) -> Result<bool> {
        Ok(self.0.store.get_setting(ONBOARDING_DONE).await?.is_some())
    }

    pub async fn finish_onboarding(&self) -> Result<()> {
        self.0.store.set_setting(ONBOARDING_DONE, "1").await
    }

    /// The first-run guide's model step, as the WebView sets it up: the
    /// DeepSeek key saved once for both of its models, Flash as the
    /// default and Pro for heavier work.
    pub async fn set_up_deepseek(&self, key: &str) -> Result<ModelSettings> {
        self.editable_models()?;
        let key = key.trim();
        ensure!(!key.is_empty(), "Paste the key first");
        let input = |model: &str| ModelInput {
            id: None,
            name: String::new(),
            fields: ModelFields {
                provider: "openai".into(),
                api_url: DEEPSEEK_URL.into(),
                model: model.into(),
                max_context: String::new(),
                max_tokens: "8192".into(),
                reasoning_effort: String::new(),
            },
            api_key: Some(key.to_owned()),
        };
        self.save_model_profile(input(DEEPSEEK_PRO)).await?;
        let saved = self.save_model_profile(input(DEEPSEEK_FLASH)).await?;
        let flash = saved
            .profiles
            .iter()
            .rev()
            .find(|profile| profile.fields.model == DEEPSEEK_FLASH)
            .map(|profile| profile.id.clone())
            .context("The model was not saved")?;
        self.set_default_model(&flash).await
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
        let (mut model, _) = self.resolve_input(&input).await?;
        // A test goes the way a turn would.
        let routes = crate::network::load(&self.0.store).await;
        if let Some(route) = crate::network::route(&routes.model_proxy_url) {
            model.provider.proxy = Some(route);
        }
        let provider = (self.0.models)(model.provider);
        let ask = [wisp_llm::Message::user("Reply with the single word: ready")];
        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            provider.complete(&ask, &[]),
        )
        .await
        .context("The model did not answer within 30 seconds")?;
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => bail!(
                "{}",
                crate::network::explain(&self.0.store, &error.to_string()).await
            ),
        };
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

    /// A picture in the project, whole, for the file browser to show: its
    /// content type and its bytes. Anything that is not a picture by its
    /// name, or is larger than a figure may be, is refused.
    pub async fn project_image(&self, project: &str, path: &str) -> Result<(String, Vec<u8>)> {
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let real = crate::files::resolve(&root, path).await?;
        let content_type = image_type(&real).context("That file is not a picture")?;
        let meta = tokio::fs::metadata(&real).await?;
        ensure!(meta.is_file(), "That is not a file");
        ensure!(
            meta.len() <= FIGURE_LIMIT,
            "Pictures of more than 32 MB are not shown here"
        );
        Ok((content_type.to_owned(), tokio::fs::read(&real).await?))
    }

    /// A document of the project (PDF, Word, Excel, PowerPoint, OpenDocument,
    /// RTF, EPUB) as Markdown to read: its text and tables, not its layout
    /// or pictures. The converter is the one the model's `read` tool uses.
    /// `None` for a file that is not such a document.
    pub async fn document_preview(&self, project: &str, path: &str) -> Result<Option<String>> {
        /// What the preview shows of a long document.
        const SHOWN: usize = 256 * 1024;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let real = crate::files::resolve(&root, path).await?;
        let meta = tokio::fs::metadata(&real).await?;
        ensure!(meta.is_file(), "That is not a file");
        ensure!(
            meta.len() <= FIGURE_LIMIT,
            "Documents of more than 32 MB are not shown here"
        );
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&real)?;
            match wisp_tools::read::document_markdown(&real, &bytes) {
                None => Ok(None),
                Some(Err(error)) => bail!("This document could not be read: {error}"),
                Some(Ok(mut text)) => {
                    if text.len() > SHOWN {
                        let mut cut = SHOWN;
                        while !text.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        text.truncate(cut);
                        text.push_str("\n\n…");
                    }
                    Ok(Some(text))
                }
            }
        })
        .await?
    }

    /// Makes an empty folder (`folder`) or file named `name` in the project
    /// folder `parent`; the reply is its project-relative path.
    pub async fn create_project_entry(
        &self,
        project: &str,
        parent: &str,
        name: &str,
        folder: bool,
    ) -> Result<String> {
        let _admission = self.0.admission.read().await;
        let root = self.project_root(project).await?;
        crate::files::create(std::path::Path::new(&root), parent, name, folder).await
    }

    /// Renames a project file or folder where it is; the reply is its new
    /// project-relative path.
    pub async fn rename_project_entry(
        &self,
        project: &str,
        path: &str,
        name: &str,
    ) -> Result<String> {
        let _admission = self.0.admission.read().await;
        let root = self.project_root(project).await?;
        crate::files::rename(std::path::Path::new(&root), path, name).await
    }

    /// Moves a project file or folder into the project's `.wisp/trash`;
    /// the reply is where it went. Nothing is erased.
    pub async fn trash_project_entry(&self, project: &str, path: &str) -> Result<String> {
        let _admission = self.0.admission.read().await;
        let root = self.project_root(project).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        crate::files::trash(std::path::Path::new(&root), path, now).await
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

    /// Writes `project`, its records and its workspace files, as one zip
    /// at `destination`, in the format the WebView desktop reads. It is
    /// refused while a turn or a run of the project is going, and no turn
    /// starts while it is written.
    pub async fn export_project(&self, project: &str, destination: &std::path::Path) -> Result<()> {
        // Exclusive: nothing is admitted while the records are copied.
        let _admission = self.0.admission.write().await;
        self.known_project(project).await?;
        ensure!(
            destination.is_absolute(),
            "Choose where to save the archive"
        );
        let running: HashSet<String> = self
            .0
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, session)| session.live.lock().unwrap().running)
            .map(|(id, _)| id.clone())
            .collect();
        crate::project_archive::ensure_project_export_idle(&self.0.store, project, &running)
            .await
            .map_err(anyhow::Error::msg)?;
        crate::project_archive::export_project_contents(
            &self.0.store,
            &self.0.config.data,
            project,
            destination,
            false,
            None,
        )
        .await
        .map_err(anyhow::Error::msg)
    }

    /// Reads a project archive, puts the project in a new folder under
    /// `parent`, and registers it. The reply is the project's id, which is
    /// the one it had where it was exported; a project that is already
    /// here is refused, not merged.
    pub async fn import_project(
        &self,
        archive: &std::path::Path,
        parent: &std::path::Path,
    ) -> Result<String> {
        let _admission = self.0.admission.write().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        let parent = tokio::fs::canonicalize(parent)
            .await
            .context("Choose a folder to put the project in")?;
        ensure!(parent.is_dir(), "Choose a folder to put the project in");
        ensure!(
            !parent.starts_with(&self.0.config.data),
            "Keep project files separate from the host data directory"
        );
        crate::project_archive::import_archived_project(
            &self.0.store,
            &self.0.config.data,
            archive,
            &parent,
        )
        .await
        .map_err(anyhow::Error::msg)
    }

    /// Makes a sample project in a new folder under `parent`: made-up
    /// data, a script and a note on what to ask. Returns the project. It
    /// is an ordinary project afterwards, the user's to change or remove.
    pub async fn create_sample_project(&self, parent: &std::path::Path) -> Result<String> {
        let parent = tokio::fs::canonicalize(parent)
            .await
            .context("Choose a folder to put the sample project in")?;
        ensure!(
            parent.is_dir(),
            "Choose a folder to put the sample project in"
        );
        ensure!(
            !parent.starts_with(&self.0.config.data),
            "Keep project files separate from the host data directory"
        );
        let folder = tokio::task::spawn_blocking(move || crate::sample::write(&parent)).await??;
        self.register_local_project(&folder).await
    }

    /// What Wisp can draw on in `project`, counted: skills, connectors, MCP
    /// servers, SSH hosts, memory, and the command-line tools found on
    /// this computer.
    pub async fn capabilities(&self, project: &str) -> Result<Capabilities> {
        let on = |flags: Vec<bool>| (flags.iter().filter(|on| **on).count(), flags.len());
        let skills = self.skills(Some(project)).await?;
        let connectors = self.connectors().await?;
        let servers = self.mcp_servers().await?;
        let hosts = self.environments().await?;
        let memory = self.memory(project).await?;
        let tools = tokio::task::spawn_blocking(|| {
            let (path, extensions) = crate::tools_found::search_path();
            crate::tools_found::TOOLS
                .iter()
                .map(|(name, what)| LocalTool {
                    name: (*name).to_owned(),
                    what: (*what).to_owned(),
                    path: crate::tools_found::find(name, &path, &extensions, |candidate| {
                        candidate.is_file()
                    })
                    .map(|found| found.to_string_lossy().into_owned()),
                })
                .collect()
        })
        .await?;
        Ok(Capabilities {
            skills: on(skills.iter().map(|skill| skill.enabled).collect()),
            connectors: on(connectors.iter().map(|row| row.enabled).collect()),
            mcp_servers: on(servers.iter().map(|row| row.server.enabled).collect()),
            // A host counts as usable once a check of it has passed.
            hosts: (
                hosts
                    .checks
                    .iter()
                    .filter(|check| check.error.is_none())
                    .count(),
                hosts.hosts.len(),
            ),
            memory_on: memory.enabled,
            memory_notes: memory.notes.len(),
            habits: memory.habits.len(),
            tools,
        })
    }

    /// The whole of a conversation as a Markdown document: what was said,
    /// and each tool call with what it was given and what it returned. It
    /// is read from what is saved, so a turn that is still running is in
    /// it only as far as it has been saved.
    pub async fn export_conversation(&self, project: &str, session: &str) -> Result<String> {
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        // The document reads oldest first.
        let messages = self.whole_transcript(session).await?;
        let mut rows = crate::runtime::items(&messages);
        let outcomes: HashMap<_, _> = store
            .load_session_ui_events(session)
            .await?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<crate::output::OutcomeEvent>(&raw).ok())
            .filter(|event| event.kind == "ServerToolResult")
            .map(|event| (event.outcome.call_id.clone(), event.outcome.ok))
            .collect();
        for row in &mut rows {
            if let Some(ok) = row.call_id.as_ref().and_then(|id| outcomes.get(id)) {
                row.ok = Some(*ok);
            }
        }
        let title = store.session_title(session).await?.unwrap_or_default();
        let name = store
            .get_project(project)
            .await?
            .map(|(name, _)| name)
            .unwrap_or_default();
        Ok(crate::export::markdown(&title, &name, &rows))
    }

    /// A conversation as a trajectory: every turn's steps in the order
    /// they happened, with what each tool and model call took.
    pub async fn trajectory(
        &self,
        project: &str,
        session: &str,
    ) -> Result<wisp_dto::TrajectorySnapshotDto> {
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        let messages = self.whole_transcript(session).await?;
        let events = store.load_session_ui_events_timed(session).await?;
        Ok(crate::trajectory::fold(session, &messages, &events))
    }

    /// Every saved message of a conversation, oldest first.
    pub(crate) async fn whole_transcript(&self, session: &str) -> Result<Vec<wisp_llm::Message>> {
        Ok(self
            .numbered_transcript(session)
            .await?
            .into_iter()
            .map(|(_, message)| message)
            .collect())
    }

    /// Every saved message of a conversation with its sequence number,
    /// oldest first.
    async fn numbered_transcript(&self, session: &str) -> Result<Vec<(i64, wisp_llm::Message)>> {
        // Pages come newest first.
        let mut pages = Vec::new();
        let mut before = None;
        loop {
            let page = self
                .0
                .store
                .load_session_transcript_page(session, before, 200)
                .await?;
            before = page.next_before_seq;
            pages.push(page.messages);
            if before.is_none() {
                break;
            }
        }
        Ok(pages.into_iter().rev().flatten().collect())
    }

    /// What happened in a project from `from` up to `until` (seconds since
    /// 1970, at most 32 days apart): runs, files, what was asked in its
    /// conversations, decisions, and the researcher's own records, newest
    /// first.
    pub async fn journey(
        &self,
        project: &str,
        from: i64,
        until: i64,
    ) -> Result<wisp_dto::ResearchJourney> {
        self.known_project(project).await?;
        self.0
            .store
            .research_journey(&wisp_store::StateScope::mainline(project), from, until)
            .await
    }

    /// Every project's history from `from` up to `until` (at most 32 days
    /// apart), for the calendar across projects. A project that cannot be
    /// read is listed with why, so the calendar never looks empty when it
    /// is only incomplete.
    pub async fn calendar(
        &self,
        from: i64,
        until: i64,
    ) -> Result<Vec<wisp_dto::ResearchCalendarProject>> {
        let projects: Vec<String> = self
            .0
            .store
            .list_projects()
            .await?
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        self.0.store.research_calendar(&projects, from, until).await
    }

    /// Adds a record the researcher writes themselves to the project's
    /// history: progress, a finding, a decision or what comes next.
    pub async fn add_journal_entry(
        &self,
        project: &str,
        input: wisp_dto::ResearchJournalInput,
    ) -> Result<String> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        self.0
            .store
            .add_research_journal_entry(&wisp_store::StateScope::mainline(project), &input)
            .await
    }

    /// A project's name, description, and the rules the model is given in
    /// it. The rules are the project's own `.wisp/WISP.md`.
    pub async fn project_settings(&self, project: &str) -> Result<ProjectSettings> {
        let (name, description, root) = self
            .0
            .store
            .get_project_meta(project)
            .await?
            .context("Project not found")?;
        let rules = tokio::fs::read_to_string(rules_file(&root))
            .await
            .unwrap_or_default();
        Ok(ProjectSettings {
            name,
            description,
            rules,
        })
    }

    /// Saves them, or changes nothing and says what is wrong. Empty rules
    /// remove the file. The model reads the rules from the next turn on, in
    /// conversations that are already open too.
    pub async fn set_project_settings(
        &self,
        project: &str,
        settings: ProjectSettings,
    ) -> Result<ProjectSettings> {
        let _admission = self.0.admission.read().await;
        let root = self.project_root(project).await?;
        let name = settings.name.trim();
        let description = settings.description.trim();
        let rules = settings.rules.trim();
        ensure!(!name.is_empty(), "Give the project a name");
        ensure!(
            name.chars().count() <= PROJECT_NAME_LIMIT && !name.chars().any(char::is_control),
            "A project name is one line of at most {PROJECT_NAME_LIMIT} characters"
        );
        ensure!(
            description.chars().count() <= PROJECT_DESCRIPTION_LIMIT,
            "Keep the description under {PROJECT_DESCRIPTION_LIMIT} characters"
        );
        // The rules are sent with every turn; a book of them leaves no room
        // for the work.
        ensure!(
            rules.chars().count() <= PROJECT_RULES_LIMIT,
            "Keep the rules under {PROJECT_RULES_LIMIT} characters"
        );
        let file = rules_file(&root);
        if rules.is_empty() {
            match tokio::fs::remove_file(&file).await {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error).context("Could not remove the project's rules")
                }
                _ => {}
            }
        } else {
            if let Some(folder) = file.parent() {
                tokio::fs::create_dir_all(folder).await?;
            }
            tokio::fs::write(&file, rules)
                .await
                .context("Could not save the project's rules")?;
        }
        self.0
            .store
            .update_project(project, name, description)
            .await?;
        self.project_settings(project).await
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
        self.leave(project, None).await
    }

    /// What deleting a project together with its folder would delete, for
    /// the question that is asked first. Refused, with the reason, when the
    /// folder is not one that may be deleted with a project.
    pub async fn project_folder(&self, project: &str) -> Result<ProjectFolder> {
        let (name, folder) = self.deletable_folder(project).await?;
        let path = match &folder {
            crate::project_folder::Target::Folder(path)
            | crate::project_folder::Target::Link(path) => path.display().to_string(),
            crate::project_folder::Target::Missing => String::new(),
        };
        let contents = match folder {
            crate::project_folder::Target::Missing => FolderContents::Missing,
            crate::project_folder::Target::Link(_) => FolderContents::Link,
            crate::project_folder::Target::Folder(path) => {
                let size = tokio::task::spawn_blocking(move || {
                    crate::project_folder::measure(&path, FOLDER_COUNT_LIMIT)
                })
                .await?;
                FolderContents::Files {
                    files: size.files,
                    bytes: size.bytes,
                    more: size.more,
                }
            }
        };
        Ok(ProjectFolder {
            name,
            path,
            contents,
        })
    }

    /// Removes the project from Wisp and deletes its folder with every
    /// file in it. This cannot be undone, so `typed` has to be the
    /// project's name, and a folder that holds more than the project is
    /// refused. Nothing is removed while a turn or a run of the project is
    /// going. If the folder cannot be deleted the project stays, so it can
    /// be tried again.
    pub async fn delete_project_and_folder(&self, project: &str, typed: &str) -> Result<()> {
        let _admission = self.0.admission.write().await;
        let (name, folder) = self.deletable_folder(project).await?;
        ensure!(
            !name.trim().is_empty() && typed.trim() == name.trim(),
            "Type the project's name exactly to delete its folder"
        );
        ensure!(
            !self
                .0
                .store
                .list_active_runs()
                .await?
                .iter()
                .any(|run| run.project_id == project),
            "Wait for this project's runs to end, or cancel them, before you delete its folder"
        );
        self.leave(project, Some(folder)).await
    }

    /// The project's name and what deleting its folder would delete.
    async fn deletable_folder(
        &self,
        project: &str,
    ) -> Result<(String, crate::project_folder::Target)> {
        self.known_project(project).await?;
        ensure!(
            removable(project),
            "The built-in workspace cannot be removed"
        );
        let mut own = None;
        let mut others = Vec::new();
        for (id, name, root, ..) in self.0.store.list_projects().await? {
            if id == project {
                own = Some((name, std::path::PathBuf::from(root)));
            } else {
                others.push((name, std::path::PathBuf::from(root)));
            }
        }
        let (name, root) = own.context("Project not found")?;
        let protected = crate::project_folder::Protected {
            home: std::env::home_dir(),
            data: self.0.config.data.clone(),
            others,
        };
        let folder =
            tokio::task::spawn_blocking(move || crate::project_folder::target(&root, &protected))
                .await??;
        Ok((name, folder))
    }

    /// Takes a project out of Wisp, deleting `folder` first when one is
    /// given. The caller holds the admission lock.
    async fn leave(
        &self,
        project: &str,
        folder: Option<crate::project_folder::Target>,
    ) -> Result<()> {
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
        // Interpreters started in the folder are stopped before it goes.
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
        // The folder first: if it cannot be deleted the project is still
        // listed and nothing is half gone from Wisp.
        if let Some(folder) = folder {
            crate::project_folder::delete(folder).await?;
        }
        self.0.store.delete_project(project).await?;
        Ok(())
    }

    pub(crate) async fn own_conversation(&self, project: &str, session: &str) -> Result<()> {
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

    /// The project's memos, pinned first, then the most recently changed.
    /// A memo is the researcher's own note. Nothing in the host hands one to
    /// the model: it reaches a conversation only as part of a message the
    /// researcher sends.
    pub async fn memos(&self, project: &str) -> Result<Vec<ProjectMemo>> {
        self.known_project(project).await?;
        self.0.store.list_project_memos(project).await
    }

    pub async fn add_memo(&self, project: &str, title: &str, body: &str) -> Result<ProjectMemo> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        let input = wisp_dto::ProjectMemoInput {
            title: title.into(),
            body: body.into(),
        };
        self.0.store.add_project_memo(project, &input).await
    }

    /// Changes the parts of a memo the patch names: title, body or pin.
    pub async fn update_memo(&self, project: &str, patch: ProjectMemoPatch) -> Result<ProjectMemo> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        self.0.store.update_project_memo(project, &patch).await
    }

    pub async fn delete_memo(&self, project: &str, memo: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        self.0.store.delete_project_memo(project, memo).await
    }

    /// What is remembered: whether memory is on, the project's notes and the
    /// habits that apply in every project.
    pub async fn memory(&self, project: &str) -> Result<MemoryView> {
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let dir = wisp_core::MemoryManager::at(&root).dir().to_owned();
        let mut notes = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if memory_note_name(&name).is_err() {
                    continue;
                }
                let (Ok(meta), Ok(text)) = (
                    entry.metadata().await,
                    tokio::fs::read_to_string(&path).await,
                ) else {
                    continue;
                };
                notes.push(MemoryNote {
                    name,
                    preview: text.chars().take(240).collect(),
                    bytes: meta.len(),
                });
            }
        }
        // Notes are named by day, so this is newest first.
        notes.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(MemoryView {
            enabled: crate::runtime::memory_enabled(&self.0.store).await,
            today: today_note(),
            notes,
            habits: self.0.store.list_global_memories(100).await?,
        })
    }

    /// Turns memory on or off for every project, from the next turn: off,
    /// the model cannot search notes and is not shown habits. Nothing is
    /// deleted.
    pub async fn set_memory_enabled(&self, enabled: bool) -> Result<()> {
        self.0
            .store
            .set_setting(crate::runtime::MEMORY_ENABLED, &enabled.to_string())
            .await
    }

    async fn memory_note_path(&self, project: &str, name: &str) -> Result<std::path::PathBuf> {
        let name = memory_note_name(name)?;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        Ok(wisp_core::MemoryManager::at(&root).dir().join(name))
    }

    /// The whole text of one note; empty when it does not exist yet.
    pub async fn memory_note(&self, project: &str, name: &str) -> Result<String> {
        let path = self.memory_note_path(project, name).await?;
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// Replaces one note with `content`.
    pub async fn save_memory_note(&self, project: &str, name: &str, content: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        let path = self.memory_note_path(project, name).await?;
        ensure!(
            !content.trim().is_empty(),
            "A note cannot be empty; delete it instead"
        );
        ensure!(
            content.len() <= NOTE_LIMIT,
            "A note holds at most {NOTE_LIMIT} bytes"
        );
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&path, content).await?;
        self.memory_changed(project).await
    }

    pub async fn delete_memory_note(&self, project: &str, name: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        let path = self.memory_note_path(project, name).await?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => self.memory_changed(project).await,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bail!("That note no longer exists")
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Project notes are research state other devices sync.
    async fn memory_changed(&self, project: &str) -> Result<()> {
        self.0
            .store
            .bump_state_generation(&wisp_store::StateScope::mainline(project.to_owned()))
            .await?;
        Ok(())
    }

    /// Keeps `text` for later turns: with `everywhere` as a habit for every
    /// project, otherwise at the end of today's note in this project.
    /// `session` says which conversation it came from, when it came from one.
    pub async fn remember(
        &self,
        project: &str,
        session: Option<&str>,
        text: &str,
        everywhere: bool,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        let text = confirmed_memory(text)?;
        if let Some(session) = session {
            self.own_conversation(project, session).await?;
        }
        if everywhere {
            self.known_project(project).await?;
            return self.insert_habit(text, session).await.map(|_| ());
        }
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let entry = match session {
            Some(session) => {
                let short: String = session.chars().take(8).collect();
                format!("[MEMORY · session {short}]\n{text}")
            }
            None => text,
        };
        // Appending is a short blocking write to a local file.
        tokio::task::spawn_blocking(move || wisp_core::MemoryManager::new(&root).append(&entry))
            .await??;
        self.memory_changed(project).await
    }

    async fn insert_habit(&self, content: String, session: Option<&str>) -> Result<GlobalMemory> {
        let now = unix_now();
        let habit = GlobalMemory {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            source_frame_id: session.map(str::to_owned),
            source_turn_index: None,
            created_at: now,
            updated_at: now,
        };
        self.0.store.insert_global_memory(&habit).await?;
        Ok(habit)
    }

    /// Adds a habit: something about how the user works that the model is
    /// shown in every project while memory is on.
    pub async fn add_habit(&self, content: &str) -> Result<GlobalMemory> {
        self.insert_habit(confirmed_memory(content)?, None).await
    }

    pub async fn update_habit(&self, id: &str, content: &str) -> Result<()> {
        let content = confirmed_memory(content)?;
        let updated = self
            .0
            .store
            .update_global_memory(id, &content, unix_now())
            .await?;
        ensure!(updated, "That habit no longer exists");
        Ok(())
    }

    pub async fn delete_habit(&self, id: &str) -> Result<()> {
        self.0.store.delete_global_memory(id).await
    }

    /// The favorites library. It is one database for every project, apart
    /// from the conversations, so deleting a project or a conversation does
    /// not take what was saved from it.
    async fn library(&self) -> Result<&wisp_store::LibraryStore> {
        self.0
            .library
            .get_or_try_init(|| async {
                wisp_store::LibraryStore::open(&self.0.config.data.join("library.sqlite")).await
            })
            .await
    }

    /// Saved items, newest first: everything, or what matches `query` in
    /// its title, text, project or conversation, of one `kind` (`code`,
    /// `figure`, `text`) when given.
    pub async fn favorites(
        &self,
        query: &str,
        kind: Option<&str>,
    ) -> Result<Vec<LibraryItemSummary>> {
        let library = self.library().await?;
        let query = query.trim();
        let mut items = if query.is_empty() && kind.is_none() {
            library.list_summaries().await?
        } else {
            library.search_summaries(query, kind).await?
        };
        items.truncate(FAVORITES_LIMIT);
        Ok(items)
    }

    /// One saved item in full, with the picture when it is a figure.
    pub async fn favorite(&self, id: &str) -> Result<Favorite> {
        let detail = self
            .library()
            .await?
            .get(id)
            .await?
            .context("That item is no longer in your favorites")?;
        Ok(Favorite {
            item: detail.item,
            image: detail.content,
        })
    }

    /// What was saved from one conversation, newest first.
    pub async fn conversation_favorites(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Vec<LibraryItem>> {
        self.own_conversation(project, session).await?;
        self.library().await?.list_for_session(session).await
    }

    /// Where a saved item says it came from.
    async fn favorite_source(
        &self,
        project: &str,
        session: &str,
    ) -> Result<wisp_store::SessionSearchResult> {
        self.own_conversation(project, session).await?;
        self.0
            .store
            .get_session_reference(session)
            .await?
            .context("Conversation not found")
    }

    /// Saves a piece of a conversation as it reads now: `kind` is `text`
    /// for prose or `code` with its `language`. Saving the same thing from
    /// the same conversation again returns the item that is already there.
    pub async fn star_text(
        &self,
        project: &str,
        session: &str,
        kind: &str,
        language: Option<&str>,
        text: &str,
    ) -> Result<LibraryItem> {
        ensure!(
            matches!(kind, "text" | "code"),
            "Only text and code are saved this way"
        );
        ensure!(!text.trim().is_empty(), "There is nothing to save");
        ensure!(
            text.len() <= FAVORITE_TEXT_LIMIT,
            "That is too large to save to your favorites"
        );
        let source = self.favorite_source(project, session).await?;
        self.library()
            .await?
            .insert(wisp_store::NewLibraryItem {
                kind: kind.into(),
                title: favorite_title(text),
                language: language
                    .map(str::trim)
                    .filter(|language| !language.is_empty() && kind == "code")
                    .map(str::to_owned),
                code: text.to_owned(),
                content_type: None,
                content: None,
                source_project_id: source.project_id,
                source_project_name: source.project_name,
                source_session_id: source.id,
                source_session_title: source.title,
                source_path: None,
            })
            .await
    }

    /// Saves a copy of an image file of the project, as it is now.
    pub async fn star_figure(
        &self,
        project: &str,
        session: &str,
        path: &str,
    ) -> Result<LibraryItem> {
        let source = self.favorite_source(project, session).await?;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let real = crate::files::resolve(&root, path).await?;
        let content_type = image_type(&real).context("Only images are saved as figures")?;
        let meta = tokio::fs::metadata(&real).await?;
        ensure!(meta.is_file(), "That is not a file");
        ensure!(
            meta.len() <= FIGURE_LIMIT,
            "Figures of more than 32 MB are not saved to your favorites"
        );
        let content = tokio::fs::read(&real).await?;
        let title = real
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Figure".into());
        self.library()
            .await?
            .insert(wisp_store::NewLibraryItem {
                kind: "figure".into(),
                title,
                language: None,
                code: String::new(),
                content_type: Some(content_type.into()),
                content: Some(content),
                source_project_id: source.project_id,
                source_project_name: source.project_name,
                source_session_id: source.id,
                source_session_title: source.title,
                source_path: Some(path.trim_start_matches("./").to_owned()),
            })
            .await
    }

    /// Takes an item out of the favorites for good.
    pub async fn unstar(&self, id: &str) -> Result<()> {
        ensure!(
            self.library().await?.delete(id).await?,
            "That item is no longer in your favorites"
        );
        Ok(())
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
        // Hosts belong to the app, not to a project, so the choice holds.
        self.store_runs_on(&id, &options.runs_on).await?;
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
        self.rewind_locked(&live, session, turn).await
    }

    /// Whether the conversation can be rewound to `turn`, and the row the
    /// turn's message is in.
    pub(crate) async fn rewind_target(&self, session: &str, turn: &TurnRef) -> Result<i64> {
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
        Ok(self.locate_turn(session, turn).await?.0)
    }

    /// The rewind itself; the caller holds the conversation's accept lock.
    pub(crate) async fn rewind_locked(
        &self,
        live: &std::sync::Arc<crate::output::Session>,
        session: &str,
        turn: &TurnRef,
    ) -> Result<String> {
        let store = &self.0.store;
        let seq = self.rewind_target(session, turn).await?;
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
            // So do the questions and the review that followed it.
            live.follow_ups.clear();
            live.review = None;
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
            // Where it split off, as the WebView records it: after the reply
            // to the last turn it kept. That is what lets it be brought back.
            let kept = messages
                .iter()
                .filter(|message| message.role == wisp_llm::Role::User)
                .count();
            store
                .set_session_branch_point(&id, session, kept - 1, "after_response")
                .await?;
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
            self.store_runs_on(&id, &options.runs_on).await?;
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
        let disabled = crate::runtime::disabled_skills(&self.0.store).await;
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        Ok(tokio::task::spawn_blocking(move || {
            crate::runtime::all_skills(&config, Some(&root))
                .filtered(&disabled)
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

    /// The routes the host's own traffic takes and where packages are
    /// installed from.
    pub async fn network(&self) -> Result<wisp_dto::NetworkSettings> {
        Ok(crate::network::load(&self.0.store).await)
    }

    /// Saves the routes, or changes nothing and says which field is wrong.
    /// Model calls and connectors follow from the next turn; programs the
    /// tools start and MCP servers follow at once, the latter by connecting
    /// again.
    pub async fn set_network(
        &self,
        settings: wisp_dto::NetworkSettings,
    ) -> Result<wisp_dto::NetworkSettings> {
        let _admission = self.0.admission.read().await;
        let saved = crate::network::save(&self.0.store, settings).await?;
        wisp_tools::network::set_command_proxy(&saved.command_proxy_url);
        self.0.mcp.set_proxy(&saved.mcp_proxy_url);
        self.0.mcp.close_all().await;
        Ok(saved)
    }

    /// The SSH hosts work can run on, which of them (if any) new work goes
    /// to, and the aliases in the user's SSH config that are not added yet.
    pub async fn environments(&self) -> Result<EnvironmentsView> {
        use wisp_runs::ssh_hosts;
        let store = &self.0.store;
        let hosts = ssh_hosts::load(store).await;
        let aliases = tokio::task::spawn_blocking(ssh_hosts::list_ssh_config_aliases)
            .await?
            .into_iter()
            .filter(|alias| !hosts.iter().any(|host| &host.alias == alias))
            .collect();
        let mut checks = Vec::new();
        for host in &hosts {
            let context = store
                .get_execution_context(&format!("ssh:{}", host.alias))
                .await?;
            checks.extend(context.and_then(|context| EnvironmentCheck::of(&host.alias, &context)));
        }
        Ok(EnvironmentsView {
            checks,
            default: ssh_hosts::stored_default_execution_context(store)
                .await
                .and_then(|id| id.strip_prefix("ssh:").map(str::to_owned)),
            hosts: hosts.into_iter().map(SshHostRow::from).collect(),
            config_aliases: aliases,
        })
    }

    /// Adds an SSH host, or changes the one that has the alias. Wisp keeps
    /// where the key file is, never the key.
    pub async fn save_ssh_host(&self, input: SshHostRow) -> Result<()> {
        use wisp_runs::ssh_hosts;
        let _admission = self.0.admission.read().await;
        let host = input.into_host()?;
        let hosts = ssh_hosts::upsert_host(ssh_hosts::load(&self.0.store).await, host);
        ssh_hosts::save_and_sync_contexts(&self.0.store, &hosts)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Adds the hosts of `~/.ssh/config` that are named here and not added
    /// yet; OpenSSH keeps resolving their address, user and key from that
    /// file.
    pub async fn import_ssh_hosts(&self, aliases: &[String]) -> Result<()> {
        use wisp_runs::ssh_hosts;
        let _admission = self.0.admission.read().await;
        let known = tokio::task::spawn_blocking(ssh_hosts::list_ssh_config_aliases).await?;
        let wanted: Vec<String> = aliases
            .iter()
            .filter(|alias| known.contains(alias))
            .cloned()
            .collect();
        ensure!(
            !wanted.is_empty(),
            "None of those hosts is in your SSH config"
        );
        let hosts = ssh_hosts::merge_config_aliases(ssh_hosts::load(&self.0.store).await, wanted);
        ssh_hosts::save_and_sync_contexts(&self.0.store, &hosts)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Connects to an SSH host now and records what it is: system,
    /// processors, GPUs, scheduler, interpreters. Work runs on a host only
    /// after a check has passed. A check that fails is recorded too and is
    /// returned, not raised; the error is for a host that is not saved.
    pub async fn check_environment(&self, alias: &str) -> Result<EnvironmentCheck> {
        self.check_environment_with(alias, &mut wisp_runs::context_probe::ProcessProbeRunner)
            .await
    }

    pub(crate) async fn check_environment_with(
        &self,
        alias: &str,
        runner: &mut dyn wisp_runs::context_probe::ProbeRunner,
    ) -> Result<EnvironmentCheck> {
        use wisp_runs::{context_probe, ssh_hosts};
        let hosts = ssh_hosts::load(&self.0.store).await;
        ensure!(
            hosts.iter().any(|host| host.alias == alias),
            "That host is no longer saved"
        );
        // A host whose execution context went missing gets it back first;
        // what earlier checks found is kept.
        ssh_hosts::save_and_sync_contexts(&self.0.store, &hosts)
            .await
            .map_err(anyhow::Error::msg)?;
        let context = context_probe::probe_and_store_with_runner(
            &self.0.store,
            &format!("ssh:{alias}"),
            runner,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        EnvironmentCheck::of(alias, &context).context("The check was not recorded")
    }

    /// One folder of a saved SSH host, to look at: `path` empty is the
    /// home folder. The host has to have passed a check, like for runs.
    /// Nothing on the host is changed.
    pub async fn remote_directory(&self, alias: &str, path: &str) -> Result<RemoteDirectory> {
        self.remote_directory_with(alias, path, self.0.runs.runner_ref())
            .await
    }

    pub(crate) async fn remote_directory_with(
        &self,
        alias: &str,
        path: &str,
        runner: &dyn wisp_runs::RunCommandRunner,
    ) -> Result<RemoteDirectory> {
        use wisp_runs::ssh_hosts;
        let context = self
            .0
            .store
            .get_execution_context(&format!("ssh:{alias}"))
            .await?
            .context("That host is no longer saved")?;
        ssh_hosts::require_managed_ssh_ready(&context).map_err(anyhow::Error::msg)?;
        let connection = ssh_hosts::SshConnection::from_execution_context(&context)
            .map_err(anyhow::Error::msg)?;
        let command = crate::remote_dir::command(&connection, path)?;
        let output = runner
            .run(command, crate::remote_dir::TIMEOUT)
            .await
            .map_err(anyhow::Error::msg)?;
        crate::remote_dir::parse(alias, output)
    }

    /// Forgets an SSH host. Runs that were made on it keep their records.
    pub async fn remove_ssh_host(&self, alias: &str) -> Result<()> {
        use wisp_runs::ssh_hosts;
        let _admission = self.0.admission.read().await;
        let hosts = ssh_hosts::load(&self.0.store).await;
        ensure!(
            hosts.iter().any(|host| host.alias == alias),
            "That host is no longer saved"
        );
        ssh_hosts::save(&self.0.store, &ssh_hosts::remove_host(hosts, alias))
            .await
            .map_err(anyhow::Error::msg)?;
        // A default that pointed at it is cleared when it is next read.
        ssh_hosts::remove_context_for_alias(&self.0.store, alias)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Says where new work runs unless told otherwise: on the host with
    /// this alias, or with `None` on this computer.
    pub async fn set_default_environment(&self, alias: Option<&str>) -> Result<()> {
        use wisp_runs::ssh_hosts;
        let id = match alias {
            Some(alias) => {
                ensure!(
                    ssh_hosts::load(&self.0.store)
                        .await
                        .iter()
                        .any(|host| host.alias == alias),
                    "That host is no longer saved"
                );
                format!("ssh:{alias}")
            }
            None => String::new(),
        };
        self.0
            .store
            .set_setting(ssh_hosts::DEFAULT_EXECUTION_CONTEXT_KEY, &id)
            .await
    }

    fn mcp_secret(key_id: &str, server: &str) -> String {
        format!("desktop.mcp.{key_id}.{server}")
    }

    /// The token saved for a server, if there is one.
    async fn mcp_token(&self, server: &str) -> Option<String> {
        let key_id = self.0.store.get_setting(MODEL_KEY_ID).await.ok()??;
        self.0
            .secrets
            .get(&Self::mcp_secret(&key_id, server))
            .filter(|token| !token.is_empty())
    }

    /// The tools the user's enabled MCP servers give a turn.
    pub(crate) async fn mcp_tools(&self) -> Vec<Box<dyn wisp_tools::Tool>> {
        let servers = crate::mcp::load(&self.0.store).await;
        if !servers.iter().any(|server| server.enabled) {
            return vec![];
        }
        let mut tokens = HashMap::new();
        for server in &servers {
            if let Some(token) = self.mcp_token(&server.id).await {
                tokens.insert(server.id.clone(), token);
            }
        }
        self.0
            .mcp
            .tools(&servers, |id| tokens.get(id).cloned())
            .await
    }

    /// The MCP servers the user added, in the order they were added, each
    /// with whether a token is saved for it and why it gave no tools the
    /// last time, if it did not.
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        let mut rows = Vec::new();
        for server in crate::mcp::load(&self.0.store).await {
            rows.push(McpServerRow {
                has_token: self.mcp_token(&server.id).await.is_some(),
                error: self.0.mcp.error(&server.id),
                server,
            });
        }
        Ok(rows)
    }

    /// Adds a server, or with an id changes the one that has it. A token is
    /// kept in the system keychain: `Some("")` forgets it, `None` leaves it.
    pub async fn save_mcp_server(&self, input: McpServerInput) -> Result<McpServer> {
        let mut servers = crate::mcp::load(&self.0.store).await;
        let existing = input
            .id
            .as_deref()
            .map(|id| {
                servers
                    .iter()
                    .position(|server| server.id == id)
                    .context("That server is no longer saved")
            })
            .transpose()?;
        let server = McpServer {
            id: input
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: input.name.trim().to_owned(),
            enabled: existing.is_none_or(|index| servers[index].enabled),
            transport: input.transport,
        };
        server.validate()?;
        ensure!(
            !servers
                .iter()
                .any(|other| other.id != server.id && other.slug() == server.slug()),
            "Another server already has that name"
        );
        if let Some(token) = input.token {
            let token = token.trim();
            ensure!(
                token.len() <= 4_096 && !token.contains(char::is_whitespace),
                "A token is one word of at most 4096 characters"
            );
            let key_id = self.key_id().await?;
            self.0
                .secrets
                .set(&Self::mcp_secret(&key_id, &server.id), token)?;
        }
        match existing {
            Some(index) => servers[index] = server.clone(),
            None => servers.push(server.clone()),
        }
        crate::mcp::save(&self.0.store, &servers).await?;
        // The next turn connects afresh, as the server is now written.
        self.0.mcp.close(&server.id).await;
        Ok(server)
    }

    /// Turns a server off or on, from the next turn.
    pub async fn set_mcp_server_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let mut servers = crate::mcp::load(&self.0.store).await;
        let server = servers
            .iter_mut()
            .find(|server| server.id == id)
            .context("That server is no longer saved")?;
        server.enabled = enabled;
        crate::mcp::save(&self.0.store, &servers).await?;
        if !enabled {
            self.0.mcp.close(id).await;
        }
        Ok(())
    }

    /// Forgets a server and its token, and closes the connection to it.
    pub async fn delete_mcp_server(&self, id: &str) -> Result<()> {
        let mut servers = crate::mcp::load(&self.0.store).await;
        let before = servers.len();
        servers.retain(|server| server.id != id);
        ensure!(servers.len() < before, "That server is no longer saved");
        crate::mcp::save(&self.0.store, &servers).await?;
        if let Some(key_id) = self.0.store.get_setting(MODEL_KEY_ID).await? {
            self.0.secrets.set(&Self::mcp_secret(&key_id, id), "")?;
        }
        self.0.mcp.close(id).await;
        Ok(())
    }

    /// Connects to a server now and says what it offers, by the names the
    /// model would call the tools by.
    pub async fn test_mcp_server(&self, id: &str) -> Result<Vec<String>> {
        let servers = crate::mcp::load(&self.0.store).await;
        let server = servers
            .iter()
            .find(|server| server.id == id)
            .context("That server is no longer saved")?;
        let token = self.mcp_token(id).await;
        Ok(self
            .0
            .mcp
            .list(server, token)
            .await?
            .iter()
            .filter(|tool| tool.visible_to_model())
            .map(|tool| server.tool_name(&tool.name))
            .collect())
    }

    fn service_secret(key_id: &str, id: &str) -> String {
        format!("desktop.service.{key_id}.{id}")
    }

    /// The service keys that are saved, as `(NAME, value)` the way the
    /// services' own documentation names them. Read at the start of a turn.
    pub(crate) async fn service_keys(&self) -> Vec<(String, String)> {
        // No id yet means nothing was ever saved; do not make one to look.
        let Ok(Some(key_id)) = self.0.store.get_setting(MODEL_KEY_ID).await else {
            return vec![];
        };
        SERVICE_KEYS
            .iter()
            .filter_map(|key| {
                let value = self.0.secrets.get(&Self::service_secret(&key_id, key.id))?;
                // A forgotten key is stored empty.
                (!value.is_empty()).then(|| (key.env.to_owned(), value))
            })
            .collect()
    }

    /// The service keys Wisp can use, with whether each is saved. The
    /// values themselves are never handed back.
    pub async fn credentials(&self) -> Result<Vec<CredentialRow>> {
        let key_id = self.0.store.get_setting(MODEL_KEY_ID).await?;
        Ok(SERVICE_KEYS
            .iter()
            .map(|key| CredentialRow {
                id: key.id,
                label: key.label,
                note: key.note,
                secret: key.secret,
                saved: key_id.as_deref().is_some_and(|key_id| {
                    self.0
                        .secrets
                        .get(&Self::service_secret(key_id, key.id))
                        .is_some_and(|value| !value.is_empty())
                }),
            })
            .collect())
    }

    /// Saves a service key in the system keychain, or with an empty value
    /// forgets it. It is used from the next turn.
    pub async fn set_credential(&self, id: &str, value: &str) -> Result<()> {
        let key = SERVICE_KEYS
            .iter()
            .find(|key| key.id == id)
            .context("There is no such credential")?;
        let value = value.trim();
        ensure!(
            value.len() <= 512 && !value.contains(char::is_whitespace),
            "{} is one word of at most 512 characters",
            key.label
        );
        let key_id = self.key_id().await?;
        self.0
            .secrets
            .set(&Self::service_secret(&key_id, key.id), value)
    }

    /// Tokens used so far: by project, by model, and by day over the last
    /// year. Counted from what each model call reported when it was made.
    pub async fn usage(&self) -> Result<UsageView> {
        let store = &self.0.store;
        Ok(UsageView {
            projects: store
                .token_usage_by_project()
                .await?
                .into_iter()
                .map(|row| ProjectUsage {
                    project_id: row.project_id,
                    name: row.name,
                    conversations: row.session_count.max(0) as u64,
                    input: row.input.max(0) as u64,
                    output: row.output.max(0) as u64,
                    reasoning: row.reasoning.max(0) as u64,
                    cached: row.cached.max(0) as u64,
                })
                .collect(),
            models: store
                .token_usage_by_model()
                .await?
                .into_iter()
                .map(|row| (row.model, row.tokens.max(0) as u64))
                .collect(),
            days: store
                .token_usage_activity()
                .await?
                .into_iter()
                .filter(|day| !day.future)
                .map(|day| (day.date, day.tokens.max(0) as u64))
                .collect(),
        })
    }

    /// Where Wisp keeps its data and how much room it takes: the database,
    /// the favorites, and what it keeps inside each project's folder.
    pub async fn storage(&self) -> Result<StorageView> {
        let data = self.0.config.data.clone();
        let projects = self.0.store.list_projects().await?;
        Ok(tokio::task::spawn_blocking(move || {
            let file = |name: &str| {
                // The write-ahead log and its index are part of the database.
                ["", "-wal", "-shm"]
                    .iter()
                    .filter_map(|suffix| {
                        std::fs::metadata(data.join(format!("{name}{suffix}"))).ok()
                    })
                    .map(|meta| meta.len())
                    .sum::<u64>()
            };
            StorageView {
                database_bytes: file("wisp.sqlite"),
                favorites_bytes: file("library.sqlite"),
                projects: projects
                    .into_iter()
                    .map(|(id, name, workspace, _, _, conversations, ..)| {
                        let root = std::path::Path::new(&workspace);
                        ProjectStorage {
                            conversations: conversations.max(0) as usize,
                            wisp_bytes: folder_size(&root.join(".wisp")),
                            uploads_bytes: folder_size(&root.join("uploads")),
                            missing: !root.is_dir(),
                            project_id: id,
                            name,
                            workspace_dir: workspace,
                        }
                    })
                    .collect(),
                data_dir: data.to_string_lossy().into_owned(),
            }
        })
        .await?)
    }

    /// The literature and biology connectors that ship with Wisp, by name,
    /// with the tools each gives the model and whether it is turned on.
    pub async fn connectors(&self) -> Result<Vec<ConnectorRow>> {
        let off = crate::runtime::disabled_connectors(&self.0.store).await;
        let mut rows: Vec<ConnectorRow> = Vec::new();
        for (domain, schema) in wisp_bio::catalog() {
            let tool = schema.function.name;
            match rows.iter_mut().find(|row| row.domain == domain) {
                Some(row) => row.tools.push(tool),
                None => {
                    let about = wisp_bio::domain_metadata(domain);
                    rows.push(ConnectorRow {
                        domain: domain.to_owned(),
                        description: about
                            .map(|about| about.description.clone())
                            .unwrap_or_default(),
                        links: about
                            .map(|about| {
                                about
                                    .links
                                    .iter()
                                    .map(|link| (link.label.clone(), link.url.clone()))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        tools: vec![tool],
                        enabled: !off.contains(domain),
                    });
                }
            }
        }
        rows.sort_by(|a, b| a.domain.cmp(&b.domain));
        Ok(rows)
    }

    /// Turns a bundled connector off or on, from the next turn. Off, its
    /// tools are not given to the model.
    pub async fn set_connector_enabled(&self, domain: &str, enabled: bool) -> Result<()> {
        ensure!(
            wisp_bio::catalog()
                .iter()
                .any(|(known, _)| *known == domain),
            "There is no such connector"
        );
        let mut off = crate::runtime::disabled_connectors(&self.0.store).await;
        if enabled {
            off.remove(domain);
        } else {
            off.insert(domain.to_owned());
        }
        let mut domains: Vec<_> = off.into_iter().collect();
        domains.sort();
        self.0
            .store
            .set_setting(
                crate::runtime::DISABLED_CONNECTORS,
                &serde_json::to_string(&domains)?,
            )
            .await
    }

    /// Every skill there is for `project` (or, without one, those the host
    /// has for all projects), by name, with whether it is turned on.
    pub async fn skills(&self, project: Option<&str>) -> Result<Vec<SkillRow>> {
        let root = match project {
            Some(project) => Some(std::path::PathBuf::from(self.project_root(project).await?)),
            None => None,
        };
        let config = self.0.config.clone();
        let disabled = crate::runtime::disabled_skills(&self.0.store).await;
        Ok(tokio::task::spawn_blocking(move || {
            let index = crate::runtime::all_skills(&config, root.as_deref());
            let mut rows: Vec<SkillRow> = index
                .all()
                .iter()
                .map(|skill| SkillRow {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    tags: skill.tags.clone(),
                    source: index
                        .source(&skill.name)
                        .map_or("extra", wisp_skills::SkillSource::as_str),
                    enabled: !disabled.contains(&skill.name),
                })
                .collect();
            rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            rows
        })
        .await?)
    }

    /// Turns a skill off or on for every project, from the next turn. Off,
    /// it is not offered to the model and cannot be attached to a message.
    pub async fn set_skill_enabled(&self, name: &str, enabled: bool) -> Result<()> {
        let name = name.trim();
        ensure!(!name.is_empty(), "Name the skill");
        let mut disabled = crate::runtime::disabled_skills(&self.0.store).await;
        if enabled {
            disabled.remove(name);
        } else {
            disabled.insert(name.to_owned());
        }
        let mut names: Vec<_> = disabled.into_iter().collect();
        names.sort();
        self.0
            .store
            .set_setting(
                crate::runtime::DISABLED_SKILLS,
                &serde_json::to_string(&names)?,
            )
            .await
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

    /// The tool call that last wrote a project file, if one of the
    /// project's conversations did: looked for in `session` first, when
    /// one is given, then in the others. A file the user put there, or one
    /// written before this was kept, has none.
    pub async fn file_origin(
        &self,
        project: &str,
        session: Option<&str>,
        path: &str,
    ) -> Result<Option<FileOrigin>> {
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        // The folder may be spelt its real way or the way it was opened.
        let real = tokio::fs::canonicalize(&root).await.unwrap_or(root.clone());
        let spelt = tokio::fs::canonicalize(path)
            .await
            .ok()
            .filter(|_| std::path::Path::new(path).is_absolute())
            .map(|found| found.to_string_lossy().into_owned());
        let path = crate::origin::relative(&root, path)
            .or_else(|| crate::origin::relative(&real, path))
            .or_else(|| spelt.and_then(|spelt| crate::origin::relative(&real, &spelt)))
            .context("That is not a project file")?;
        let mut frames = self.0.store.list_project_frame_ids(project).await?;
        if let Some(open) = session {
            // Only a conversation of this project is looked in.
            if let Some(at) = frames.iter().position(|frame| frame == open) {
                frames.swap(0, at);
            }
        }
        crate::origin::find(&self.0.store, &frames, &path).await
    }

    /// What a run wrote in full, or the end of it when it is very long.
    /// Only a run on this computer keeps its log where it can be read; a
    /// run on another machine has its last lines in the run record.
    pub async fn run_log(&self, project: &str, run: &str) -> Result<RunLog> {
        self.project_root(project).await?;
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        ensure!(
            record.kind == "local_detached",
            "This run's log is on the machine it ran on; its last lines are in the output"
        );
        let home = std::env::home_dir().context("The home folder could not be found")?;
        let folder = crate::run_log::folder(
            &home,
            record.remote_workdir.as_deref().unwrap_or_default(),
            &record.id,
        )?;
        tokio::task::spawn_blocking(move || crate::run_log::read(&folder)).await?
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
            auto_review: crate::review::automatic(&self.0.store, session).await,
            // One that was removed since reads as none.
            expert: crate::experts::of_conversation(&self.0.store, session)
                .await
                .map(|expert| expert.id)
                .unwrap_or_default(),
            runs_on: runs_on(
                &wisp_runs::ssh_hosts::stored_session_default_execution_context(
                    &self.0.store,
                    session,
                )
                .await,
            ),
            acp_agent: crate::acp_turn::chosen(&self.0.store, session)
                .await?
                .unwrap_or_default(),
        })
    }

    /// Saves where a conversation's work runs. Nothing is written when it
    /// is already so, which also leaves alone a choice this desktop cannot
    /// offer but another one made.
    async fn store_runs_on(&self, session: &str, wanted: &str) -> Result<()> {
        use wisp_runs::ssh_hosts::{self, SessionDefaultExecutionContext as Choice};
        let store = &self.0.store;
        let wanted = wanted.trim();
        let current = ssh_hosts::stored_session_default_execution_context(store, session).await;
        if runs_on(&current) == wanted {
            return Ok(());
        }
        let choice = match wanted {
            "" => Choice::Unset,
            RUNS_LOCALLY => Choice::Local,
            alias => {
                ensure!(
                    ssh_hosts::load(store)
                        .await
                        .iter()
                        .any(|host| host.alias == alias),
                    "That host is no longer saved; choose another"
                );
                Choice::Remote(format!("ssh:{alias}"))
            }
        };
        ssh_hosts::persist_session_default_execution_context(store, session, choice)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(())
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
        let acp = crate::acp_turn::controls(&session).await;
        let runtimes = session
            .runtime
            .lock()
            .unwrap()
            .as_ref()
            .map(|manager| manager.list())
            .unwrap_or_default()
            .into_iter()
            .map(RuntimeRow::from)
            .collect();
        let live = session.live.lock().unwrap();
        let mut allowed_tools: Vec<_> = live.allowed_tools.iter().cloned().collect();
        allowed_tools.sort();
        let review = match (&live.review, live.reviewing) {
            (_, true) => Some(ReviewOutcome::Reading),
            (Some(Ok(found)), false) => Some(ReviewOutcome::Done(found.clone())),
            (Some(Err(problem)), false) => Some(ReviewOutcome::Failed(problem.clone())),
            (None, false) => None,
        };
        Ok(ConversationStatus {
            options,
            usage: live.usage.clone(),
            allowed_tools,
            reasoning: live.reasoning.clone(),
            runtimes,
            review,
            acp,
        })
    }

    /// Asks the project's release page whether a newer version has been
    /// published. Nothing is downloaded; the request says nothing about
    /// the user or their work.
    pub async fn check_for_update(&self) -> Result<UpdateCheck> {
        crate::updates::check().await
    }

    /// The research archive of a conversation, if it has been frozen.
    pub async fn research_archive(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Option<ResearchArchive>> {
        self.own_conversation(project, session).await?;
        Ok(self
            .0
            .store
            .research_archive(session)
            .await?
            .filter(|archive| archive.frozen_at.is_some()))
    }

    /// Has the conversation's model write a draft archive of it: a report
    /// and the scripts that can be put together from what was run. Nothing
    /// is saved; the researcher reviews the draft and freezes it.
    pub async fn draft_research_archive(
        &self,
        project: &str,
        session: &str,
    ) -> Result<ArchiveDraft> {
        self.archivable(project, session).await?;
        let store = &self.0.store;
        let options = self.stored_options(session).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let routes = crate::network::load(store).await;
        if let Some(route) = crate::network::route(&routes.model_proxy_url) {
            model.proxy = Some(route);
        }
        // A draft is a document to write, not a problem to think about.
        model.reasoning_effort = None;
        let messages = self.whole_transcript(session).await?;
        let provider = (self.0.models)(model);
        let transcript = crate::review::transcript(&messages);
        match crate::research_archive::draft(provider.as_ref(), transcript).await {
            Ok(draft) => Ok(draft),
            Err(error) => bail!(
                "{}",
                crate::network::explain(store, &format!("{error:#}")).await
            ),
        }
    }

    /// The conversations Codex CLI and Claude Code keep on this computer,
    /// newest first, with whether each is in this project already.
    pub async fn external_conversations(&self, project: &str) -> Result<Vec<ExternalConversation>> {
        let home = crate::session_import::home().context("The home folder is not known")?;
        self.external_in(project, home).await
    }

    pub(crate) async fn external_in(
        &self,
        project: &str,
        home: std::path::PathBuf,
    ) -> Result<Vec<ExternalConversation>> {
        self.project_root(project).await?;
        let found = tokio::task::spawn_blocking(move || crate::session_import::list(&home)).await?;
        let mut listed = Vec::with_capacity(found.len());
        for external in found {
            let imported = match &external.key {
                Some(key) => self.existing_import(project, key).await?.is_some(),
                None => false,
            };
            listed.push(ExternalConversation {
                source: external.source.label(),
                path: external.path,
                title: external.title,
                folder: external.folder,
                modified_ms: external.modified_ms,
                size: external.size,
                imported,
            });
        }
        Ok(listed)
    }

    /// The conversation an earlier import of `key` made in this project.
    async fn existing_import(&self, project: &str, key: &str) -> Result<Option<String>> {
        let store = &self.0.store;
        for key in [
            crate::session_import::project_key(project, key),
            key.to_owned(),
        ] {
            if let Some(frame) = store.find_codex_import(&key).await? {
                if store.frame_project_id(&frame).await?.as_deref() == Some(project) {
                    return Ok(Some(frame));
                }
            }
        }
        Ok(None)
    }

    /// Brings one conversation into this project, in a group named after
    /// the tool, or brings in what was added to it since it was imported.
    /// Returns the conversation and what was done.
    pub async fn import_external_conversation(
        &self,
        project: &str,
        path: &str,
    ) -> Result<(String, ImportOutcome)> {
        let home = crate::session_import::home().context("The home folder is not known")?;
        self.import_external_in(project, &home, path).await
    }

    pub(crate) async fn import_external_in(
        &self,
        project: &str,
        home: &std::path::Path,
        path: &str,
    ) -> Result<(String, ImportOutcome)> {
        self.project_root(project).await?;
        let (home, wanted) = (home.to_owned(), path.to_owned());
        let (source, parsed) =
            tokio::task::spawn_blocking(move || crate::session_import::read(&home, &wanted))
                .await??;
        let store = &self.0.store;
        let key = source.import_key(&parsed.session_id);
        let messages = crate::session_import::messages(source, &parsed);
        let now = chrono::Utc::now().timestamp();
        let seconds = |ms: i64| if ms > 0 { ms / 1000 } else { now };
        let (created, updated) = (
            seconds(parsed.created_at_ms),
            seconds(parsed.last_active_at_ms),
        );
        let recorded = crate::session_import::project_key(project, &key);
        if let Some(frame) = self.existing_import(project, &key).await? {
            self.ensure_idle(&frame, "import into")?;
            // Only forward: a conversation continued here keeps its turns.
            if messages.len() as i64 <= store.message_count(&frame).await? {
                return Ok((frame, ImportOutcome::Unchanged));
            }
            store.replace_messages(&frame, &messages).await?;
            store.set_frame_timestamps(&frame, created, updated).await?;
            store.record_codex_import(&recorded, &frame, path).await?;
            // Nothing in it is news to whoever had it.
            store.mark_frame_seen(&frame).await?;
            return Ok((frame, ImportOutcome::Updated));
        }
        let folder = match store
            .list_folders(project)
            .await?
            .into_iter()
            .find(|(_, name, _)| name.eq_ignore_ascii_case(source.folder()))
        {
            Some((id, ..)) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                store.create_folder(&id, project, source.folder()).await?;
                id
            }
        };
        let frame = uuid::Uuid::new_v4().to_string();
        store
            .create_frame(&frame, project, source.label(), &self.model_id(""))
            .await?;
        store
            .move_session_to_folder(&frame, project, Some(&folder))
            .await?;
        for (index, message) in messages.iter().enumerate() {
            store
                .append_message(&frame, index as i64 + 1, message)
                .await?;
        }
        store.set_frame_timestamps(&frame, created, updated).await?;
        store.record_codex_import(&recorded, &frame, path).await?;
        store.mark_frame_seen(&frame).await?;
        Ok((frame, ImportOutcome::Imported))
    }

    /// Where a branch stands for being brought back into the conversation
    /// it came from. Refused for a conversation that is not a branch or has
    /// been brought back already.
    pub async fn branch_merge(&self, project: &str, branch: &str) -> Result<BranchMerge> {
        self.own_conversation(project, branch).await?;
        let preview = self
            .0
            .store
            .preview_session_branch_merge(branch, project)
            .await?;
        Ok(BranchMerge {
            source: preview.main_session_id,
            new_messages: preview.new_message_count,
            guard: preview.guard_hash,
        })
    }

    /// Has the branch's model summarise what the branch did after it split
    /// off. `guard` is the one [`Self::branch_merge`] gave: a branch that
    /// changed since is summarised again first. Nothing is saved.
    pub async fn draft_branch_summary(
        &self,
        project: &str,
        branch: &str,
        guard: &str,
    ) -> Result<String> {
        self.own_conversation(project, branch).await?;
        let store = &self.0.store;
        let preview = store.preview_session_branch_merge(branch, project).await?;
        ensure!(
            preview.guard_hash == guard,
            "The branch changed before it could be summarized. Summarize it again."
        );
        let payload = crate::branch_merge::payload(&preview)?;
        let options = self.stored_options(branch).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let routes = crate::network::load(store).await;
        if let Some(route) = crate::network::route(&routes.model_proxy_url) {
            model.proxy = Some(route);
        }
        model.reasoning_effort = None;
        let provider = (self.0.models)(model);
        match crate::branch_merge::summarize(provider.as_ref(), payload).await {
            Ok(summary) => Ok(summary),
            Err(error) => bail!(
                "{}",
                crate::network::explain(store, &format!("{error:#}")).await
            ),
        }
    }

    /// Adds `summary` to the end of the conversation the branch came from,
    /// as one message the model reads from then on, and marks the branch
    /// brought back. Returns that conversation.
    pub async fn merge_branch(
        &self,
        project: &str,
        branch: &str,
        guard: &str,
        summary: &str,
    ) -> Result<String> {
        self.own_conversation(project, branch).await?;
        let merged = self
            .0
            .store
            .merge_session_branch_summary(branch, project, guard, summary)
            .await?;
        Ok(merged.main_session_id)
    }

    /// Freezes `draft` as the conversation's research archive: the report
    /// and scripts are written under `.wisp/research-archives/` in the
    /// project and the record is kept. From then on the conversation is
    /// read-only. No project file is moved or deleted.
    pub async fn freeze_research_archive(
        &self,
        project: &str,
        session: &str,
        draft: ArchiveDraft,
    ) -> Result<ResearchArchive> {
        let _admission = self.0.admission.write().await;
        self.archivable(project, session).await?;
        let draft = crate::research_archive::checked(draft)?;
        let store = &self.0.store;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let id = uuid::Uuid::new_v4().to_string();
        let (_, source_hash) = store.research_archive_source(session).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or_default();
        let folder = crate::research_archive::folder(&root, &id);
        let written = draft.clone();
        let target = folder.clone();
        tokio::task::spawn_blocking(move || crate::research_archive::write(&target, &written))
            .await??;
        let mut record =
            crate::research_archive::record(id, project, session, source_hash, draft, now);
        let kept = async {
            store.save_research_archive_draft(&record).await?;
            record.frozen_at = Some(now);
            store.freeze_research_archive(&record).await
        }
        .await;
        if let Err(error) = kept {
            // Nothing was frozen: the files written for it go again.
            let _ = tokio::fs::remove_dir_all(&folder).await;
            return Err(error);
        }
        // What the conversation kept running is no longer needed.
        let runtime = self.session(session).runtime.lock().unwrap().take();
        if let Some(runtime) = runtime {
            runtime.shutdown_all().await;
        }
        Ok(record)
    }

    /// A conversation can be archived once: when it is the project's own,
    /// no turn is running in it and it has not been archived before.
    async fn archivable(&self, project: &str, session: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        ensure!(
            !self.session(session).live.lock().unwrap().running,
            "Wait for the turn to end before you archive this conversation"
        );
        ensure!(
            !self.archived(session).await?,
            "This conversation's research is already archived"
        );
        Ok(())
    }

    /// The papers the project is writing, the one changed last first.
    pub async fn papers(&self, project: &str) -> Result<Vec<PaperRow>> {
        self.known_project(project).await?;
        crate::papers::list(&self.0.store, project).await
    }

    /// One paper at `version`, or at its newest version.
    pub async fn paper(&self, project: &str, paper: &str, version: Option<&str>) -> Result<Paper> {
        self.known_project(project).await?;
        crate::papers::open(&self.0.store, project, paper, version).await
    }

    /// Starts a paper with its first version.
    pub async fn create_paper(&self, project: &str, input: PaperInput) -> Result<Paper> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::create(&self.0.store, project, input).await
    }

    pub async fn rename_paper(
        &self,
        project: &str,
        paper: &str,
        title: &str,
        description: &str,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::rename(&self.0.store, project, paper, title, description).await
    }

    /// Removes a paper that has only drafts.
    pub async fn delete_paper(&self, project: &str, paper: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::remove(&self.0.store, project, paper).await
    }

    /// Adds a figure, table or claim to a draft version, or changes one.
    pub async fn save_paper_item(
        &self,
        project: &str,
        version: &str,
        input: PaperItemInput,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::save_item(&self.0.store, project, version, input).await
    }

    pub async fn delete_paper_item(&self, project: &str, version: &str, item: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::remove_item(&self.0.store, project, version, item).await
    }

    /// The project's results or runs to choose evidence from, and whether
    /// there are more after these.
    pub async fn paper_source_choices(
        &self,
        project: &str,
        kind: PaperSourceKind,
        query: &str,
        offset: u32,
    ) -> Result<(Vec<PaperSourceChoice>, bool)> {
        self.known_project(project).await?;
        crate::papers::choices(&self.0.store, project, kind, query, offset).await
    }

    /// Binds a result or a run to an item of a draft version.
    pub async fn bind_paper_source(
        &self,
        project: &str,
        version: &str,
        item: &str,
        kind: PaperSourceKind,
        source: &str,
        purpose: &str,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::bind(&self.0.store, project, version, item, kind, source, purpose).await
    }

    pub async fn unbind_paper_source(
        &self,
        project: &str,
        version: &str,
        binding: &str,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::unbind(&self.0.store, project, version, binding).await
    }

    /// Checks a draft version against `policy` without changing it.
    pub async fn check_paper(
        &self,
        project: &str,
        version: &str,
        policy: PaperPolicy,
    ) -> Result<PaperCheck> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::check(&self.0.store, project, version, policy, false).await
    }

    /// Checks a draft version again and, when nothing blocks it, locks it
    /// with its evidence. Later changes need a new version.
    pub async fn finalize_paper(
        &self,
        project: &str,
        version: &str,
        policy: PaperPolicy,
    ) -> Result<PaperCheck> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::check(&self.0.store, project, version, policy, true).await
    }

    /// Lets a finding of a draft version stand, with the reason why.
    pub async fn waive_paper_finding(
        &self,
        project: &str,
        version: &str,
        code: &str,
        reason: &str,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::waive(&self.0.store, project, version, code, reason).await
    }

    /// Starts a draft from `version`, with its items and evidence, and
    /// opens the paper at it.
    pub async fn new_paper_version(
        &self,
        project: &str,
        paper: &str,
        version: &str,
        label: &str,
    ) -> Result<Paper> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        let created = crate::papers::new_version(&self.0.store, project, version, label).await?;
        crate::papers::open(&self.0.store, project, paper, Some(&created)).await
    }

    /// The ACP agents that can be started from here, with whether each
    /// program is found.
    pub async fn acp_agents(&self) -> Vec<AcpAgentRow> {
        crate::acp_agents::rows(&self.0.store).await
    }

    /// Adds an agent, or changes the one with its id; returns its id.
    pub async fn save_acp_agent(&self, agent: AcpAgent) -> Result<String> {
        let _admission = self.0.admission.read().await;
        crate::acp_agents::save(&self.0.store, agent).await
    }

    pub async fn remove_acp_agent(&self, id: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        crate::acp_agents::remove(&self.0.store, id).await
    }

    /// Starts the agent, reads how it introduces itself, and stops it.
    pub async fn test_acp_agent(&self, id: &str) -> Result<AcpAgentReport> {
        crate::acp_agents::test(&self.0.store, id).await
    }

    /// Puts the ACP agent running a conversation in another of its modes.
    pub async fn set_acp_mode(
        &self,
        project: &str,
        session: &str,
        mode: &str,
    ) -> Result<AcpControls> {
        self.own_conversation(project, session).await?;
        crate::acp_turn::set_mode(&self.session(session), mode).await
    }

    /// Sets one of the options of the ACP agent running a conversation.
    pub async fn set_acp_option(
        &self,
        project: &str,
        session: &str,
        option: &str,
        pick: AcpPick,
    ) -> Result<AcpControls> {
        self.own_conversation(project, session).await?;
        crate::acp_turn::set_option(&self.session(session), option, pick).await
    }

    /// Signs in to an agent with one of the ways it offers, or says what
    /// to run in a terminal to do so.
    pub async fn sign_in_acp_agent(&self, id: &str, method: &str) -> Result<AcpSignInOutcome> {
        crate::acp_agents::sign_in(&self.0.store, id, method).await
    }

    /// Python, R and Node installed under the data directory, and how the
    /// current or last install is going.
    pub fn toolchain(&self) -> wisp_dto::ToolchainStatus {
        crate::toolchain::snapshot(
            &self.0.toolchain,
            &crate::toolchain::Layout::new(&self.0.config.data),
        )
    }

    /// Starts installing `tools` (`python`, `r`, `node`, `uv`) under the
    /// data directory and returns at once; [`Self::toolchain`] follows it.
    /// It downloads pixi and the tools on the Code requests route, and
    /// changes nothing outside the data directory.
    pub async fn install_toolchain(&self, tools: Vec<String>) -> Result<wisp_dto::ToolchainStatus> {
        let network = crate::network::load(&self.0.store).await;
        let setup = crate::toolchain::Setup::real(&self.0.config.data, network)
            .map_err(anyhow::Error::msg)?;
        self.start_toolchain(setup, tools)
    }

    pub(crate) fn start_toolchain(
        &self,
        setup: crate::toolchain::Setup,
        tools: Vec<String>,
    ) -> Result<wisp_dto::ToolchainStatus> {
        crate::toolchain::start(&self.0.toolchain, self.0.store.clone(), setup, &tools)
            .map_err(anyhow::Error::msg)
    }

    /// Stops the running install; what finished stays usable.
    pub fn cancel_toolchain_install(&self) -> bool {
        crate::toolchain::cancel(&self.0.toolchain)
    }

    /// The installed plugins, and whether `project` has each switched on;
    /// with no project, none reads as on.
    pub async fn plugins(&self, project: Option<&str>) -> Result<Vec<PluginRow>> {
        if let Some(project) = project {
            self.known_project(project).await?;
        }
        crate::plugins::list(&self.0.store, project).await
    }

    /// Installs the plugin in `folder` into this host's data directory.
    /// Nothing in the package is run.
    pub async fn install_plugin(&self, folder: &std::path::Path) -> Result<PluginRow> {
        let _admission = self.0.admission.read().await;
        crate::plugins::install(&self.0.store, &self.0.config.data, folder).await
    }

    /// Switches a plugin on or off for one project. Its skills are there
    /// from the project's next turn.
    pub async fn set_plugin_enabled(
        &self,
        project: &str,
        id: &str,
        version: &str,
        enabled: bool,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::plugins::set_enabled(&self.0.store, project, id, version, enabled).await
    }

    /// Removes an installed plugin from every project, with its files.
    pub async fn remove_plugin(&self, id: &str, version: &str) -> Result<()> {
        let _admission = self.0.admission.read().await;
        crate::plugins::remove(&self.0.store, &self.0.config.data, id, version).await
    }

    /// The key id when one was made; looking never makes one.
    pub(crate) async fn known_key_id(&self) -> Option<String> {
        self.0.store.get_setting(MODEL_KEY_ID).await.ok().flatten()
    }

    /// The decision model as the form shows it. The key is never part of it.
    pub async fn decision_model(&self) -> Result<wisp_dto::DecisionModelSettings> {
        let key_id = self.known_key_id().await;
        Ok(
            crate::decision_model::load(&self.0.store, self.0.secrets.as_ref(), key_id.as_deref())
                .await,
        )
    }

    /// Keeps the decision model form; a typed key replaces the service's
    /// stored one. It applies from each conversation's next turn.
    pub async fn save_decision_model(
        &self,
        settings: wisp_dto::DecisionModelSettings,
        api_key: Option<String>,
    ) -> Result<wisp_dto::DecisionModelSettings> {
        let key_id = self.key_id().await?;
        crate::decision_model::save(
            &self.0.store,
            self.0.secrets.as_ref(),
            &key_id,
            settings,
            api_key.as_deref(),
        )
        .await
    }

    /// Forgets the key kept for one decision model service.
    pub async fn clear_decision_model_key(
        &self,
        preset: &str,
    ) -> Result<wisp_dto::DecisionModelSettings> {
        if let Some(key_id) = self.known_key_id().await {
            crate::decision_model::forget_key(self.0.secrets.as_ref(), &key_id, preset)?;
        }
        self.decision_model().await
    }

    /// Asks the form's endpoint one fixed question. Nothing is saved.
    pub async fn test_decision_model(
        &self,
        settings: wisp_dto::DecisionModelSettings,
        api_key: Option<String>,
    ) -> Result<wisp_dto::DecisionModelProbe> {
        let key_id = self.known_key_id().await;
        crate::decision_model::test(
            &self.0.store,
            self.0.secrets.as_ref(),
            key_id.as_deref(),
            settings,
            api_key.as_deref(),
        )
        .await
    }

    /// The decision model a turn's tools are built with.
    pub(crate) async fn decision_model_in_use(&self) -> Option<wisp_llm::systemone::DecisionModel> {
        let key_id = self.known_key_id().await;
        crate::decision_model::configured(&self.0.store, self.0.secrets.as_ref(), key_id.as_deref())
            .await
    }

    /// The pet the user chose, checked again each time it is asked for.
    pub async fn pet(&self) -> Result<Pet> {
        Ok(crate::pet::read(&self.0.store).await)
    }

    /// Turns the pet on or off and keeps its folder. Turning it on with a
    /// folder that is no pet is refused and changes nothing.
    pub async fn set_pet(&self, enabled: bool, directory: &str) -> Result<Pet> {
        crate::pet::save(&self.0.store, enabled, directory).await
    }

    /// The requests kept as quick actions, in the order they were added.
    pub async fn quick_actions(&self) -> Result<Vec<QuickAction>> {
        Ok(crate::quick_actions::list(&self.0.store).await)
    }

    /// Adds a quick action, or changes the one `input.id` names.
    pub async fn save_quick_action(&self, input: QuickActionInput) -> Result<Vec<QuickAction>> {
        crate::quick_actions::upsert(&self.0.store, input).await
    }

    pub async fn delete_quick_action(&self, id: &str) -> Result<Vec<QuickAction>> {
        crate::quick_actions::remove(&self.0.store, id).await
    }

    /// The experts a conversation can be started as.
    pub async fn experts(&self) -> Result<Vec<Expert>> {
        Ok(crate::experts::list(&self.0.store).await)
    }

    /// Adds an expert, or changes the one `input.id` names. One that ships
    /// with Wisp keeps its instructions.
    pub async fn save_expert(&self, input: ExpertInput) -> Result<Vec<Expert>> {
        crate::experts::upsert(&self.0.store, input).await
    }

    /// Removes an expert of the user's own. Conversations started as it go
    /// on under the instructions they were given.
    pub async fn delete_expert(&self, id: &str) -> Result<Vec<Expert>> {
        crate::experts::remove(&self.0.store, id).await
    }

    /// Puts away the review that followed the latest turn.
    pub async fn dismiss_review(&self, project: &str, session: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        self.session(session).live.lock().unwrap().review = None;
        Ok(())
    }

    /// The model profile reviews are made with; empty is whichever model
    /// the conversation runs on.
    pub async fn review_model(&self) -> Result<String> {
        Ok(self
            .0
            .store
            .get_setting(crate::review::MODEL_SETTING)
            .await?
            .unwrap_or_default())
    }

    /// Chooses the model profile reviews are made with, or with an empty
    /// id goes back to the conversation's own model.
    pub async fn set_review_model(&self, profile: &str) -> Result<String> {
        let store = &self.0.store;
        if profile.is_empty() {
            store.delete_setting(crate::review::MODEL_SETTING).await?;
        } else {
            ensure!(
                self.0
                    .model
                    .read()
                    .unwrap()
                    .settings()
                    .profiles
                    .iter()
                    .any(|saved| saved.id == profile),
                "That model was removed; choose another"
            );
            store
                .set_setting(crate::review::MODEL_SETTING, profile)
                .await?;
        }
        self.review_model().await
    }

    /// The model profile sub-agents work on; empty is whichever model the
    /// conversation runs on. The key is the WebView desktop's.
    pub async fn subagent_model(&self) -> Result<String> {
        Ok(self
            .0
            .store
            .get_setting(SUBAGENT_MODEL)
            .await?
            .unwrap_or_default())
    }

    /// Chooses the model profile sub-agents work on, or with an empty id
    /// goes back to each conversation's own model.
    pub async fn set_subagent_model(&self, profile: &str) -> Result<String> {
        let store = &self.0.store;
        if profile.is_empty() {
            store.delete_setting(SUBAGENT_MODEL).await?;
        } else {
            ensure!(
                self.0
                    .model
                    .read()
                    .unwrap()
                    .settings()
                    .profiles
                    .iter()
                    .any(|saved| saved.id == profile),
                "That model was removed; choose another"
            );
            store.set_setting(SUBAGENT_MODEL, profile).await?;
        }
        self.subagent_model().await
    }

    /// The model a conversation's sub-agents work on: the conversation's
    /// own choice for them when it has one, else the one chosen in
    /// Settings, else `own`, the conversation's model. A choice that names
    /// a profile which is gone or cannot be used falls through to the next.
    pub(crate) async fn subagent_model_for(&self, session: &str, own: &ModelConfig) -> ModelConfig {
        let store = &self.0.store;
        let chosen = [
            store.frame_subagent_model(session).await.ok().flatten(),
            store.get_setting(SUBAGENT_MODEL).await.ok().flatten(),
        ];
        let models = self.0.model.read().unwrap();
        chosen
            .into_iter()
            .flatten()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .find_map(|id| {
                // `pick` would quietly hand back the default for an id
                // that is gone; only a profile that is there counts.
                models
                    .settings()
                    .profiles
                    .iter()
                    .any(|saved| saved.id == id)
                    .then(|| models.pick(&id).ok())
                    .flatten()
            })
            .unwrap_or_else(|| own.clone())
    }

    /// The model a review is made with: the chosen profile when it is
    /// still there and usable, otherwise `own`, the conversation's model.
    pub(crate) async fn reviewer(
        &self,
        own: wisp_llm::ProviderConfig,
    ) -> Box<dyn wisp_llm::Provider> {
        let store = &self.0.store;
        let chosen = self.review_model().await.unwrap_or_default();
        let mut model = match chosen.as_str() {
            "" => own,
            id => {
                let models = self.0.model.read().unwrap();
                // A profile removed since is not quietly replaced by the
                // default one: the conversation's own model reviews.
                match models
                    .settings()
                    .profiles
                    .iter()
                    .any(|saved| saved.id == id)
                {
                    true => models.pick(id).map(|model| model.provider).unwrap_or(own),
                    false => own,
                }
            }
        };
        // The request goes the way a turn would.
        let routes = crate::network::load(store).await;
        if let Some(route) = crate::network::route(&routes.model_proxy_url) {
            model.proxy = Some(route);
        }
        (self.0.models)(model)
    }

    /// Puts `text` into the conversation's running turn, which reads it at
    /// its next step as a message from the user. False when no turn is
    /// running or it has already stopped reading: the text is then the
    /// caller's to send as a turn of its own.
    pub async fn guide(&self, project: &str, session: &str, text: &str) -> Result<bool> {
        let text = text.trim();
        ensure!(!text.is_empty(), "Write a message first");
        self.own_conversation(project, session).await?;
        let running = self.session(session);
        let live = running.live.lock().unwrap();
        if !live.running || !live.guidable {
            return Ok(false);
        }
        let id = running.guided.fetch_add(1, Ordering::Relaxed) + 1;
        running.guidance.lock().unwrap().push((id, text.to_owned()));
        Ok(true)
    }

    /// Stops one of the conversation's interpreters, losing what it has in
    /// memory; files it wrote stay. Asked again of one that has stopped, it
    /// takes the row away, and the model's next call starts a fresh one.
    pub async fn stop_runtime(&self, project: &str, session: &str, id: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        let manager = self.session(session).runtime.lock().unwrap().clone();
        let found = manager.as_ref().and_then(|manager| {
            manager
                .list()
                .into_iter()
                .find(|info| info.runtime_id == id)
        });
        let (Some(manager), Some(info)) = (manager, found) else {
            bail!("That interpreter is no longer there");
        };
        if info.status == wisp_runtime::RuntimeStatus::Dead {
            manager.dismiss_dead(id)?;
        } else {
            manager.stop(&info.key).await;
        }
        Ok(())
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
        let started_as = crate::experts::of_conversation(&self.0.store, session)
            .await
            .map(|expert| expert.id)
            .unwrap_or_default();
        if options.expert != started_as {
            // The instructions go in with the first turn; later they would
            // not be what the conversation was held under.
            ensure!(
                !self
                    .0
                    .store
                    .load_messages(session)
                    .await?
                    .iter()
                    .any(|message| message.role != wisp_llm::Role::System),
                "An expert is chosen before the first message"
            );
            let key = crate::experts::frame_key(session);
            if options.expert.is_empty() {
                self.0.store.delete_setting(&key).await?;
            } else {
                ensure!(
                    crate::experts::get(&self.0.store, &options.expert)
                        .await
                        .is_some(),
                    "That expert was removed; choose another"
                );
                self.0.store.set_setting(&key, &options.expert).await?;
            }
        }
        // Written only when it is the conversation's own choice, so one
        // that never chose keeps following the default.
        if options.auto_review != crate::review::automatic(&self.0.store, session).await {
            self.0
                .store
                .set_setting(
                    &crate::review::setting(session),
                    &options.auto_review.to_string(),
                )
                .await?;
        }
        self.store_runs_on(session, &options.runs_on).await?;
        crate::acp_turn::choose(&self.0.store, project, session, &options.acp_agent).await?;
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
        remember: Remember,
    ) -> Result<()> {
        self.own_conversation(project, session).await?;
        let live = self.session(session);
        // What the answer is about, before anything is decided.
        let tool = {
            let live = live.live.lock().unwrap();
            let (pending, _) = live
                .approvals
                .get(approval)
                .context("Approval expired or already answered")?;
            ensure!(
                remember == Remember::No
                    || (approved && pending.message == tool_question(&pending.tool)),
                "This request can only be answered once"
            );
            pending.tool.clone()
        };
        if remember == Remember::Project {
            // Saved first: an approval that says "always" must not be lost.
            let mut tools = crate::runtime::project_grants(&self.0.store, project).await;
            if !tools.contains(&tool) {
                tools.push(tool.clone());
                tools.sort();
                self.0
                    .store
                    .set_setting(
                        &crate::runtime::project_grants_key(project),
                        &serde_json::to_string(&tools)?,
                    )
                    .await?;
            }
        }
        let mut live = live.live.lock().unwrap();
        let (_, sender) = live
            .approvals
            .remove(approval)
            .context("Approval expired or already answered")?;
        if remember != Remember::No {
            // The running turn asks no more either way.
            live.allowed_tools.insert(tool);
        }
        sender
            .send(if approved {
                wisp_tools::ConfirmDecision::Approved
            } else {
                wisp_tools::ConfirmDecision::Denied { feedback: None }
            })
            .map_err(|_| anyhow::anyhow!("Approval expired"))
    }

    /// Turns a waiting request down with a word on why, which the model
    /// reads with the refusal: how a proposed plan is sent back to be
    /// changed.
    pub async fn answer_with_feedback(
        &self,
        project: &str,
        session: &str,
        approval: &str,
        feedback: &str,
    ) -> Result<()> {
        let feedback = feedback.trim();
        ensure!(!feedback.is_empty(), "Write what should change first");
        self.own_conversation(project, session).await?;
        let live = self.session(session);
        let (_, sender) = live
            .live
            .lock()
            .unwrap()
            .approvals
            .remove(approval)
            .context("Approval expired or already answered")?;
        sender
            .send(wisp_tools::ConfirmDecision::Denied {
                feedback: Some(feedback.to_owned()),
            })
            .map_err(|_| anyhow::anyhow!("Approval expired"))
    }

    /// Tools the user allowed for good, project by project. Projects with
    /// none are left out.
    pub async fn project_grants(&self) -> Result<Vec<ProjectGrants>> {
        let mut all = Vec::new();
        for (id, name, ..) in self.0.store.list_projects().await? {
            let tools = crate::runtime::project_grants(&self.0.store, &id).await;
            if !tools.is_empty() {
                all.push(ProjectGrants {
                    project_id: id,
                    project_name: name,
                    tools,
                });
            }
        }
        Ok(all)
    }

    /// Takes back "always allow" for one tool in one project. Turns that
    /// start afterwards ask again; conversations that were also told "for
    /// this conversation" keep that until the app restarts or they are
    /// asked to forget it.
    pub async fn revoke_project_grant(&self, project: &str, tool: &str) -> Result<()> {
        self.known_project(project).await?;
        let mut tools = crate::runtime::project_grants(&self.0.store, project).await;
        let before = tools.len();
        tools.retain(|other| other != tool);
        ensure!(
            tools.len() < before,
            "That tool is not allowed here any more"
        );
        let key = crate::runtime::project_grants_key(project);
        if tools.is_empty() {
            self.0.store.delete_setting(&key).await
        } else {
            self.0
                .store
                .set_setting(&key, &serde_json::to_string(&tools)?)
                .await
        }
    }

    /// How turns run and how new conversations start.
    pub async fn turn_defaults(&self) -> Result<TurnDefaults> {
        let turns = crate::runtime::turn_settings(&self.0.config, &self.0.store).await;
        let range = wisp_core::context::AUTO_COMPACT_PERCENT_RANGE;
        Ok(TurnDefaults {
            max_iterations: turns.max_iterations,
            auto_compact: turns.auto_compact,
            auto_compact_percent: turns.auto_compact_percent,
            compact_range: (*range.start(), *range.end()),
            full_access: self
                .0
                .store
                .get_setting(DEFAULT_FULL_ACCESS)
                .await?
                .as_deref()
                == Some("1"),
            follow_ups: crate::follow_ups::enabled(&self.0.store).await,
            auto_review: crate::review::automatic_by_default(&self.0.store).await,
        })
    }

    /// Answers a question about a conversation without adding anything to
    /// it: the conversation's model reads its latest turns and `earlier`,
    /// the side exchanges so far, and has no tools. It can be asked while a
    /// turn runs, and reads what has been saved by then.
    pub async fn ask_aside(
        &self,
        project: &str,
        session: &str,
        earlier: &[AsideExchange],
        question: &str,
    ) -> Result<String> {
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        let options = self.stored_options(session).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        // The request goes the way a turn would.
        let routes = crate::network::load(store).await;
        if let Some(route) = crate::network::route(&routes.model_proxy_url) {
            model.proxy = Some(route);
        }
        let messages = store
            .load_recent_turn_preview_messages(session, crate::aside::TURNS)
            .await?;
        let provider = (self.0.models)(model);
        match crate::aside::ask(provider.as_ref(), &messages, earlier, question).await {
            Ok(answer) => Ok(answer),
            Err(error) => bail!(
                "{}",
                crate::network::explain(store, &format!("{error:#}")).await
            ),
        }
    }

    /// What the model is sent for this conversation: its saved working
    /// set, which after a compaction is shorter than the transcript and
    /// always begins with the instructions the transcript does not show.
    /// While a turn runs it is what had been saved when it was read.
    pub async fn model_view(&self, project: &str, session: &str) -> Result<ModelView> {
        self.own_conversation(project, session).await?;
        let messages = self.0.store.load_messages(session).await?;
        Ok(crate::model_view::fold(&messages))
    }

    /// Has the conversation's model review the conversation as a
    /// reviewer: where a claim is not borne out by the tool output it came
    /// from. Nothing is added to the conversation and the report is not
    /// kept. It waits for a running turn to end, because half a turn
    /// cannot be judged.
    pub async fn review_conversation(&self, project: &str, session: &str) -> Result<Review> {
        self.own_conversation(project, session).await?;
        let messages = self.whole_transcript(session).await?;
        self.review_messages(session, messages).await
    }

    /// Reviews the conversation as it stood when the turn `turn` names had
    /// ended: that turn and what led to it, without what came after. A
    /// turn that cannot be found is refused, never guessed.
    pub async fn review_turn(
        &self,
        project: &str,
        session: &str,
        turn: &TurnRef,
    ) -> Result<Review> {
        self.own_conversation(project, session).await?;
        let (_, next) = self.locate_turn(session, turn).await?;
        let mut messages = self.numbered_transcript(session).await?;
        if let Some(next) = next {
            messages.retain(|(seq, _)| *seq < next);
        }
        let messages = messages.into_iter().map(|(_, message)| message).collect();
        self.review_messages(session, messages).await
    }

    async fn review_messages(
        &self,
        session: &str,
        messages: Vec<wisp_llm::Message>,
    ) -> Result<Review> {
        ensure!(
            !self.session(session).live.lock().unwrap().running,
            "Wait for the turn to end before you ask for a review"
        );
        let store = &self.0.store;
        let options = self.stored_options(session).await?;
        let own = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let provider = self.reviewer(own).await;
        match crate::review::ask(provider.as_ref(), &messages).await {
            Ok(review) => Ok(review),
            Err(error) => bail!(
                "{}",
                crate::network::explain(store, &format!("{error:#}")).await
            ),
        }
    }

    /// Returns once nothing is being asked of a model about the turn that
    /// just ended in a conversation, neither what could come next nor a
    /// review: at once when nothing is, and after a few minutes at the
    /// latest. The questions are then in the snapshot and the review in
    /// the status.
    pub async fn after_turn_settled(&self, project: &str, session: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        let session = self.session(session);
        let settled = async {
            loop {
                let woken = session.suggested.notified();
                tokio::pin!(woken);
                // Listening before looking, so a wake in between is kept.
                woken.as_mut().enable();
                {
                    let live = session.live.lock().unwrap();
                    if !live.suggesting && !live.reviewing {
                        return;
                    }
                }
                woken.await;
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(240), settled).await;
        Ok(())
    }

    /// Puts away the questions suggested after the latest answer.
    pub async fn dismiss_follow_ups(&self, project: &str, session: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        self.session(session)
            .live
            .lock()
            .unwrap()
            .follow_ups
            .clear();
        Ok(())
    }

    /// Saves them. They apply from the next turn, and to conversations made
    /// from now on.
    pub async fn set_turn_defaults(&self, defaults: TurnDefaults) -> Result<TurnDefaults> {
        use crate::runtime::{AUTO_COMPACT, AUTO_COMPACT_PERCENT, ITERATION_RANGE, MAX_ITERATIONS};
        ensure!(
            ITERATION_RANGE.contains(&defaults.max_iterations),
            "Steps per turn are between {} and {}",
            ITERATION_RANGE.start(),
            ITERATION_RANGE.end()
        );
        let range = wisp_core::context::AUTO_COMPACT_PERCENT_RANGE;
        ensure!(
            range.contains(&defaults.auto_compact_percent),
            "Compaction starts between {}% and {}% of the context",
            range.start(),
            range.end()
        );
        let store = &self.0.store;
        store
            .set_setting(MAX_ITERATIONS, &defaults.max_iterations.to_string())
            .await?;
        store
            .set_setting(AUTO_COMPACT, &defaults.auto_compact.to_string())
            .await?;
        store
            .set_setting(
                AUTO_COMPACT_PERCENT,
                &defaults.auto_compact_percent.to_string(),
            )
            .await?;
        store
            .set_setting(crate::follow_ups::SETTING, &defaults.follow_ups.to_string())
            .await?;
        store
            .set_setting(
                crate::review::DEFAULT_SETTING,
                &defaults.auto_review.to_string(),
            )
            .await?;
        if defaults.full_access {
            store.set_setting(DEFAULT_FULL_ACCESS, "1").await?;
        } else {
            store.delete_setting(DEFAULT_FULL_ACCESS).await?;
        }
        self.turn_defaults().await
    }

    /// Starts an empty conversation the way the desktop's defaults say.
    pub async fn create_conversation(&self, project: &str) -> Result<String> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let store = &self.0.store;
        store
            .create_frame(&id, project, "main", &self.model_id(""))
            .await?;
        store
            .rename_session(&id, project, "New conversation")
            .await?;
        if self.turn_defaults().await?.full_access {
            store.set_setting(&full_access_key(&id), "1").await?;
        }
        Ok(id)
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

/// A conversation had in Codex CLI or Claude Code on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalConversation {
    /// "Codex" or "Claude Code".
    pub source: &'static str,
    pub path: String,
    pub title: String,
    /// The folder it was had in.
    pub folder: String,
    pub modified_ms: i64,
    pub size: u64,
    /// Already brought into this project.
    pub imported: bool,
}

/// What importing a conversation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    /// It had been imported; what was added since was brought in.
    Updated,
    /// It had been imported and nothing was added since.
    Unchanged,
}

/// A branch as it stands for being brought back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchMerge {
    /// The conversation it came from.
    pub source: String,
    /// Messages since it split off.
    pub new_messages: usize,
    /// What the summary is checked against before it is added.
    pub guard: String,
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
    /// Have each turn that used a tool or said a good deal reviewed when it
    /// ends. A conversation that never chose follows the default.
    pub auto_review: bool,
    /// Id of the expert the conversation is started as; empty is none. It
    /// can only be chosen before the first message.
    pub expert: String,
    /// Where this conversation's work runs unless the model is told
    /// otherwise: empty follows the default in Settings, [`RUNS_LOCALLY`]
    /// is this computer, anything else is the name of an SSH host.
    pub runs_on: String,
    /// Id of the ACP agent that runs this conversation instead of Wisp's
    /// own model; empty is Wisp. It can only be chosen before the first
    /// message.
    pub acp_agent: String,
}

/// The [`ConversationOptions::runs_on`] value for this computer.
pub const RUNS_LOCALLY: &str = "local";

/// A stored choice as [`ConversationOptions::runs_on`] spells it.
fn runs_on(choice: &wisp_runs::ssh_hosts::SessionDefaultExecutionContext) -> String {
    use wisp_runs::ssh_hosts::SessionDefaultExecutionContext as Choice;
    match choice {
        Choice::Unset => String::new(),
        Choice::Local => RUNS_LOCALLY.into(),
        // A context that is not an SSH host keeps its whole id, so a choice
        // made in the WebView desktop is carried, not lost.
        Choice::Remote(id) => id.strip_prefix("ssh:").unwrap_or(id).to_owned(),
    }
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
    /// The interpreters the conversation keeps between tool calls.
    pub runtimes: Vec<RuntimeRow>,
    /// The review that followed the latest turn, when the conversation
    /// has turns reviewed; gone when the next turn starts.
    pub review: Option<ReviewOutcome>,
    /// The mode and options of the ACP agent running the conversation,
    /// once it has started.
    pub acp: Option<AcpControls>,
}

/// An interpreter a conversation started and keeps, with what it has in
/// memory, until it is stopped or the conversation goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRow {
    /// Names this one interpreter, to stop it.
    pub id: String,
    pub language: RuntimeLanguage,
    pub state: RuntimeState,
    /// The interpreter's path and version, once it has started.
    pub interpreter: Option<String>,
    pub version: Option<String>,
    /// Memory it holds, when the system could say.
    pub memory_bytes: Option<u64>,
    /// Why it stopped, when it did not stop by itself being asked.
    pub last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeLanguage {
    Python,
    R,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeState {
    Starting,
    /// Idle, with its variables in memory.
    Ready,
    /// Running code.
    Busy,
    Stopping,
    /// Gone; what it had in memory is lost. The row stays until dismissed,
    /// so the model's next call says so instead of quietly starting anew.
    Stopped,
}

impl From<wisp_runtime::RuntimeInfo> for RuntimeRow {
    fn from(info: wisp_runtime::RuntimeInfo) -> Self {
        Self {
            id: info.runtime_id,
            language: match info.key.language {
                wisp_runtime::RuntimeLanguage::Python => RuntimeLanguage::Python,
                wisp_runtime::RuntimeLanguage::R => RuntimeLanguage::R,
            },
            state: match info.status {
                wisp_runtime::RuntimeStatus::Starting => RuntimeState::Starting,
                wisp_runtime::RuntimeStatus::Ready => RuntimeState::Ready,
                wisp_runtime::RuntimeStatus::Busy => RuntimeState::Busy,
                wisp_runtime::RuntimeStatus::Stopping => RuntimeState::Stopping,
                wisp_runtime::RuntimeStatus::Dead => RuntimeState::Stopped,
            },
            interpreter: info.interpreter,
            version: info.version,
            memory_bytes: info.resident_memory_bytes,
            last_error: info.last_error,
        }
    }
}

/// An SSH host as Settings shows and edits it. Signing in is by key or by
/// whatever the user's SSH config and agent provide; a password is never
/// part of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshHostRow {
    /// What the host is called here and in tool calls.
    pub alias: String,
    /// Its address. Empty leaves it to the SSH config entry of that alias.
    pub host_name: String,
    pub user: String,
    pub port: Option<u16>,
    /// Where the private key is. The key itself is never read or kept.
    pub identity_file: String,
    /// A jump host, as OpenSSH's ProxyJump takes it.
    pub proxy_jump: String,
    pub notes: String,
}

impl From<wisp_runs::ssh_hosts::SshHost> for SshHostRow {
    fn from(host: wisp_runs::ssh_hosts::SshHost) -> Self {
        Self {
            alias: host.alias,
            host_name: host.host_name.unwrap_or_default(),
            user: host.user.unwrap_or_default(),
            port: host.port,
            identity_file: host.identity_file.unwrap_or_default(),
            proxy_jump: host.proxy_jump.unwrap_or_default(),
            notes: host.notes.unwrap_or_default(),
        }
    }
}

impl SshHostRow {
    fn into_host(self) -> Result<wisp_runs::ssh_hosts::SshHost> {
        let filled = |text: String| Some(text.trim().to_owned()).filter(|text| !text.is_empty());
        let alias = self.alias.trim().to_owned();
        ensure!(!alias.is_empty(), "Give the host a name");
        ensure!(
            self.notes.chars().count() <= 500,
            "Notes are limited to 500 characters"
        );
        let host = wisp_runs::ssh_hosts::SshHost {
            alias,
            host_name: filled(self.host_name),
            user: filled(self.user),
            port: self.port,
            identity_file: filled(self.identity_file),
            proxy_jump: filled(self.proxy_jump),
            notes: filled(self.notes),
            auth_method: None,
            has_password: false,
            password: None,
        };
        // The same rules a connection is held to, before anything is saved.
        wisp_runs::ssh_hosts::SshConnection::from_host(&host).map_err(anyhow::Error::msg)?;
        Ok(host)
    }
}

/// What Wisp can draw on in a project. Each pair is how many are on, or
/// usable, out of how many there are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub skills: (usize, usize),
    pub connectors: (usize, usize),
    pub mcp_servers: (usize, usize),
    /// SSH hosts whose last check passed, out of those saved.
    pub hosts: (usize, usize),
    pub memory_on: bool,
    pub memory_notes: usize,
    pub habits: usize,
    pub tools: Vec<LocalTool>,
}

/// A command-line tool and where it was found, if it was.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalTool {
    pub name: String,
    pub what: String,
    pub path: Option<String>,
}

/// What Project settings shows and saves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectSettings {
    pub name: String,
    pub description: String,
    /// What the model is told to follow in this project, in the user's own
    /// words. Kept in the project folder as `.wisp/WISP.md`.
    pub rules: String,
}

pub const PROJECT_NAME_LIMIT: usize = 120;
pub const PROJECT_DESCRIPTION_LIMIT: usize = 2_000;
pub const PROJECT_RULES_LIMIT: usize = 20_000;

fn rules_file(root: &str) -> std::path::PathBuf {
    std::path::Path::new(root).join(".wisp").join("WISP.md")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentsView {
    pub hosts: Vec<SshHostRow>,
    /// Alias of the host new work runs on; `None` is this computer.
    pub default: Option<String>,
    /// Hosts named in `~/.ssh/config` that are not added yet.
    pub config_aliases: Vec<String>,
    /// What the last check of each host found; a host that was never
    /// checked has none, and work cannot run on it yet.
    pub checks: Vec<EnvironmentCheck>,
}

pub use wisp_runs::context_probe::ProbeResult;

/// The last check of an SSH host: what it is, or why it cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentCheck {
    pub alias: String,
    /// When it was checked, in seconds since 1970.
    pub at: i64,
    /// Why the host could not be used. Work is refused until a check passes.
    pub error: Option<String>,
    /// What answered, when something did.
    pub found: Option<ProbeResult>,
}

impl EnvironmentCheck {
    fn of(alias: &str, context: &wisp_store::ExecutionContext) -> Option<Self> {
        let at = context.last_probe_at?;
        let passed = context.last_probe_status.as_deref() == Some("ok");
        Some(Self {
            alias: alias.to_owned(),
            at,
            error: (!passed).then(|| {
                context
                    .last_probe_error
                    .clone()
                    .filter(|error| !error.trim().is_empty())
                    .unwrap_or_else(|| "The check failed".into())
            }),
            found: passed
                .then(|| serde_json::from_str(&context.capabilities_json).ok())
                .flatten(),
        })
    }
}

pub use crate::mcp::{McpServer, McpTransport};

/// An MCP server as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerRow {
    pub server: McpServer,
    /// A token is saved for it. The token itself is never handed back.
    pub has_token: bool,
    /// Why it gave no tools the last time it was asked.
    pub error: Option<String>,
}

/// What Settings sends to add or change a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerInput {
    /// The server to change; `None` adds one.
    pub id: Option<String>,
    pub name: String,
    pub transport: McpTransport,
    /// A bearer token for an HTTP server: `Some("")` forgets the saved
    /// one, `None` leaves it as it is.
    pub token: Option<String>,
}

/// A service key the bundled connectors can use.
struct ServiceKey {
    id: &'static str,
    /// The name the service's documentation gives it.
    env: &'static str,
    label: &'static str,
    note: &'static str,
    /// Shown masked while it is typed.
    secret: bool,
}

const SERVICE_KEYS: &[ServiceKey] = &[
    ServiceKey {
        id: "ncbi_api_key",
        env: "NCBI_API_KEY",
        label: "NCBI API key",
        note: "PubMed and the other NCBI sources allow more requests per second with a key.",
        secret: true,
    },
    ServiceKey {
        id: "ncbi_email",
        env: "NCBI_EMAIL",
        label: "NCBI contact email",
        note: "NCBI asks automated clients to say who they are. It is sent with each request.",
        secret: false,
    },
    ServiceKey {
        id: "openalex_api_key",
        env: "OPENALEX_API_KEY",
        label: "OpenAlex API key",
        note: "Literature search through OpenAlex; without a key its shared limits apply.",
        secret: true,
    },
];

/// A service key as Settings lists it. Its value is never part of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialRow {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    pub secret: bool,
    pub saved: bool,
}

/// Tokens one project's conversations used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectUsage {
    pub project_id: String,
    pub name: String,
    /// Conversations that used any.
    pub conversations: u64,
    /// Includes `cached`.
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cached: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageView {
    /// Most recently active first.
    pub projects: Vec<ProjectUsage>,
    /// `(model, input + output)`, largest first.
    pub models: Vec<(String, u64)>,
    /// `(YYYY-MM-DD, input + output)` for each day up to today, oldest first.
    pub days: Vec<(String, u64)>,
}

/// What Wisp keeps for one project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectStorage {
    pub project_id: String,
    pub name: String,
    pub workspace_dir: String,
    pub conversations: usize,
    /// Size of the project's `.wisp` folder: memory, snapshots, sessions.
    pub wisp_bytes: u64,
    /// Size of the files attached from outside the project.
    pub uploads_bytes: u64,
    /// The folder is not there (an unplugged disk, a moved folder).
    pub missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageView {
    pub data_dir: String,
    /// Conversations, settings and run records of every project.
    pub database_bytes: u64,
    pub favorites_bytes: u64,
    pub projects: Vec<ProjectStorage>,
}

/// Entries one folder measurement may look at.
const SIZE_BUDGET: usize = 50_000;

/// The size of the regular files under `root`, links not followed. A very
/// large folder is measured up to a bound rather than for minutes.
fn folder_size(root: &std::path::Path) -> u64 {
    let mut total = 0;
    let mut seen = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SIZE_BUDGET {
                return total;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata().map(|meta| meta.len()).unwrap_or_default();
            }
        }
    }
    total
}

/// A bundled connector as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectorRow {
    /// Its name and the id its switch is saved under, such as `pubmed`.
    pub domain: String,
    pub description: String,
    /// Where it is documented: `(label, URL)`.
    pub links: Vec<(String, String)>,
    /// Names of the tools it gives the model.
    pub tools: Vec<String>,
    pub enabled: bool,
}

/// A skill as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRow {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// Where it was found: `bundled` with the app, `project` in the open
    /// project, `extra` in a directory the host was told about.
    pub source: &'static str,
    pub enabled: bool,
}

/// Whether an approval is remembered, and for how long.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Remember {
    /// This once.
    #[default]
    No,
    /// Until the app restarts or the conversation is told to ask again.
    Conversation,
    /// For every conversation of the project, until it is taken back.
    Project,
}

/// Tools one project runs without asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectGrants {
    pub project_id: String,
    pub project_name: String,
    /// Sorted.
    pub tools: Vec<String>,
}

pub use crate::acp_agents::{
    AcpAgent, AcpAgentReport, AcpAgentRow, AcpSignIn, AcpSignInKind, AcpSignInOutcome,
};
pub use crate::acp_turn::{AcpChoice, AcpControls, AcpPick, AcpSetting, AcpValue};
pub use crate::aside::Exchange as AsideExchange;
pub use crate::browser::{BrowserSettings, BrowserStatus};
pub use crate::browser_filters::{BrowserFilters, BrowserHostRule};
pub use crate::compact::Compacted;
pub use crate::experts::{Expert, ExpertInput};
pub use crate::model_view::{ModelRowKind, ModelView, ModelViewRow};
pub use crate::origin::FileOrigin;
pub use crate::papers::{
    Paper, PaperAudience, PaperCheck, PaperFinding, PaperInput, PaperItem, PaperItemInput,
    PaperItemKind, PaperLevel, PaperPolicy, PaperRow, PaperSource, PaperSourceChoice,
    PaperSourceKind, PaperVersion, PaperVersionState,
};
pub use crate::pet::{Pet, PetLook, SHEET_HEIGHT, SHEET_WIDTH};
pub use crate::plugins::PluginRow;
pub use crate::quick_actions::{QuickAction, QuickActionInput};
pub use crate::remote_access::{RemoteAccess, RemoteState};
pub use crate::remote_dir::RemoteDirectory;
pub use crate::research_archive::ArchiveDraft;
pub use crate::review::{Finding as ReviewFinding, Outcome as ReviewOutcome, Review};
pub use crate::run_log::RunLog;
pub use crate::schedules::{ScheduleInput, ScheduleRow, ScheduleRun, MIN_INTERVAL_SECS};
pub use crate::turn_undo::TurnFiles;
pub use crate::updates::UpdateCheck;
pub use crate::workflows::{
    Workflow, WorkflowInput, WorkflowTask, TASK_LIMIT as WORKFLOW_TASK_LIMIT,
};
pub use wisp_dto::{ArchiveScript, ResearchArchive};

/// A project's folder, as the question before it is deleted describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFolder {
    /// The project's name, which has to be typed to delete the folder.
    pub name: String,
    /// The folder's real path; empty when there is none any more.
    pub path: String,
    pub contents: FolderContents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderContents {
    /// The folder is already gone: only the project's records are removed.
    Missing,
    /// The registered path is a link: the link is removed, and the folder
    /// it points at is kept.
    Link,
    /// What is in the folder. `more` says the count stopped early.
    Files { files: u64, bytes: u64, more: bool },
}

/// Entries counted before the question is asked with "at least".
const FOLDER_COUNT_LIMIT: u64 = 200_000;

/// How turns run and how new conversations start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnDefaults {
    /// Model calls one turn may make.
    pub max_iterations: usize,
    pub auto_compact: bool,
    /// How full the context is, in percent, when older turns are summarised.
    pub auto_compact_percent: u8,
    /// The lowest and highest percent that can be chosen.
    pub compact_range: (u8, u8),
    /// New conversations run tools that change things without asking.
    pub full_access: bool,
    /// After an answer, ask the model for questions that could come next.
    pub follow_ups: bool,
    /// Conversations that have not chosen have their turns reviewed.
    pub auto_review: bool,
}

const DEFAULT_FULL_ACCESS: &str = "desktop.conversation.default_full_access";
/// The WebView desktop's key for the model sub-agents work on.
const SUBAGENT_MODEL: &str = "subagent_model_id";

/// A saved item in full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub item: LibraryItem,
    /// The picture's bytes when the item is a figure.
    pub image: Option<Vec<u8>>,
}

/// Items one listing of the favorites returns.
const FAVORITES_LIMIT: usize = 500;
/// The WebView desktop's limits for the same library.
const FAVORITE_TEXT_LIMIT: usize = 2 * 1024 * 1024;
const FIGURE_LIMIT: u64 = 32 * 1024 * 1024;

/// A saved item is called by its first line.
fn favorite_title(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Untitled");
    let mut title: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        title.push('…');
    }
    title
}

fn image_type(path: &std::path::Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

/// One day's notes in a project's memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryNote {
    /// File name, `YYYY-MM-DD.md` for notes Wisp wrote.
    pub name: String,
    /// The first characters of the note.
    pub preview: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryView {
    /// Off, the model neither searches notes nor is shown habits.
    pub enabled: bool,
    /// The note new entries go into today.
    pub today: String,
    /// Newest first.
    pub notes: Vec<MemoryNote>,
    /// Newest first.
    pub habits: Vec<GlobalMemory>,
}

/// Characters one remembered entry may hold, as in the WebView desktop.
const MEMORY_LIMIT: usize = 4_000;
const NOTE_LIMIT: usize = 1_000_000;

fn confirmed_memory(text: &str) -> Result<String> {
    let text = text.trim();
    ensure!(!text.is_empty(), "There is nothing to remember");
    ensure!(
        text.chars().count() <= MEMORY_LIMIT,
        "One memory holds at most {MEMORY_LIMIT} characters"
    );
    Ok(text.to_owned())
}

/// A note is a Markdown file directly inside the project's memory folder.
fn memory_note_name(name: &str) -> Result<&str> {
    let name = name.trim();
    ensure!(
        !name.is_empty()
            && !name.starts_with('.')
            && !name.contains(['/', '\\'])
            && !name.contains("..")
            && name.len() <= 120,
        "That is not a note's name"
    );
    ensure!(
        std::path::Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("md"),
        "Notes are Markdown files (.md)"
    );
    Ok(name)
}

fn today_note() -> String {
    wisp_core::MemoryManager::today_file_name()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
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
