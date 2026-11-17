//! Operations for the desktop process that embeds this host: conversation
//! management and the model it runs on. They are typed methods rather than
//! dispatch commands, so nothing here can be reached through the remote
//! allowlist.
pub use crate::files::FileSave;
pub use crate::output::{tool_question, TokenUsage};
use crate::{config::ModelConfig, Server};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::Ordering;
pub use wisp_dto::native_conversations::ReferenceOption;
use wisp_dto::SessionSearchInfo;
pub use wisp_dto::{ProjectMemo, ProjectMemoPatch};
use wisp_llm::ProviderKind;
pub use wisp_store::GlobalMemory;
use wisp_store::Store;
pub use wisp_store::{LibraryItem, LibraryItemSummary, LibraryItemVersion};
/// What both hosts answer when a branch is branched again, or a frozen one
/// is handed off, reviewed or moved away; a client saying so before it asks
/// uses the same words.
pub use wisp_store::{BRANCH_AGAIN_REFUSAL, FROZEN_BRANCH_CHANGE_REFUSAL};

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
    /// This model's own route, as the WebView's `proxy_url`: empty follows
    /// the Network page, `none` connects directly, else a proxy's address.
    pub proxy: String,
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
    /// Whether the model reads pictures, by the model catalog.
    pub sees: bool,
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
    let route = crate::network::own_route("Route", &fields.proxy)?;
    let mut model = resolve_fields(fields, key)?;
    model.provider.proxy = crate::network::route(&route);
    Ok(model)
}

fn resolve_fields(fields: &ModelFields, key: &str) -> Result<ModelConfig> {
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
            proxy: String::new(),
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
                // A subscription's key is its sign-in, shared by its models.
                let subscription =
                    crate::subscriptions::Subscription::of_provider(&profile.fields.provider);
                let key = match subscription {
                    Some(subscription) => crate::subscriptions::load(secrets, subscription)
                        .map(|creds| creds.access().to_owned()),
                    None => key_id
                        .as_deref()
                        .and_then(|id| secrets.get(&key_name(id, &profile.id))),
                };
                let model = match (&key, subscription) {
                    (Some(key), _) => resolve(&profile.fields, key).map_err(|e| format!("{e:#}")),
                    (None, Some(subscription)) => Err(format!(
                        "Not signed in to {}; sign in again",
                        crate::subscriptions::name(subscription)
                    )),
                    (None, None) => Err("The saved API key is missing; enter it again".into()),
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

    pub(crate) fn settings(&self) -> ModelSettings {
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
                    sees: loaded
                        .model
                        .as_ref()
                        .is_ok_and(|model| model.supports_vision),
                })
                .collect(),
        }
    }

    /// The saved model that describes pictures for a model that cannot see
    /// them: `chosen` while it is saved, usable and can see, else the first
    /// saved model that can. None when none can.
    pub(crate) fn picture_reader(&self, chosen: &str) -> Option<ModelConfig> {
        let sees = |loaded: &&Loaded| {
            loaded
                .model
                .as_ref()
                .is_ok_and(|model| model.supports_vision)
        };
        let mut seeing = self.profiles.iter().filter(sees);
        let first = seeing.clone().next();
        seeing
            .find(|loaded| loaded.profile.id == chosen)
            .or(first)
            .and_then(|loaded| loaded.model.clone().ok())
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
    /// Subagent conversation id to the id of the conversation that started
    /// it; it is watched, not written to.
    pub dispatched: HashMap<String, String>,
    /// Whether the list was cut off: there are older conversations (or, for
    /// a search, more matches) than the pages asked for, and asking for one
    /// page more lists them.
    pub more: bool,
}

/// How many conversations the sidebar is given at a time. Asking for more
/// pages lists the next ones under them.
pub const CONVERSATION_PAGE: usize = 100;

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

    pub(crate) fn editable_models(&self) -> Result<()> {
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
        // A subscription model runs on its sign-in, never on a typed key.
        if let Some(subscription) =
            crate::subscriptions::Subscription::of_provider(&input.fields.provider)
        {
            let creds = crate::subscriptions::load(self.0.secrets.as_ref(), subscription)
                .with_context(|| {
                    format!(
                        "Sign in to {} first",
                        crate::subscriptions::name(subscription)
                    )
                })?;
            return Ok((resolve(&input.fields, creds.access())?, None));
        }
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
            fields: ModelFields {
                // Saved as it is used: trimmed, `none` or an address.
                proxy: input.fields.proxy.trim().to_owned(),
                ..input.fields
            },
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

    /// Adds saved models brought from elsewhere, each with its key when it
    /// has one; one whose id is already here is left alone. While there is
    /// no default, `preferred` becomes it when it came, else the first that
    /// came. Returns the ids added.
    pub(crate) async fn adopt_model_profiles(
        &self,
        profiles: Vec<(String, String, ModelFields, Option<String>)>,
        preferred: Option<&str>,
    ) -> Result<Vec<String>> {
        self.editable_models()?;
        let mut saved = saved_models(&self.0.store).await?;
        let mut added = Vec::new();
        for (id, name, fields, key) in profiles {
            if id.trim().is_empty() || saved.profiles.iter().any(|profile| profile.id == id) {
                continue;
            }
            if let Some(key) = key.filter(|key| !key.trim().is_empty()) {
                self.0
                    .secrets
                    .set(&key_name(&self.key_id().await?, &id), key.trim())?;
            }
            let name = match name.trim() {
                "" => fields.model.trim(),
                name => name,
            };
            saved.profiles.push(SavedProfile {
                id: id.clone(),
                name: name.chars().take(NAME_LIMIT).collect(),
                fields,
            });
            added.push(id);
        }
        let Some(first) = added.first() else {
            return Ok(added);
        };
        if !saved
            .profiles
            .iter()
            .any(|profile| profile.id == saved.default)
        {
            saved.default = preferred
                .filter(|preferred| added.iter().any(|id| id == preferred))
                .unwrap_or(first)
                .to_owned();
        }
        self.store_models(&saved).await?;
        Ok(added)
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
                proxy: String::new(),
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

    /// Each subscription, and the account signed in to it if any.
    pub fn subscriptions(&self) -> Vec<SubscriptionRow> {
        crate::subscriptions::Subscription::ALL
            .into_iter()
            .map(|subscription| SubscriptionRow {
                subscription,
                account: crate::subscriptions::load(self.0.secrets.as_ref(), subscription)
                    .map(|creds| creds.account()),
            })
            .collect()
    }

    /// A client for sign-in and token refresh, on the Network page's
    /// subscription route.
    async fn subscription_client(&self) -> Result<reqwest::Client> {
        let routes = crate::network::load(&self.0.store).await;
        let mut client =
            reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30));
        match routes.subscription_proxy_url.trim() {
            "" => {}
            "none" => client = client.no_proxy(),
            proxy => client = client.proxy(reqwest::Proxy::all(proxy)?),
        }
        Ok(client.build()?)
    }

    /// Starts signing in to a subscription, in this computer's browser or
    /// with a code on another device.
    pub async fn start_subscription_sign_in(
        &self,
        subscription: Subscription,
        device: bool,
    ) -> Result<SignInChallenge> {
        self.editable_models()?;
        let client = self.subscription_client().await?;
        self.0
            .sign_ins
            .start(subscription, device, client, self.0.secrets.as_ref())
            .await
    }

    pub fn subscription_sign_in(&self, id: &str) -> Result<SignInState> {
        self.0.sign_ins.state(id)
    }

    /// Finishes a browser sign-in with the address the browser ended on.
    pub async fn submit_subscription_redirect(
        &self,
        id: &str,
        redirect: &str,
    ) -> Result<SignInState> {
        self.0.sign_ins.redirect(id, redirect).await
    }

    pub fn cancel_subscription_sign_in(&self, id: &str) {
        self.0.sign_ins.cancel(id)
    }

    /// Keeps a finished sign-in in the keychain and adds a model of the
    /// subscription when there is none yet; the first model becomes the
    /// default.
    pub async fn save_subscription_sign_in(&self, id: &str) -> Result<ModelSettings> {
        self.editable_models()?;
        let creds = self.0.sign_ins.take(id)?;
        crate::subscriptions::store(self.0.secrets.as_ref(), &creds)?;
        let subscription = creds.subscription();
        let mut saved = saved_models(&self.0.store).await?;
        if !saved
            .profiles
            .iter()
            .any(|profile| profile.fields.provider == subscription.provider())
        {
            let id = uuid::Uuid::new_v4().to_string();
            saved.profiles.push(SavedProfile {
                id: id.clone(),
                name: format!(
                    "{} · {}",
                    subscription.default_model(),
                    crate::subscriptions::name(subscription)
                ),
                fields: ModelFields {
                    provider: subscription.provider().into(),
                    api_url: subscription.default_url().into(),
                    model: subscription.default_model().into(),
                    max_context: subscription.context_window().to_string(),
                    max_tokens: "32000".into(),
                    reasoning_effort: String::new(),
                    proxy: String::new(),
                },
            });
            if !saved.profiles.iter().any(|p| p.id == saved.default) {
                saved.default = id;
            }
        }
        self.store_models(&saved).await
    }

    /// Forgets a subscription's sign-in; its models stay listed, waiting
    /// for the next one.
    pub async fn sign_out_subscription(&self, subscription: Subscription) -> Result<ModelSettings> {
        self.editable_models()?;
        crate::subscriptions::forget(self.0.secrets.as_ref(), subscription)?;
        let saved = saved_models(&self.0.store).await?;
        self.store_models(&saved).await
    }

    /// The ChatGPT (Codex) accounts kept on this computer, one active.
    pub fn codex_accounts(&self) -> Result<Vec<CodexAccount>> {
        crate::subscriptions::codex_accounts(self.0.secrets.as_ref())
    }

    /// Runs ChatGPT (Codex) models on another kept account.
    pub async fn switch_codex_account(&self, id: &str) -> Result<ModelSettings> {
        crate::subscriptions::switch_codex_account(self.0.secrets.as_ref(), id)?;
        let saved = saved_models(&self.0.store).await?;
        self.store_models(&saved).await
    }

    pub async fn remove_codex_account(&self, id: &str) -> Result<ModelSettings> {
        crate::subscriptions::remove_codex_account(self.0.secrets.as_ref(), id)?;
        let saved = saved_models(&self.0.store).await?;
        self.store_models(&saved).await
    }

    /// How much of an account's allowance is used; asks ChatGPT.
    pub async fn codex_account_usage(&self, id: &str) -> Result<CodexUsage> {
        let client = self.subscription_client().await?;
        crate::subscriptions::codex_usage(self.0.secrets.as_ref(), &client, id).await
    }

    /// Brings the sign-ins of Codex CLI and CLIProxyAPI on this computer
    /// into the pool; returns how many accounts were found.
    pub async fn import_codex_accounts(&self) -> Result<usize> {
        let home = crate::session_import::home().context("Could not find the home folder")?;
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from);
        let found = crate::subscriptions::import_codex_accounts(
            self.0.secrets.as_ref(),
            &home,
            codex_home,
        )?;
        let saved = saved_models(&self.0.store).await?;
        self.store_models(&saved).await?;
        Ok(found)
    }

    /// A subscription model's token, refreshed when it is about to expire.
    pub(crate) async fn fresh_subscription(
        &self,
        model: &mut ModelConfig,
        provider: &str,
    ) -> Result<()> {
        if let Some(subscription) = crate::subscriptions::Subscription::of_provider(provider) {
            let client = self.subscription_client().await?;
            model.provider.api_key =
                crate::subscriptions::fresh_access(self.0.secrets.as_ref(), subscription, &client)
                    .await?;
        }
        Ok(())
    }

    /// The provider name a conversation's model profile carries.
    pub(crate) fn provider_of(&self, profile: &str) -> String {
        let state = self.0.model.read().unwrap();
        state
            .profile(profile)
            .or_else(|| state.default_profile())
            .map(|loaded| loaded.profile.fields.provider.clone())
            .unwrap_or_default()
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
        crate::network::route_model(&mut model.provider.proxy, &routes);
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

    /// How the last turn of each conversation this host ran since it
    /// started ended, for the ones not running now: `true` when it stopped
    /// on an error. One it has not run a turn in is not listed.
    pub fn finished_turns(&self) -> HashMap<String, bool> {
        self.0
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, session)| {
                let live = session.live.lock().unwrap();
                (!live.running && live.generation > 0).then(|| (id.clone(), live.error.is_some()))
            })
            .collect()
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

    /// Cuts `region` (fractions of the picture) out of a picture in the
    /// project and keeps it as a PNG named `name` in the project's
    /// `uploads`, for a message to carry: what the WebView's "Add to chat"
    /// on a region does in the browser (`crop_region_to_upload` in
    /// `ui/src/api.js`), the picture's own pixels whatever zoom it was
    /// marked at. An SVG is refused: there is no renderer for it here.
    pub async fn import_image_region(
        &self,
        project: &str,
        path: &str,
        region: wisp_dto::image_pins::ImageRegion,
        name: &str,
    ) -> Result<String> {
        let (kind, bytes) = self.project_image(project, path).await?;
        ensure!(
            kind != "image/svg+xml",
            "A region of an SVG picture cannot be cut out here; add a note to the region instead"
        );
        let png = tokio::task::spawn_blocking(move || crop_png(&bytes, region)).await??;
        self.import_pasted(project, name, &png).await
    }

    /// A PDF of the project, whole, for the window to draw its pages; the
    /// WebView shows them with pdf.js.
    pub async fn project_pdf(&self, project: &str, path: &str) -> Result<Vec<u8>> {
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let real = crate::files::resolve(&root, path).await?;
        ensure!(
            real.extension()
                .and_then(|kind| kind.to_str())
                .is_some_and(|kind| kind.eq_ignore_ascii_case("pdf")),
            "That file is not a PDF"
        );
        let meta = tokio::fs::metadata(&real).await?;
        ensure!(meta.is_file(), "That is not a file");
        ensure!(
            meta.len() <= FIGURE_LIMIT,
            "PDFs of more than 32 MB are not drawn here"
        );
        Ok(tokio::fs::read(&real).await?)
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

    /// Copies files chosen elsewhere into the project folder `folder`
    /// (empty is the project's own), as the WebView's Upload does: never
    /// over a file that is there, and each with its own outcome.
    pub async fn upload_files(
        &self,
        project: &str,
        folder: &str,
        sources: Vec<std::path::PathBuf>,
    ) -> Result<Vec<wisp_dto::LocalFileUploadResult>> {
        let _admission = self.0.admission.read().await;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let folder = folder.to_owned();
        tokio::task::spawn_blocking(move || crate::files::upload(&root, &folder, &sources)).await?
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

    /// Saves the user's edit of an existing project file, as the WebView's
    /// source editor does (`save_file`): with `original`, only while the
    /// file still holds that text, so an edit left open across a turn that
    /// rewrote the file does not silently replace that turn's work. A
    /// project frozen while one of its explorations runs takes no edits,
    /// as no other write.
    pub async fn save_project_file(
        &self,
        project: &str,
        path: &str,
        original: Option<&str>,
        text: &str,
    ) -> Result<FileSave> {
        let _admission = self.0.admission.read().await;
        let root = self.project_root(project).await?;
        crate::explorations::service::require_writable_scope(
            &self.0.store,
            &wisp_store::StateScope::mainline(project.to_owned()),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let saved = crate::files::save(std::path::Path::new(&root), path, original, text).await?;
        if saved == FileSave::Saved {
            self.0
                .store
                .bump_state_generation(&wisp_store::StateScope::mainline(project.to_owned()))
                .await?;
        }
        Ok(saved)
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

    pub(crate) async fn known_project(&self, project: &str) -> Result<()> {
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
        self.export_project_as(project, destination, false).await
    }

    /// Exports a project as a folder package, the WebView's other kind of
    /// export: a new folder under `parent`, named after the project, that
    /// another computer opens in place. Returns the folder.
    pub async fn export_project_folder(
        &self,
        project: &str,
        parent: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        ensure!(
            parent.is_absolute(),
            "Choose the folder to put the package in"
        );
        ensure!(parent.is_dir(), "Choose the folder to put the package in");
        let (name, _, _) = self
            .0
            .store
            .get_project_meta(project)
            .await?
            .context("Project not found")?;
        let package = crate::project_archive::unique_destination(parent, &name)
            .map_err(anyhow::Error::msg)?;
        self.export_project_as(project, &package, true).await?;
        Ok(package)
    }

    async fn export_project_as(
        &self,
        project: &str,
        destination: &std::path::Path,
        folder: bool,
    ) -> Result<()> {
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
        self.0.archive.clear();
        let written = crate::project_archive::export_project_contents(
            &self.0.store,
            &self.0.config.data,
            project,
            destination,
            folder,
            Some(self.0.archive.clone()),
        )
        .await;
        self.0.archive.clear();
        written.map_err(anyhow::Error::msg)
    }

    /// How far the project export or import under way has got; `None`
    /// when none is.
    pub fn archive_progress(&self) -> Option<ArchiveProgress> {
        self.0.archive.latest()
    }

    /// Opens a project folder package where it is: one exported as a
    /// folder, or a project folder that keeps its own database. The reply
    /// is the project's id; one that is already here is refused.
    pub async fn import_project_folder(&self, package: &std::path::Path) -> Result<String> {
        let _admission = self.0.admission.write().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        let package = tokio::fs::canonicalize(package)
            .await
            .context("Choose the project's folder")?;
        ensure!(package.is_dir(), "Choose the project's folder");
        ensure!(
            !package.starts_with(&self.0.config.data),
            "Keep project files separate from the host data directory"
        );
        self.0.archive.clear();
        let opened = crate::project_archive::import_project_folder(
            &self.0.store,
            &self.0.config.data,
            &package,
            Some(&self.0.archive),
        )
        .await;
        self.0.archive.clear();
        opened.map_err(anyhow::Error::msg)
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
        self.0.archive.clear();
        let imported = crate::project_archive::import_archived_project(
            &self.0.store,
            &self.0.config.data,
            archive,
            &parent,
            Some(&self.0.archive),
        )
        .await;
        self.0.archive.clear();
        imported.map_err(anyhow::Error::msg)
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

    /// The example conversations that ship with Wisp, in the order of the
    /// research they record. None when the app has no `seed/` folder.
    pub async fn demos(&self) -> Result<Vec<DemoInfo>> {
        Ok(
            tokio::task::spawn_blocking(|| crate::demos::list(wisp_paths::seed_dir().as_deref()))
                .await?,
        )
    }

    /// One example conversation, to be read as it was recorded.
    pub async fn demo(&self, id: &str) -> Result<Demo> {
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let dir = wisp_paths::seed_dir().context("This app has no example conversations")?;
            crate::demos::load(&dir, &id).map(|manifest| manifest.demo)
        })
        .await?
    }

    /// Copies an example into `project` as a conversation of its own, to be
    /// continued there, with the files it refers to; files already in the
    /// project are left as they are. Returns the new conversation.
    pub async fn copy_demo(&self, id: &str, project: &str) -> Result<String> {
        let _admission = self.0.admission.read().await;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let id = id.to_owned();
        let (title, messages) = tokio::task::spawn_blocking(move || {
            let dir = wisp_paths::seed_dir().context("This app has no example conversations")?;
            let manifest = crate::demos::load(&dir, &id)?;
            let messages = crate::demos::messages(&manifest.demo.rows);
            ensure!(
                !messages.is_empty(),
                "The example {id} has no conversation to copy"
            );
            crate::demos::write_files(&dir, &id, &root, &manifest.files)?;
            anyhow::Ok((manifest.demo.title, messages))
        })
        .await??;
        let store = &self.0.store;
        let frame = uuid::Uuid::new_v4().to_string();
        store
            .create_frame(&frame, project, "main", &self.model_id(""))
            .await?;
        store.replace_messages(&frame, &messages).await?;
        if !title.trim().is_empty() {
            store.rename_session(&frame, project, title.trim()).await?;
        }
        store.mark_frame_seen(&frame).await?;
        Ok(frame)
    }

    /// Where the local environment's executables are: the path saved for
    /// each, as the WebView's Environments page keeps it, and where it is
    /// found on the search path otherwise.
    pub async fn tool_paths(&self) -> Result<Vec<ToolPath>> {
        let saved: serde_json::Map<String, serde_json::Value> = self
            .0
            .store
            .get_execution_context("local")
            .await?
            .and_then(|context| serde_json::from_str(&context.config_json).ok())
            .unwrap_or_default();
        Ok(tokio::task::spawn_blocking(move || {
            let (path, extensions) = crate::tools_found::search_path();
            crate::tools_found::PATHS
                .iter()
                .map(|(key, label, program)| ToolPath {
                    key,
                    label,
                    saved: saved
                        .get(*key)
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    found: crate::tools_found::find(program, &path, &extensions, |candidate| {
                        candidate.is_file()
                    })
                    .map(|found| found.to_string_lossy().into_owned()),
                })
                .collect()
        })
        .await?)
    }

    /// Saves the paths as typed: an empty one goes back to looking on the
    /// search path. A path that names no file is refused; nothing is run.
    pub async fn set_tool_paths(&self, paths: BTreeMap<String, String>) -> Result<Vec<ToolPath>> {
        for (key, value) in &paths {
            let label = crate::tools_found::PATHS
                .iter()
                .find(|(known, ..)| known == key)
                .map(|(_, label, _)| *label)
                .with_context(|| format!("There is no tool called {key}"))?;
            let value = value.trim();
            ensure!(
                value.is_empty() || std::path::Path::new(value).is_file(),
                "{label}: there is no file at {value}"
            );
        }
        let paths = paths
            .into_iter()
            .map(|(key, value)| (key, value.trim().to_owned()))
            .collect();
        self.0.store.save_local_environment_paths(&paths).await?;
        self.tool_paths().await
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

    /// What was said in a conversation, for a page to share: your messages
    /// and its answers, oldest first, without tool calls. Read from what is
    /// saved, as the export is.
    pub async fn share_rows(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Vec<wisp_dto::native_conversations::ShareRow>> {
        self.own_conversation(project, session).await?;
        let messages = self.whole_transcript(session).await?;
        Ok(crate::export::share_rows(&crate::runtime::items(&messages)))
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

    /// What each saved turn of a conversation used, for the line under its
    /// answer; turns that made no model call are left out.
    pub async fn turn_tokens(&self, project: &str, session: &str) -> Result<Vec<TurnTokens>> {
        let trajectory = self.trajectory(project, session).await?;
        Ok(crate::trajectory::turn_tokens(&trajectory))
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
    pub(crate) async fn numbered_transcript(
        &self,
        session: &str,
    ) -> Result<Vec<(i64, wisp_llm::Message)>> {
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

    /// The project's research graph — its decisions, papers, data, runs and
    /// outputs, and the relations between them — as the journey's
    /// Relationships view shows it: the mainline's, as the WebView reads it.
    pub async fn research_graph(&self, project: &str) -> Result<wisp_dto::ResearchGraph> {
        self.known_project(project).await?;
        let graph = self
            .0
            .store
            .research_graph_in_scope(&wisp_store::StateScope::mainline(project))
            .await?;
        Ok(wisp_dto::ResearchGraph {
            nodes: graph
                .nodes
                .into_iter()
                .map(|node| wisp_dto::ResearchNode {
                    id: node.id,
                    kind: node.kind.as_str().to_owned(),
                    title: node.title,
                    ref_id: node.ref_id,
                    metadata_json: node.metadata_json,
                })
                .collect(),
            edges: graph
                .edges
                .into_iter()
                .map(|edge| wisp_dto::ResearchEdge {
                    source_id: edge.source_id,
                    target_id: edge.target_id,
                    relation: edge.relation,
                    metadata_json: edge.metadata_json,
                })
                .collect(),
        })
    }

    /// Where one version of an output in the journey came from: the run
    /// that produced it, with what that run read, as the WebView's source
    /// details show it. A version of another project, or of an exploration,
    /// is refused before anything of it is read.
    pub async fn journey_source(&self, project: &str, version: &str) -> Result<JourneySource> {
        self.known_project(project).await?;
        let store = &self.0.store;
        let source = store
            .research_journey_source(&wisp_store::StateScope::mainline(project), version)
            .await?;
        let path_of = |version: Option<String>| async move {
            let version = version?;
            let context = store
                .get_artifact_version_context(&version)
                .await
                .ok()
                .flatten()?;
            (context.project_id == project)
                .then(|| project_path(context.logical_key.as_deref()))
                .flatten()
        };
        let path = path_of(Some(version.to_owned())).await;
        let mut input_paths = Vec::with_capacity(source.inputs.len());
        for input in &source.inputs {
            input_paths.push(path_of(input.version_id.clone()).await);
        }
        Ok(JourneySource {
            source,
            path,
            input_paths,
        })
    }

    /// One run of the project as the journey's run record shows it.
    pub async fn journey_run(&self, project: &str, run: &str) -> Result<JourneyRun> {
        self.known_project(project).await?;
        ensure!(
            self.0
                .store
                .run_visible_in_scope(run, &wisp_store::StateScope::mainline(project))
                .await?,
            "Run is unavailable in this project"
        );
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        Ok(JourneyRun {
            id: record.id,
            title: record.title,
            status: record.status.as_str().to_owned(),
            context: record.context_id,
            exit_code: record.exit_code,
            started_at: record.started_at,
            ended_at: record.ended_at,
            command: record.command,
            stdout_tail: record.stdout_tail,
            stderr_tail: record.stderr_tail,
        })
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
        let (succeeded, failed, orphaned) = self.0.store.project_run_retention(project).await?;
        Ok(ProjectSettings {
            name,
            description,
            rules,
            retention: RunRetention {
                succeeded,
                failed,
                orphaned,
            },
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
        let retention = settings.retention;
        ensure!(
            [retention.succeeded, retention.failed, retention.orphaned]
                .into_iter()
                .flatten()
                .all(|days| (1..=RETENTION_DAYS_LIMIT).contains(&days)),
            "Keep runs and files from 1 to {RETENTION_DAYS_LIMIT} days, or leave the field empty"
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
        self.0
            .store
            .set_project_run_retention(
                project,
                retention.succeeded,
                retention.failed,
                retention.orphaned,
            )
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
        crate::explorations::service::reject_private_exploration_project_mutation(
            &self.0.store,
            project,
            "Removing the project",
        )
        .await
        .map_err(anyhow::Error::msg)?;
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

    /// Keeps a file of the project as an artifact of the conversation, as
    /// the WebView's Register as an artifact does: a new version when it was
    /// kept before. Not for Wisp's or git's own files, or while the project
    /// is frozen for an exploration.
    pub async fn register_artifact(&self, project: &str, session: &str, path: &str) -> Result<()> {
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        let (working, scope) =
            crate::explorations::service::working_project_in(store, &self.0.config.data, session)
                .await
                .map_err(anyhow::Error::msg)?;
        ensure!(
            !crate::explorations::service::conversation_project_write_locked(
                store,
                &scope,
                Some(session)
            )
            .await
            .map_err(anyhow::Error::msg)?,
            "The project is frozen while an exploration runs; keep the file once it ends."
        );
        let root = tokio::fs::canonicalize(&working.root).await?;
        let source = std::path::PathBuf::from(path);
        let source = if source.is_absolute() {
            source
        } else {
            root.join(source)
        };
        let real = tokio::fs::canonicalize(&source)
            .await
            .context("That file no longer exists")?;
        let inside = real
            .strip_prefix(&root)
            .ok()
            .filter(|inside| {
                !inside
                    .components()
                    .any(|part| matches!(part.as_os_str().to_str(), Some(".wisp" | ".git")))
            })
            .filter(|_| real.is_file());
        ensure!(
            inside.is_some(),
            "Only a file of the project can be kept as an artifact"
        );
        crate::runtime::record_artifacts(
            store,
            project,
            scope.scope_key(),
            &root,
            session,
            vec![real.to_string_lossy().into_owned()],
        )
        .await;
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
    /// ones included), or the archive itself: the first page of
    /// [`Self::conversation_pages`].
    pub async fn conversations(
        &self,
        project: &str,
        query: &str,
        archived: bool,
    ) -> Result<ConversationList> {
        self.conversation_pages(project, query, archived, 1).await
    }

    /// The first `pages` pages of [`CONVERSATION_PAGE`] conversations each,
    /// newest first; `more` says whether older ones were left out. While
    /// browsing, a pinned conversation is listed even when it is older than
    /// the pages, as the WebView lists it on its first page.
    pub async fn conversation_pages(
        &self,
        project: &str,
        query: &str,
        archived: bool,
        pages: usize,
    ) -> Result<ConversationList> {
        let query = query.trim();
        let wanted = pages.max(1).saturating_mul(CONVERSATION_PAGE);
        let mut grouped = HashMap::new();
        let mut branched = HashMap::new();
        let mut more = false;
        let pinned: Vec<String> = self
            .0
            .store
            .list_pinned_sessions(project)
            .await?
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        let rows: Vec<SessionSearchInfo> = if archived {
            let mut page = self
                .0
                .store
                .list_sessions_page_with_visibility(
                    project,
                    None,
                    wanted.saturating_add(1),
                    Some(true),
                    query,
                )
                .await?;
            more = page.len() > wanted;
            page.truncate(wanted);
            page.into_iter()
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
            let archived_too = !query.is_empty();
            let mut rows: Vec<SessionSearchInfo> = Vec::new();
            let mut seen = HashSet::new();
            let mut offset = 0;
            loop {
                let page = self
                    .session_rows_window(
                        Some(project),
                        query,
                        archived_too,
                        offset,
                        CONVERSATION_PAGE,
                    )
                    .await?;
                let full = page.len() == CONVERSATION_PAGE;
                // A conversation that moved up while the pages were read is
                // listed once.
                rows.extend(page.into_iter().filter(|row| seen.insert(row.id.clone())));
                offset += CONVERSATION_PAGE;
                if !full {
                    break;
                }
                if offset >= wanted {
                    more = !self
                        .session_rows_window(Some(project), query, archived_too, offset, 1)
                        .await?
                        .is_empty();
                    break;
                }
            }
            if query.is_empty() {
                for id in &pinned {
                    if seen.contains(id) {
                        continue;
                    }
                    if let Some(row) = self.session_row(project, id).await? {
                        seen.insert(row.id.clone());
                        rows.push(row);
                    }
                }
            }
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
        let dispatched = self.0.store.list_dispatched_sessions(project).await?;
        Ok(ConversationList {
            rows,
            pinned,
            folders,
            grouped,
            branched,
            dispatched,
            more,
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

    /// Deletes every note of the project's memory, as "Clear all" on the
    /// WebView's Memory page does: the Markdown notes directly inside the
    /// project's memory folder (`.wisp/memory`). Habits, which apply in every
    /// project, and other projects' notes are left alone. Returns how many
    /// notes were deleted; a project without a memory folder has none.
    pub async fn clear_memory_notes(&self, project: &str) -> Result<usize> {
        let _admission = self.0.admission.read().await;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let dir = wisp_core::MemoryManager::at(&root).dir().to_owned();
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut removed = 0;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Only what the Memory tab lists as a note; a folder is not one.
            if memory_note_name(&name).is_err() || entry.file_type().await?.is_dir() {
                continue;
            }
            tokio::fs::remove_file(entry.path()).await?;
            removed += 1;
        }
        if removed > 0 {
            self.memory_changed(project).await?;
        }
        Ok(removed)
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
    pub(crate) async fn library(&self) -> Result<&wisp_store::LibraryStore> {
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

    /// Every version of a saved item's code, the original first: the
    /// item as it was saved is version 1, and each saved edit adds one.
    /// Nothing for an item that is gone.
    pub async fn favorite_versions(&self, id: &str) -> Result<Vec<LibraryItemVersion>> {
        self.library().await?.list_versions(id).await
    }

    /// Saves an edit of a saved item's code as its next version; the item
    /// as it was saved never changes. A text excerpt cannot be edited.
    pub async fn save_favorite_code(&self, id: &str, code: &str) -> Result<LibraryItemVersion> {
        self.library()
            .await?
            .edit_code(id, None, code.to_owned())
            .await
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
        self.transfer(project, session, target, keep, None).await
    }

    /// What moving a conversation to `target` with its artifacts would take
    /// along: the artifacts it owns and their files, and those that stay
    /// because another conversation here uses them. The move is made with
    /// this preview's fingerprint, so what was read is what moves.
    pub async fn preview_move_with_artifacts(
        &self,
        project: &str,
        session: &str,
        target: &str,
    ) -> Result<wisp_dto::SessionArtifactPreview> {
        ensure!(project != target, "Choose a different project");
        self.own_conversation(project, session).await?;
        self.known_project(target).await?;
        self.ensure_branch_changeable(session).await?;
        self.0
            .store
            .preview_session_artifacts(session, project, Some(target))
            .await
    }

    /// Moves a conversation to `target` with the artifacts its preview
    /// listed, as the WebView's move with artifacts does; refused when what
    /// would move has changed since the preview was read.
    pub async fn move_conversation_with_artifacts(
        &self,
        project: &str,
        session: &str,
        target: &str,
        fingerprint: &str,
    ) -> Result<String> {
        self.transfer(project, session, target, false, Some(fingerprint))
            .await
    }

    async fn transfer(
        &self,
        project: &str,
        session: &str,
        target: &str,
        keep: bool,
        artifacts: Option<&str>,
    ) -> Result<String> {
        ensure!(project != target, "Choose a different project");
        let live = self.session(session);
        // Acceptance takes the same lock, so no turn can start mid-transfer.
        let _accept = live.accept.lock().await;
        let _admission = self.0.admission.read().await;
        self.own_conversation(project, session).await?;
        self.known_project(target).await?;
        self.ensure_idle(session, if keep { "copy" } else { "move" })?;
        self.ensure_not_exploring(session, if keep { "copy" } else { "move" })
            .await?;
        let options = self.stored_options(session).await?;
        let id = uuid::Uuid::new_v4().to_string();
        if keep {
            self.0
                .store
                .copy_session_to_project(session, project, target, &id)
                .await?;
        } else {
            // A frozen branch can be copied, not moved away.
            self.ensure_branch_changeable(session).await?;
            self.without_branches(project, session, "move").await?;
            match artifacts {
                Some(fingerprint) => {
                    // Nothing else may touch either project's files meanwhile.
                    let busy = || {
                        anyhow::anyhow!(
                            "A project is busy. Try again when its current work finishes."
                        )
                    };
                    let _source = self
                        .0
                        .project_locks
                        .project(project)
                        .try_write_owned()
                        .map_err(|_| busy())?;
                    let _target = self
                        .0
                        .project_locks
                        .project(target)
                        .try_write_owned()
                        .map_err(|_| busy())?;
                    self.0
                        .store
                        .operate_session_with_artifacts(
                            session,
                            project,
                            Some((target, &id)),
                            fingerprint,
                        )
                        .await?;
                }
                None => {
                    self.0
                        .store
                        .move_session_to_project(session, project, target, &id)
                        .await?;
                }
            }
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
    pub(crate) async fn locate_turn(
        &self,
        session: &str,
        turn: &TurnRef,
    ) -> Result<(i64, Option<i64>)> {
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
        self.ensure_not_exploring(session, "rewind").await?;
        let store = &self.0.store;
        store.require_unarchived_session(session).await?;
        ensure!(
            !store.session_branch_frozen(session).await?,
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
            // The latest request was made of what is now taken back.
            live.context = None;
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
        self.ensure_not_exploring(session, "branch").await?;
        let store = &self.0.store;
        // A branch, frozen or not, is not branched again, as in the WebView.
        ensure!(
            !store.session_is_branch(session).await?,
            wisp_store::BRANCH_AGAIN_REFUSAL
        );
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

    /// An exploration's conversation is kept or discarded, and the source
    /// of an open round stays as it is until the round ends.
    async fn ensure_not_exploring(&self, session: &str, action: &str) -> Result<()> {
        let store = &self.0.store;
        if matches!(
            store.frame_state_scope(session).await?,
            Some(wisp_store::StateScope::Exploration { .. })
        ) {
            bail!("You cannot {action} an exploration's conversation; keep or discard the exploration instead");
        }
        if store.mainline_frame_is_frozen(session).await? {
            bail!("exploration_mainline_frozen: you cannot {action} this conversation until its exploration round is resolved");
        }
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
            self.ensure_not_exploring(session, "archive").await?;
        }
        self.0
            .store
            .set_session_shelved(session, project, archived)
            .await
    }

    pub(crate) async fn project_root(&self, project: &str) -> Result<String> {
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

    /// Keeps something pasted (a picture, say) in the project's `uploads`
    /// folder as `name`, for a message to carry.
    pub async fn import_pasted(&self, project: &str, name: &str, bytes: &[u8]) -> Result<String> {
        let root = self.project_root(project).await?;
        crate::files::import_bytes(std::path::Path::new(&root), name, bytes).await
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
            hosts: hosts
                .into_iter()
                .map(|host| {
                    let mut row = SshHostRow::from(host);
                    row.has_password = self.ssh_password(&row.alias).is_some();
                    row
                })
                .collect(),
            config_aliases: aliases,
        })
    }

    /// Adds an SSH host, or changes the one that has the alias. Wisp keeps
    /// where the key file is, never the key.
    pub async fn save_ssh_host(&self, input: SshHostRow) -> Result<()> {
        use wisp_runs::ssh_hosts;
        let _admission = self.0.admission.read().await;
        let typed = input
            .password
            .clone()
            .filter(|password| !password.is_empty());
        let password_sign_in = input.password_sign_in;
        let host = input.into_host()?;
        let alias = host.alias.clone();
        // A password host needs one, typed now or saved before.
        ensure!(
            !password_sign_in || typed.is_some() || self.ssh_password(&alias).is_some(),
            "Give the password to sign in with"
        );
        let hosts = ssh_hosts::upsert_host(ssh_hosts::load(&self.0.store).await, host);
        ssh_hosts::save_and_sync_contexts(&self.0.store, &hosts)
            .await
            .map_err(anyhow::Error::msg)?;
        let name = ssh_hosts::password_secret_name(&alias);
        match (password_sign_in, typed) {
            (true, Some(password)) => self.0.secrets.set(&name, &password)?,
            (true, None) => {}
            // Signing in by key: no password is kept for it.
            (false, _) => self.0.secrets.set(&name, "")?,
        }
        Ok(())
    }

    /// The password saved for the SSH host `alias`, from the keyring.
    fn ssh_password(&self, alias: &str) -> Option<String> {
        self.0
            .secrets
            .get(&wisp_runs::ssh_hosts::password_secret_name(alias))
            .filter(|password| !password.is_empty())
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
    /// What the system's terminal runs to open a shell on an execution
    /// context, as the WebView's terminal launches it: `ssh` for an SSH
    /// host, in `folder` on it when one is given, and `wsl.exe` for a WSL
    /// distribution, in the project's folder. A password is typed in the
    /// terminal; it is never put on the command line.
    pub async fn terminal_launch(
        &self,
        project: &str,
        context: &str,
        folder: Option<&str>,
    ) -> Result<TerminalLaunch> {
        use wisp_runs::ssh_hosts;
        let context = self
            .0
            .store
            .get_execution_context(context)
            .await?
            .context("That environment is no longer saved")?;
        match context.kind {
            wisp_store::ExecutionContextKind::Ssh => {
                let connection = ssh_hosts::SshConnection::from_execution_context(&context)
                    .map_err(anyhow::Error::msg)?;
                let mut args = connection
                    .interactive_ssh_args()
                    .map_err(anyhow::Error::msg)?;
                if let Some(folder) = folder.map(str::trim).filter(|folder| !folder.is_empty()) {
                    args.push(format!(
                        "cd {} && exec \"${{SHELL:-/bin/sh}}\" -l",
                        shell_quote(folder)
                    ));
                }
                Ok(TerminalLaunch {
                    program: "ssh".into(),
                    args,
                })
            }
            wisp_store::ExecutionContextKind::Wsl => {
                let config: serde_json::Value =
                    serde_json::from_str(&context.config_json).unwrap_or_default();
                let distro = config
                    .get("distro")
                    .and_then(|value| value.as_str())
                    .unwrap_or_else(|| context.id.strip_prefix("wsl:").unwrap_or(&context.id))
                    .to_owned();
                let root = self.project_root(project).await?;
                Ok(TerminalLaunch {
                    program: "wsl.exe".into(),
                    args: vec!["-d".into(), distro, "--cd".into(), root],
                })
            }
            wisp_store::ExecutionContextKind::Local => {
                bail!("This computer's terminal opens in the project folder")
            }
        }
    }

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

    /// A file of a saved SSH host, for the Files tab to show, read as the
    /// WebView's preview reads it: text as its first megabyte, a picture
    /// whole up to 32 MB. `listed_size` is what the folder listing said; a
    /// picture larger than the cap by it is not fetched. Nothing is kept,
    /// in the project or anywhere else.
    pub async fn remote_file(
        &self,
        alias: &str,
        path: &str,
        listed_size: Option<u64>,
    ) -> Result<RemoteFile> {
        let context = self.host_to_read(alias, path).await?;
        let (alias, path) = (alias.trim().to_owned(), path.to_owned());
        tokio::task::spawn_blocking(move || {
            let mut runner = wisp_runs::remote_file::ProcessRemoteRunner;
            crate::remote_file::fetch(&alias, &context, &path, listed_size, &mut runner)
        })
        .await?
        .map_err(anyhow::Error::msg)
    }

    /// The saved host `alias`, when `path` of it may be read: a path is
    /// named, the host's files were not given up when it was discarded,
    /// and it has passed a check.
    async fn host_to_read(&self, alias: &str, path: &str) -> Result<wisp_store::ExecutionContext> {
        ensure!(!path.trim().is_empty(), "Choose a file of the host");
        let id = format!("ssh:{}", alias.trim());
        let context = self
            .0
            .store
            .get_execution_context(&id)
            .await?
            .context("That host is no longer saved")?;
        wisp_runs::remote_files::refuse_if_context_path_discarded(&self.0.store, &id, path)
            .await
            .map_err(anyhow::Error::msg)?;
        wisp_runs::ssh_hosts::require_managed_ssh_ready(&context).map_err(anyhow::Error::msg)?;
        Ok(context)
    }

    /// What the project placed on a saved SSH host and keeps a record of:
    /// the WebView's Remote files list of a server. Each says whether a
    /// run still needs it, a newer copy replaced it, or nothing does.
    pub async fn host_staged_files(&self, project: &str, alias: &str) -> Result<Vec<StagedFile>> {
        self.known_project(project).await?;
        let context = self.saved_host(alias).await?;
        wisp_runs::remote_files::list_remote_files(&self.0.store, project, &context.id)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Deletes one of those files from the host for good, as the WebView's
    /// Delete from server does: only a file in the project's record, and
    /// not one a run still needs. A replaced one only leaves the record,
    /// since its path holds the newer copy. Answers the list again.
    pub async fn remove_host_staged_file(
        &self,
        project: &str,
        alias: &str,
        id: &str,
    ) -> Result<Vec<StagedFile>> {
        self.remove_host_staged_file_with(project, alias, id, self.0.runs.runner_ref())
            .await
    }

    pub(crate) async fn remove_host_staged_file_with(
        &self,
        project: &str,
        alias: &str,
        id: &str,
        runner: &dyn wisp_runs::RunCommandRunner,
    ) -> Result<Vec<StagedFile>> {
        self.known_project(project).await?;
        let context = self.saved_host(alias).await?;
        let _admission = self.0.admission.read().await;
        wisp_runs::remote_files::remove_remote_files(
            &self.0.store,
            runner,
            project,
            &context,
            &[id.to_owned()],
            false,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        wisp_runs::remote_files::list_remote_files(&self.0.store, project, &context.id)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// The project's runs on a saved SSH host, newest first: the WebView's
    /// Runs list of a server.
    pub async fn host_runs(&self, project: &str, alias: &str) -> Result<Vec<HostRun>> {
        self.known_project(project).await?;
        let context = self.saved_host(alias).await?;
        Ok(self
            .0
            .store
            .list_run_summaries_in_scope(&wisp_store::StateScope::mainline(project.to_owned()))
            .await?
            .into_iter()
            .filter(|run| run.context_id == context.id)
            .map(|run| HostRun {
                id: run.id,
                session: run.frame_id,
                title: run.title,
                kind: run.kind,
                status: run.status.as_str().to_owned(),
                created_at: run.created_at,
                ended_at: run.ended_at,
                exit_code: run.exit_code,
            })
            .collect())
    }

    /// A saved SSH host by its alias.
    async fn saved_host(&self, alias: &str) -> Result<wisp_store::ExecutionContext> {
        let context = self
            .0
            .store
            .get_execution_context(&format!("ssh:{}", alias.trim()))
            .await?
            .context("That host is no longer saved")?;
        ensure!(
            context.kind == wisp_store::ExecutionContextKind::Ssh,
            "That is not an SSH host"
        );
        Ok(context)
    }

    /// A saved SSH host's execution context, ready for work, and `session`
    /// checked to be `project`'s when one is given.
    async fn host_for_transfer(
        &self,
        project: &str,
        session: Option<&str>,
        alias: &str,
    ) -> Result<wisp_store::ExecutionContext> {
        match session {
            Some(session) => self.own_conversation(project, session).await?,
            None => drop(self.project_root(project).await?),
        }
        let context = self
            .0
            .store
            .get_execution_context(&format!("ssh:{}", alias.trim()))
            .await?
            .context("That host is no longer saved")?;
        wisp_runs::ssh_hosts::require_managed_ssh_ready(&context).map_err(anyhow::Error::msg)?;
        Ok(context)
    }

    /// Copies a file of a saved SSH host to `destination` on this computer.
    /// It goes on as a run of the conversation, when one is given, whose
    /// progress shows with its runs; `destination` is only replaced once the
    /// whole file has come. Answers the run's id.
    pub async fn download_from_host(
        &self,
        project: &str,
        session: Option<&str>,
        alias: &str,
        path: &str,
        destination: std::path::PathBuf,
    ) -> Result<String> {
        let context = self.host_for_transfer(project, session, alias).await?;
        let _admission = self.0.admission.read().await;
        self.0
            .runs
            .submit_ssh_file_download(
                &self.0.store,
                project,
                session,
                &context,
                path,
                &destination,
            )
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Copies files of this computer into `folder` of a saved SSH host, one
    /// run each, which go on as the conversation's runs when one is given.
    /// A name already there is not replaced. Answers the runs' ids.
    pub async fn upload_to_host(
        &self,
        project: &str,
        session: Option<&str>,
        alias: &str,
        folder: &str,
        sources: Vec<String>,
    ) -> Result<Vec<String>> {
        ensure!(!sources.is_empty(), "Choose the files to send");
        let context = self.host_for_transfer(project, session, alias).await?;
        let _admission = self.0.admission.read().await;
        Ok(wisp_runs::submit_local_uploads_to_context(
            &self.0.store,
            &self.0.runs,
            project,
            session,
            &context.id,
            folder,
            &sources,
        )
        .await
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .map(|item| item.run_id)
        .collect())
    }

    /// Where a project's work keeps things on a saved SSH host: the folder
    /// data is uploaded to, the root of run folders there, and the
    /// project folder results come back into. What was never saved reads
    /// as the defaults the run manager uses, and says so.
    pub async fn host_storage(&self, project: &str, alias: &str) -> Result<HostStorage> {
        self.known_project(project).await?;
        let context = format!("ssh:{}", alias.trim());
        self.0
            .store
            .get_execution_context(&context)
            .await?
            .context("That host is no longer saved")?;
        let (prefs, saved) =
            wisp_runs::storage_prefs::effective_prefs(&self.0.store, project, &context)
                .await
                .map_err(anyhow::Error::msg)?;
        Ok(HostStorage {
            data_root: prefs.remote_data_root,
            workdir_root: prefs.remote_workdir_root,
            results_dir: prefs.local_results_dir,
            saved,
        })
    }

    /// Keeps where a project's work keeps things on a saved SSH host. The
    /// paths are checked as the run manager needs them.
    pub async fn set_host_storage(
        &self,
        project: &str,
        alias: &str,
        storage: HostStorage,
    ) -> Result<HostStorage> {
        // The project and the host are checked the same way.
        self.host_storage(project, alias).await?;
        let now = chrono::Utc::now().timestamp();
        // The store names the fields as its columns; say which folder.
        let say = |error: anyhow::Error| {
            anyhow::anyhow!(
                "{}",
                format!("{error:#}")
                    .replace("remote_data_root", "The folder for uploaded data")
                    .replace("remote_workdir_root", "The place for run folders")
                    .replace("local_results_dir", "The results folder")
            )
        };
        self.0
            .store
            .upsert_context_storage_prefs(&wisp_store::ContextStoragePrefs {
                project_id: project.to_owned(),
                context_id: format!("ssh:{}", alias.trim()),
                remote_data_root: storage.data_root.trim().to_owned(),
                remote_workdir_root: storage.workdir_root.trim().to_owned(),
                local_results_dir: storage.results_dir.trim().to_owned(),
                created_at: now,
                updated_at: now,
            })
            .await
            .map_err(say)?;
        self.host_storage(project, alias).await
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
        // Its password goes with it.
        self.0
            .secrets
            .set(&ssh_hosts::password_secret_name(alias), "")?;
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

    /// Where a server's environment variables and headers are kept, with
    /// their values, as one JSON object: `{"env": {…}, "headers": {…}}`.
    pub(crate) fn mcp_extra_secret(key_id: &str, server: &str) -> String {
        format!("desktop.mcp.{key_id}.{server}.extra")
    }

    /// The values saved for a server's environment variables and headers.
    async fn mcp_extra(
        &self,
        server: &str,
    ) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
        #[derive(Default, serde::Deserialize)]
        struct Extra {
            #[serde(default)]
            env: BTreeMap<String, String>,
            #[serde(default)]
            headers: BTreeMap<String, String>,
        }
        let Some(key_id) = self.0.store.get_setting(MODEL_KEY_ID).await.ok().flatten() else {
            return Default::default();
        };
        let extra: Extra = self
            .0
            .secrets
            .get(&Self::mcp_extra_secret(&key_id, server))
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        (extra.env, extra.headers)
    }

    /// What a connection to `server` is made with: its token, and the
    /// variables and headers it names with their saved values.
    pub(crate) async fn mcp_credentials(&self, server: &McpServer) -> crate::mcp::Credentials {
        let (env, headers) = self.mcp_extra(&server.id).await;
        let pick = |names: &[String], values: &BTreeMap<String, String>| {
            names
                .iter()
                .filter_map(|name| Some((name.clone(), values.get(name)?.clone())))
                .collect()
        };
        crate::mcp::Credentials {
            token: self.mcp_token(&server.id).await,
            env: pick(&server.env, &env),
            headers: pick(&server.headers, &headers),
            plugin: None,
        }
    }

    /// What MCP gives a turn in `project`: your servers' tools and those of
    /// the servers its plugins declare.
    pub(crate) async fn turn_mcp_tools(&self, project: &str) -> Vec<Box<dyn wisp_tools::Tool>> {
        let mut tools = self.mcp_tools().await;
        for tool in self.plugin_mcp_tools(project).await {
            if !tools.iter().any(|other| other.name() == tool.name()) {
                tools.push(tool);
            }
        }
        tools
    }

    /// The tools the MCP servers of `project`'s plugins give a turn; each
    /// asks before it runs. A server that cannot start gives none.
    pub(crate) async fn plugin_mcp_tools(&self, project: &str) -> Vec<Box<dyn wisp_tools::Tool>> {
        let (launches, problems) = crate::plugins::enabled_launches(&self.0.store, project).await;
        for problem in problems {
            tracing::warn!("{problem}");
        }
        let mut credentials = HashMap::new();
        let servers: Vec<McpServer> = launches
            .into_iter()
            .map(|launch| {
                credentials.insert(
                    launch.id.clone(),
                    crate::mcp::Credentials {
                        env: launch.env.into_iter().collect(),
                        plugin: Some((launch.cwd, launch.root)),
                        ..Default::default()
                    },
                );
                McpServer {
                    id: launch.id,
                    name: launch.name,
                    enabled: true,
                    transport: McpTransport::Stdio {
                        command: launch.command.to_string_lossy().into_owned(),
                        args: launch.args,
                    },
                    env: vec![],
                    headers: vec![],
                    proxy: String::new(),
                }
            })
            .collect();
        if servers.is_empty() {
            return vec![];
        }
        let decision = self.decision_model_in_use().await;
        self.0
            .mcp
            .tools(
                &servers,
                |id| credentials.get(id).cloned().unwrap_or_default(),
                decision,
            )
            .await
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
        let mut credentials = HashMap::new();
        for server in &servers {
            credentials.insert(server.id.clone(), self.mcp_credentials(server).await);
        }
        let decision = self.decision_model_in_use().await;
        self.0
            .mcp
            .tools(
                &servers,
                |id| credentials.get(id).cloned().unwrap_or_default(),
                decision,
            )
            .await
    }

    /// The MCP servers the user added, in the order they were added, each
    /// with whether a token is saved for it and why it gave no tools the
    /// last time, if it did not.
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        let mut rows = Vec::new();
        for server in crate::mcp::load(&self.0.store).await {
            let (env, headers) = self.mcp_extra(&server.id).await;
            let unset = server
                .env
                .iter()
                .filter(|name| !env.contains_key(*name))
                .chain(
                    server
                        .headers
                        .iter()
                        .filter(|name| !headers.contains_key(*name)),
                )
                .cloned()
                .collect();
            rows.push(McpServerRow {
                unset,
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
            env: Vec::new(),
            headers: Vec::new(),
            proxy: match input.proxy {
                Some(route) => crate::network::own_route("Route", &route)?,
                None => existing
                    .map(|index| servers[index].proxy.clone())
                    .unwrap_or_default(),
            },
        };
        let mut server = server;
        server.validate()?;
        // The variables and headers: a name typed with no value keeps the
        // value saved for it; one left out is forgotten.
        let (saved_env, saved_headers) = self.mcp_extra(&server.id).await;
        let merge = |given: Option<Vec<(String, String)>>,
                     saved: &BTreeMap<String, String>,
                     kept: &[String],
                     valid: fn(&str) -> bool,
                     what: &str|
         -> Result<BTreeMap<String, String>> {
            let Some(given) = given else {
                return Ok(kept
                    .iter()
                    .filter_map(|name| Some((name.clone(), saved.get(name)?.clone())))
                    .collect());
            };
            ensure!(given.len() <= 32, "Keep to 32 {what}s");
            let mut out = BTreeMap::new();
            for (name, value) in given {
                let name = name.trim().to_owned();
                ensure!(valid(&name), "\"{name}\" cannot be the name of a {what}");
                let value = match value.trim() {
                    "" => saved
                        .get(&name)
                        .cloned()
                        .with_context(|| format!("Give {name} a value"))?,
                    value => value.to_owned(),
                };
                ensure!(
                    crate::mcp::valid_value(&value),
                    "The value of {name} is one line of at most 8192 characters"
                );
                out.insert(name, value);
            }
            Ok(out)
        };
        let before = existing.map(|index| servers[index].clone());
        let env = merge(
            input.env,
            &saved_env,
            before.as_ref().map_or(&[][..], |server| &server.env),
            crate::mcp::valid_env_name,
            "variable",
        )?;
        let headers = merge(
            input.headers,
            &saved_headers,
            before.as_ref().map_or(&[][..], |server| &server.headers),
            crate::mcp::valid_header_name,
            "header",
        )?;
        server.env = env.keys().cloned().collect();
        server.headers = headers.keys().cloned().collect();
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
        if !env.is_empty()
            || !headers.is_empty()
            || !saved_env.is_empty()
            || !saved_headers.is_empty()
        {
            let key_id = self.key_id().await?;
            let extra = match env.is_empty() && headers.is_empty() {
                true => String::new(),
                false => serde_json::to_string(&serde_json::json!({
                    "env": env,
                    "headers": headers,
                }))?,
            };
            self.0
                .secrets
                .set(&Self::mcp_extra_secret(&key_id, &server.id), &extra)?;
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
            self.0
                .secrets
                .set(&Self::mcp_extra_secret(&key_id, id), "")?;
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
        let credentials = self.mcp_credentials(server).await;
        Ok(self
            .0
            .mcp
            .list(server, credentials)
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
    /// services' own documentation names them, and then the credentials of
    /// your own under the names you gave them. Read at the start of a turn.
    pub(crate) async fn service_keys(&self) -> Vec<(String, String)> {
        let mut keys = Vec::new();
        // No id yet means nothing was ever saved; do not make one to look.
        if let Ok(Some(key_id)) = self.0.store.get_setting(MODEL_KEY_ID).await {
            keys.extend(SERVICE_KEYS.iter().filter_map(|key| {
                let value = self.0.secrets.get(&Self::service_secret(&key_id, key.id))?;
                // A forgotten key is stored empty.
                (!value.is_empty()).then(|| (key.env.to_owned(), value))
            }));
        }
        for row in custom_rows(&self.0.store).await {
            if let Some(value) = self.0.secrets.get(&custom_secret(&row.id)) {
                if !value.is_empty() {
                    keys.push((row.env_var, value));
                }
            }
        }
        keys
    }

    /// Credentials of your own: a name, the environment variable code run
    /// by the model reads, and whether a value is saved (never the value).
    /// Kept as the WebView keeps them, so either desktop has them.
    pub async fn custom_credentials(&self) -> Result<Vec<CustomCredential>> {
        Ok(custom_rows(&self.0.store)
            .await
            .into_iter()
            .map(|row| CustomCredential {
                saved: self
                    .0
                    .secrets
                    .get(&custom_secret(&row.id))
                    .is_some_and(|value| !value.is_empty()),
                id: row.id,
                name: row.name,
                env_var: row.env_var,
            })
            .collect())
    }

    /// Adds a credential of your own, or gives the one under the same
    /// environment variable a new name and value. The value goes to the
    /// system keychain; only the name and variable are kept here.
    pub async fn add_custom_credential(
        &self,
        name: &str,
        env_var: &str,
        value: &str,
    ) -> Result<Vec<CustomCredential>> {
        let (name, env_var, value) = (name.trim(), env_var.trim(), value.trim());
        ensure!(!name.is_empty(), "Give the credential a name");
        ensure!(
            name.chars().count() <= 80,
            "A credential's name is at most 80 characters"
        );
        ensure!(
            env_var.len() <= 128 && valid_env_var(env_var),
            "An environment variable starts with a letter or underscore and has only letters, digits and underscores"
        );
        ensure!(!value.is_empty(), "Enter the credential's value");
        ensure!(
            !SERVICE_KEYS
                .iter()
                .any(|key| key.env.eq_ignore_ascii_case(env_var)),
            "{env_var} is one of the service keys above"
        );
        let mut rows = custom_rows(&self.0.store).await;
        let id = match rows
            .iter_mut()
            .find(|row| row.env_var.eq_ignore_ascii_case(env_var))
        {
            // The same variable again replaces its value in place, so a
            // lost value never blocks setting it again.
            Some(row) => {
                row.name = name.to_owned();
                row.id.clone()
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                rows.push(CustomRow {
                    id: id.clone(),
                    name: name.to_owned(),
                    env_var: env_var.to_owned(),
                });
                id
            }
        };
        self.0.secrets.set(&custom_secret(&id), value)?;
        self.0
            .store
            .set_setting(CUSTOM_CREDENTIALS, &serde_json::to_string(&rows)?)
            .await?;
        self.custom_credentials().await
    }

    /// Removes a credential of your own and its value.
    pub async fn remove_custom_credential(&self, id: &str) -> Result<Vec<CustomCredential>> {
        let mut rows = custom_rows(&self.0.store).await;
        let before = rows.len();
        rows.retain(|row| row.id != id);
        ensure!(rows.len() < before, "That credential is no longer there");
        // Stored empty, as a forgotten service key is.
        self.0.secrets.set(&custom_secret(id), "")?;
        self.0
            .store
            .set_setting(CUSTOM_CREDENTIALS, &serde_json::to_string(&rows)?)
            .await?;
        self.custom_credentials().await
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
        // Two CLIs keep the key where they read it, as the WebView sets up.
        match key.id {
            "infinisynapse_api_key" if !value.is_empty() => {
                init_agent_infini(&self.tools_home()?, !cfg!(test), value).await?
            }
            "scimaster_api_key" => sync_scimaster(&self.tools_home()?, value)?,
            _ => {}
        }
        let key_id = self.key_id().await?;
        self.0
            .secrets
            .set(&Self::service_secret(&key_id, key.id), value)
    }

    /// The folder command-line tools keep their settings in: your home
    /// folder, or one inside the data directory under test, so tests never
    /// write to the real one.
    fn tools_home(&self) -> Result<std::path::PathBuf> {
        if cfg!(test) {
            return Ok(self.0.config.data.join("home"));
        }
        std::env::home_dir().context("The home folder could not be found")
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
            tools: rank_tool_calls(
                store.tool_call_counts().await?,
                &crate::mcp::load(store).await,
            ),
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
                tools_bytes: folder_size(&data.join("toolchain")),
                plugins_bytes: folder_size(&data.join("plugins")),
                data_bytes: folder_size(&data),
                projects: projects
                    .into_iter()
                    .map(|(id, name, workspace, _, _, conversations, ..)| {
                        let root = std::path::Path::new(&workspace);
                        let (workspace_bytes, workspace_more) = folder_size_bounded(root);
                        ProjectStorage {
                            conversations: conversations.max(0) as usize,
                            wisp_bytes: folder_size(&root.join(".wisp")),
                            uploads_bytes: folder_size(&root.join("uploads")),
                            workspace_bytes,
                            workspace_more,
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
        let own_tags = skill_tags(&self.0.store).await;
        Ok(tokio::task::spawn_blocking(move || {
            let index = crate::runtime::all_skills(&config, root.as_deref());
            let mut rows: Vec<SkillRow> = index
                .all()
                .iter()
                .map(|skill| SkillRow {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    // The researcher's tags take the place of the skill's own.
                    tags: own_tags
                        .get(&skill.name)
                        .cloned()
                        .unwrap_or_else(|| skill.tags.clone()),
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

    /// Gives a skill the researcher's own tags in place of those it came
    /// with; none puts its own back. Kept, as the WebView keeps them, under
    /// `skill_tags`.
    pub async fn set_skill_tags(&self, name: &str, tags: Vec<String>) -> Result<()> {
        let name = name.trim();
        ensure!(!name.is_empty(), "Which skill?");
        let mut all = skill_tags(&self.0.store).await;
        let tags = normalize_tags(tags);
        ensure!(
            tags.iter().all(|tag| tag.chars().count() <= 40),
            "A tag is at most 40 characters"
        );
        if tags.is_empty() {
            all.remove(name);
        } else {
            all.insert(name.to_owned(), tags);
        }
        self.0
            .store
            .set_setting(SKILL_TAGS, &serde_json::to_string(&all)?)
            .await
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
    /// The versions kept of one of the project's artifacts, oldest first:
    /// a snapshot each time a turn wrote the file.
    pub async fn artifact_versions(
        &self,
        project: &str,
        artifact: &str,
    ) -> Result<Vec<ArtifactVersionRow>> {
        self.project_root(project).await?;
        let versions = self.0.store.list_artifact_versions(artifact).await?;
        let first = versions.first().context("That artifact has no versions")?;
        let context = self
            .0
            .store
            .get_artifact_version_context(&first.id)
            .await?
            .context("That artifact has no versions")?;
        ensure!(
            context.project_id == project,
            "That artifact belongs to another project"
        );
        Ok(versions
            .into_iter()
            .map(|version| ArtifactVersionRow {
                id: version.id,
                number: version.version_number,
                kind: version.content_type,
                size_bytes: version.size_bytes.and_then(|size| u64::try_from(size).ok()),
                created_at: version.created_at,
            })
            .collect())
    }

    /// Saves a copy of one version of an artifact where the researcher
    /// chose, from its snapshot, after checking it is what was kept.
    pub async fn save_artifact_version(
        &self,
        project: &str,
        version: &str,
        destination: &std::path::Path,
    ) -> Result<()> {
        const LIMIT: usize = 512 * 1024 * 1024;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let (_, bytes) = crate::method_search::coordinator::load_artifact_bytes(
            &self.0.store,
            &root,
            project,
            version,
            LIMIT,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        tokio::fs::write(destination, bytes)
            .await
            .with_context(|| format!("Could not save {}", destination.display()))
    }

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

    /// The files each turn of a conversation recorded as artifacts, oldest
    /// turn first, each with the turn's last tool call so it can be shown
    /// under the answer that closes that turn, as the WebView shows a
    /// reply's generated files. A version deleted since is left out; one a
    /// later turn wrote again is marked. Turns from before the host kept
    /// this have none.
    pub async fn reply_artifacts(
        &self,
        project: &str,
        session: &str,
    ) -> Result<Vec<ReplyArtifact>> {
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        let mut rows = Vec::new();
        for saved in store.load_session_ui_events(session).await? {
            let Ok(event) = serde_json::from_str::<crate::output::ArtifactsEvent>(&saved) else {
                continue;
            };
            if event.kind != crate::output::ARTIFACTS_EVENT {
                continue;
            }
            for version in event.versions {
                let Some(context) = store.get_artifact_version_context(&version).await? else {
                    continue;
                };
                rows.push(ReplyArtifact {
                    call_id: event.call_id.clone(),
                    artifact: context.version.artifact_id,
                    version: context.version.id,
                    name: context.filename,
                    kind: context.version.content_type,
                    path: context
                        .logical_key
                        .as_deref()
                        .and_then(|key| key.strip_prefix("path:"))
                        .map(str::to_owned),
                    superseded: false,
                });
            }
        }
        Ok(mark_superseded(rows))
    }

    /// Asks a run that has not finished to stop.
    pub async fn cancel_run(&self, project: &str, run: &str) -> Result<()> {
        self.project_root(project).await?;
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        // A method search ends through its own coordinator, as in the WebView.
        if record.kind == "method_search" {
            return self.cancel_method_search(project, run).await.map(drop);
        }
        self.0
            .runs
            .cancel(&self.0.store, run)
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Cancels a run as the button that was pressed meant it, as the Tauri
    /// desktop does for its native clients (`cancel_with_intent`): `force`
    /// off asks a run that is going to stop; on, it finishes a run already
    /// stopping whose stop never came back, as cancelled. A button drawn
    /// for the other state is refused, so a stale Cancel never forces.
    pub async fn cancel_run_as_meant(&self, project: &str, run: &str, force: bool) -> Result<()> {
        self.project_root(project).await?;
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        // A method search ends through its own coordinator and is never
        // forced, as in the Tauri desktop.
        if record.kind == "method_search" {
            ensure!(!force, "Run state changed; refresh before cancelling");
            return self.cancel_method_search(project, run).await.map(drop);
        }
        self.0
            .runs
            .cancel_with_intent(&self.0.store, run, Some(force))
            .await
            .map_err(anyhow::Error::msg)
    }

    /// Deletes the workspace a finished run kept on the machine it ran on,
    /// once its logs are saved into the project. Outputs it declared but
    /// never brought back are kept from loss unless `force` says they may
    /// go. A run already cleaned is left as it is.
    pub async fn clean_run(&self, project: &str, run: &str, force: bool) -> Result<()> {
        self.project_root(project).await?;
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        ensure!(
            record.kind != "method_search" && record.kind != "file_transfer",
            "This run keeps no workspace of its own to clean"
        );
        self.0
            .runs
            .cleanup_run_workspace(&self.0.store, run, force)
            .await
            .map(drop)
            .map_err(anyhow::Error::msg)
    }

    /// A run of `project`, checked to be its own.
    pub(crate) async fn own_run(&self, project: &str, run: &str) -> Result<wisp_store::RunRecord> {
        self.project_root(project).await?;
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        Ok(record)
    }

    /// One page of one folder of the workspace a finished run left on its
    /// SSH host, `filter` matching names there. Read each time; nothing of
    /// it is kept.
    pub async fn run_workspace(
        &self,
        project: &str,
        run: &str,
        path: &str,
        filter: &str,
        offset: usize,
    ) -> Result<wisp_dto::WorkspaceListing> {
        self.own_run(project, run).await?;
        let listing = self
            .0
            .runs
            .list_run_workspace_files(&self.0.store, run, path, filter.trim(), offset, 200)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(wisp_dto::WorkspaceListing {
            truncated: listing.truncated,
            entries: listing
                .entries
                .into_iter()
                .map(|entry| wisp_dto::WorkspaceEntry {
                    path: entry.path,
                    kind: entry.kind,
                    size_bytes: entry.size_bytes,
                    file_count: entry.file_count,
                })
                .collect(),
        })
    }

    /// Brings the chosen files and folders of a finished run's workspace
    /// into the project as the conversation's results (a folder as one
    /// archive). Returns how many were brought.
    pub async fn download_run_files(
        &self,
        project: &str,
        run: &str,
        files: Vec<String>,
        folders: Vec<String>,
    ) -> Result<usize> {
        self.own_run(project, run).await?;
        ensure!(
            !files.is_empty() || !folders.is_empty(),
            "Choose the files or folders to bring back"
        );
        let _admission = self.0.admission.read().await;
        self.0
            .runs
            .download_run_files(&self.0.store, run, &files, &folders)
            .await
            .map(|brought| brought.len())
            .map_err(anyhow::Error::msg)
    }

    /// Deletes the chosen files and folders of a finished run's workspace
    /// on its host.
    pub async fn delete_run_files(
        &self,
        project: &str,
        run: &str,
        paths: Vec<String>,
    ) -> Result<()> {
        self.own_run(project, run).await?;
        ensure!(!paths.is_empty(), "Choose the files or folders to delete");
        ensure!(
            paths.iter().all(|path| !path.is_empty()),
            "The whole workspace is deleted by cleaning the run up"
        );
        // Each path is checked to stay inside the workspace there.
        self.0
            .runs
            .delete_run_files(&self.0.store, run, &paths)
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
    /// A run on this computer keeps its log where it can be read, and so
    /// does one whose workspace was cleaned up, its log saved into the
    /// project; any other run on another machine has its last lines in the
    /// run record.
    pub async fn run_log(&self, project: &str, run: &str) -> Result<RunLog> {
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let record = self.0.store.get_run(run).await?.context("Run not found")?;
        ensure!(
            record.project_id == project,
            "Run does not belong to the selected project"
        );
        if let Some(saved) = record.logs_path.as_deref() {
            let folder = crate::run_log::saved(&root, saved, &record.id)?;
            return tokio::task::spawn_blocking(move || crate::run_log::read(&folder)).await?;
        }
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
            fast: is_fast(self.0.store.frame_service_tier(session).await?.as_deref()),
            subagent_model: self
                .0
                .store
                .frame_subagent_model(session)
                .await?
                .map(|id| id.trim().to_owned()),
        })
    }

    /// Saves where a conversation's work runs. Nothing is written when it
    /// is already so, which also leaves alone a choice this desktop cannot
    /// offer but another one made.
    /// The saved SSH hosts this conversation's work may run on, by alias.
    /// This computer always may; a run on a host that is not attached is
    /// refused.
    pub async fn conversation_hosts(&self, project: &str, session: &str) -> Result<Vec<String>> {
        self.own_conversation(project, session).await?;
        Ok(self
            .0
            .store
            .list_session_execution_context_ids(session)
            .await?
            .into_iter()
            .filter_map(|id| id.strip_prefix("ssh:").map(str::to_owned))
            .collect())
    }

    /// Lets this conversation's work run on a saved SSH host, or stops it.
    /// The host its new work goes to stays attached until another is chosen.
    /// Answers the hosts attached now.
    pub async fn set_conversation_host(
        &self,
        project: &str,
        session: &str,
        alias: &str,
        attached: bool,
    ) -> Result<Vec<String>> {
        use wisp_runs::ssh_hosts::{self, SessionDefaultExecutionContext as Choice};
        self.own_conversation(project, session).await?;
        let store = &self.0.store;
        let id = format!("ssh:{}", alias.trim());
        {
            let _admission = self.0.admission.read().await;
            if attached {
                ensure!(
                    ssh_hosts::load(store)
                        .await
                        .iter()
                        .any(|host| host.alias == alias.trim()),
                    "That host is no longer saved; choose another"
                );
            } else {
                ensure!(
                    ssh_hosts::stored_session_default_execution_context(store, session).await
                        != Choice::Remote(id.clone()),
                    "New work in this conversation runs on that host; choose another where it runs first"
                );
            }
            store
                .set_session_execution_context_enabled(session, &id, attached)
                .await?;
        }
        self.conversation_hosts(project, session).await
    }

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
        // An ACP agent runs on its own model, not one of Wisp's.
        let fast_available = options.acp_agent.is_empty()
            && self
                .0
                .model
                .read()
                .unwrap()
                .pick(&options.model)
                .is_ok_and(|model| fast_available(&model.provider));
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
        let queued = session.queue.lock().unwrap().list();
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
            fast_available,
            memory_proposal: live.memory_proposal.clone(),
            queued,
            hook_failures: live.hook_failures.clone(),
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

    /// Goes on from a frozen archive, as the WebView's Continue research
    /// does: a new conversation of the project, linked to the archive,
    /// named after it and opened with the report and where the retained
    /// materials and the original conversation are. Nothing is sent yet.
    /// Returns the new conversation.
    pub async fn continue_research(&self, project: &str, session: &str) -> Result<String> {
        let archive = self
            .research_archive(project, session)
            .await?
            .context("This conversation's research is not archived")?;
        let id = self.create_conversation(project).await?;
        let store = &self.0.store;
        store.link_archive_continuation(&archive, &id).await?;
        store
            .rename_session(&id, project, &format!("Continue · {}", archive.title))
            .await?;
        store
            .append_message(
                &id,
                1,
                &wisp_llm::Message::user(format!(
                    "Continue research from the archived milestone: {}\n\n{}\n\nRead the \
                     retained materials when relevant: .wisp/research-archives/{}/REPORT.md\n\
                     Original notebook: {}. Preserve the original record; record later \
                     corrections as new findings.",
                    archive.title, archive.report, archive.id, archive.frame_id
                )),
            )
            .await?;
        Ok(id)
    }

    /// The conversation's research archive draft that was not frozen yet,
    /// to go on reviewing it, as the WebView reopens one.
    pub async fn research_archive_draft(
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
            .filter(|archive| archive.frozen_at.is_none()))
    }

    /// Has the conversation's model write a draft archive of it, as the
    /// WebView's Archive research does: a report, the scripts that can be
    /// put together from what was run, and the local files the conversation
    /// recorded, each kept as a snapshot unless the model recommends
    /// deleting one that may be deleted. The draft is kept, in place of an
    /// earlier one, for the researcher to review and freeze; no file is
    /// touched.
    pub async fn draft_research_archive(
        &self,
        project: &str,
        session: &str,
    ) -> Result<ResearchArchive> {
        self.archivable(project, session).await?;
        let store = &self.0.store;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let (_, source_hash) = store.research_archive_source(session).await?;
        let mut files = wisp_store::research_archive_files::candidates(store, &root, session)
            .await
            .map_err(anyhow::Error::msg)?;
        let options = self.stored_options(session).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let routes = crate::network::load(store).await;
        crate::network::route_model(&mut model.proxy, &routes);
        // A draft is a document to write, not a problem to think about.
        model.reasoning_effort = None;
        let messages = self.whole_transcript(session).await?;
        let provider = (self.0.models)(model);
        let transcript = crate::review::transcript(&messages);
        let draft =
            match crate::research_archive::draft(provider.as_ref(), transcript, &files).await {
                Ok(draft) => draft,
                Err(error) => bail!(
                    "{}",
                    crate::network::explain(store, &format!("{error:#}")).await
                ),
            };
        wisp_store::research_archive_files::recommend_deletions(&mut files, &draft.delete_paths);
        let archive = ResearchArchive {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project.to_owned(),
            frame_id: session.to_owned(),
            source_hash,
            title: draft.title,
            report: draft.report,
            scripts: draft.scripts,
            files,
            created_at: chrono::Utc::now().timestamp(),
            frozen_at: None,
            warnings: wisp_store::research_archive_files::DRAFT_WARNINGS
                .map(String::from)
                .to_vec(),
        };
        // What was said while the model wrote is not in the draft.
        self.archivable(project, session).await?;
        ensure!(
            store.research_archive_source(session).await?.1 == archive.source_hash,
            "The notebook changed while preparing the draft. Generate it again."
        );
        store.save_research_archive_draft(&archive).await?;
        Ok(archive)
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
        crate::network::route_model(&mut model.proxy, &routes);
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

    /// Freezes the conversation's kept draft as the researcher reviewed it
    /// in `input`, as the WebView's Archive and clean up does: snapshots,
    /// scripts, the notebook and the report are written under
    /// `.wisp/research-archives/<id>/`, the record is frozen and the
    /// conversation is read-only, and then the files chosen for deletion
    /// are deleted for good, each with a receipt. Refused, with every file
    /// as it was, when the conversation or a listed file changed since the
    /// draft, or a choice is not one the draft offered. A deletion that
    /// fails after the freeze leaves the archive frozen with its receipt;
    /// [`Self::retry_research_archive_cleanup`] tries it again.
    pub async fn freeze_research_archive(
        &self,
        project: &str,
        session: &str,
        input: ConfirmResearchArchive,
    ) -> Result<ResearchArchive> {
        let _admission = self.0.admission.write().await;
        self.archivable(project, session).await?;
        let store = &self.0.store;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        let draft = store
            .research_archive(session)
            .await?
            .filter(|archive| archive.frozen_at.is_none())
            .context("Prepare an archive draft first")?;
        let result =
            wisp_store::research_archive_files::freeze_files(store, &root, draft, input).await;
        // Even a failed receipt leaves the conversation frozen.
        if self.archived(session).await? {
            // What the conversation kept running is no longer needed.
            let runtime = self.session(session).runtime.lock().unwrap().take();
            if let Some(runtime) = runtime {
                runtime.shutdown_all().await;
            }
        }
        result.map_err(anyhow::Error::msg)
    }

    /// Deletes again the files a frozen archive chose to delete that were
    /// not deleted, as the WebView's Retry incomplete cleanup does. A file
    /// that changed or is used elsewhere since is kept.
    pub async fn retry_research_archive_cleanup(
        &self,
        project: &str,
        session: &str,
    ) -> Result<ResearchArchive> {
        let _admission = self.0.admission.write().await;
        let archive = self
            .research_archive(project, session)
            .await?
            .context("Archive is not frozen")?;
        let root = std::path::PathBuf::from(self.project_root(project).await?);
        wisp_store::research_archive_files::cleanup_files(&self.0.store, &root, &archive)
            .await
            .map_err(anyhow::Error::msg)
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

    /// Reruns one run of a finalized version in a clean temporary folder
    /// with only its frozen inputs and code, as the WebView's "Verify in
    /// clean workspace" does, and records how its outputs compare. Runs
    /// through the host's run runner; only local runs can be rerun.
    pub async fn verify_paper_run(&self, project: &str, version: &str, run: &str) -> Result<()> {
        self.verify_paper_run_with(project, version, run, self.0.runs.runner_ref())
            .await
    }

    pub(crate) async fn verify_paper_run_with(
        &self,
        project: &str,
        version: &str,
        run: &str,
        runner: &dyn wisp_runs::RunCommandRunner,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::verify(&self.0.store, project, version, run, runner).await
    }

    /// Saves the evidence capsule of a finalized version as a new `.zip`
    /// at `destination`, which the person chose; the attempt is recorded
    /// with the version either way.
    pub async fn build_paper_capsule(
        &self,
        project: &str,
        version: &str,
        destination: &std::path::Path,
    ) -> Result<()> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        crate::papers::build_capsule(&self.0.store, project, version, destination).await
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

    /// Downloads a plugin's `.zip` from an HTTPS address, on the connector
    /// route, and installs it once its SHA-256 is the one given.
    pub async fn install_plugin_from_address(
        &self,
        address: &str,
        sha256: &str,
    ) -> Result<PluginRow> {
        let _admission = self.0.admission.read().await;
        let routes = crate::network::load(&self.0.store).await;
        let mut client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                // HTTPS all the way, and not round in circles.
                if attempt.url().scheme() == "https" && attempt.previous().len() < 6 {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }));
        match routes.mcp_proxy_url.trim() {
            "" => {}
            "none" => client = client.no_proxy(),
            proxy => client = client.proxy(reqwest::Proxy::all(proxy)?),
        }
        crate::plugins::install_from_address(
            &self.0.store,
            &self.0.config.data,
            address,
            sha256,
            client.build()?,
        )
        .await
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

    /// Which projects are hidden while recording, and whether that is on:
    /// the WebView's privacy mode, kept where both desktops read it.
    pub async fn privacy_mode(&self) -> Result<wisp_dto::project_browser::PrivacyMode> {
        self.0.store.project_privacy().await
    }

    /// Hides `projects` while recording, or with `active` off shows all
    /// again. Only projects that are here can be hidden.
    pub async fn set_privacy_mode(&self, active: bool, projects: Vec<String>) -> Result<()> {
        let known = self.0.store.list_projects().await?;
        let mut kept: Vec<String> = projects
            .into_iter()
            .filter(|id| known.iter().any(|project| &project.0 == id))
            .collect();
        kept.sort();
        kept.dedup();
        ensure!(
            !active || !kept.is_empty(),
            "Choose the projects to hide while recording"
        );
        self.0
            .store
            .write_project_privacy(
                wisp_dto::project_browser::PrivacyMode {
                    active,
                    project_ids: kept,
                },
                false,
            )
            .await
    }

    /// Whether each project made from now on keeps its own database in its
    /// folder, the WebView's decentralized project storage. Projects that
    /// exist stay where they are.
    pub async fn separate_project_storage(&self) -> Result<bool> {
        self.0.store.decentralized_project_storage().await
    }

    pub async fn set_separate_project_storage(&self, on: bool) -> Result<()> {
        self.0
            .store
            .set_setting("decentralized_project_storage", &on.to_string())
            .await
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

    /// Starts a quick action's workflow as agents in `project`, on what
    /// was typed: a new conversation holds the drafted batch, as Run as
    /// agents makes it, with the action's request and `input` told to
    /// every task.
    pub async fn start_quick_action(
        &self,
        project: &str,
        id: &str,
        input: &str,
    ) -> Result<crate::workflows::DraftedWorkflow> {
        let action = crate::quick_actions::list(&self.0.store)
            .await
            .into_iter()
            .find(|action| action.id == id)
            .context("That action is no longer there")?;
        ensure!(
            !action.workflow.is_empty(),
            "This action puts a request into the composer; it starts no workflow"
        );
        let told = [action.prompt.trim(), input.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        self.draft_workflow_on(project, &action.workflow, &told)
            .await
    }

    /// The experts a conversation can be started as.
    pub async fn experts(&self) -> Result<Vec<Expert>> {
        Ok(crate::experts::list(&self.0.store).await)
    }

    /// Adds an expert, or changes the one `input.id` names. One that ships
    /// with Wisp keeps its instructions.
    pub async fn save_expert(&self, input: ExpertInput) -> Result<Vec<Expert>> {
        let model = input.model.trim();
        ensure!(
            model.is_empty()
                || self
                    .0
                    .model
                    .read()
                    .unwrap()
                    .settings()
                    .profiles
                    .iter()
                    .any(|profile| profile.id == model),
            "That model was removed; choose another"
        );
        crate::experts::upsert(&self.0.store, input).await
    }

    /// The model profile a conversation's turns run on: the expert's own
    /// when the conversation was started as an expert that has one still
    /// saved, as the WebView runs its specialists, or else the
    /// conversation's choice (empty is the default).
    pub(crate) async fn turn_profile(&self, options: &ConversationOptions) -> String {
        if !options.expert.is_empty() {
            if let Some(expert) = crate::experts::get(&self.0.store, &options.expert).await {
                let saved = self
                    .0
                    .model
                    .read()
                    .unwrap()
                    .settings()
                    .profiles
                    .iter()
                    .any(|profile| profile.id == expert.model);
                if !expert.model.is_empty() && saved {
                    return expert.model;
                }
            }
        }
        options.model.clone()
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

    /// The model profile that describes pictures for a conversation whose
    /// model cannot see them; empty is the first saved one that can.
    pub async fn picture_model(&self) -> Result<String> {
        Ok(self
            .0
            .store
            .get_setting(PICTURE_MODEL)
            .await?
            .unwrap_or_default())
    }

    /// Chooses that profile, or with an empty id goes back to the first
    /// saved one that can see. One that cannot see is refused.
    pub async fn set_picture_model(&self, profile: &str) -> Result<String> {
        let store = &self.0.store;
        if profile.is_empty() {
            store.delete_setting(PICTURE_MODEL).await?;
        } else {
            let found = self
                .0
                .model
                .read()
                .unwrap()
                .settings()
                .profiles
                .into_iter()
                .find(|saved| saved.id == profile);
            let found = found.context("That model was removed; choose another")?;
            ensure!(
                found.sees,
                "That model cannot read pictures; choose one that can"
            );
            store.set_setting(PICTURE_MODEL, profile).await?;
        }
        self.picture_model().await
    }

    /// The model that describes the pictures attached for a conversation
    /// whose model cannot see them, if any saved model can.
    pub(crate) async fn picture_reader(&self) -> Option<ModelConfig> {
        let chosen = self.picture_model().await.unwrap_or_default();
        self.0.model.read().unwrap().picture_reader(chosen.trim())
    }

    /// The model a conversation's sub-agents work on: the conversation's
    /// own choice for them when it has one, else the one chosen in
    /// Settings, else `own`, the conversation's model. A choice that names
    /// a profile which is gone or cannot be used falls through to the next.
    pub(crate) async fn subagent_model_for(&self, session: &str, own: &ModelConfig) -> ModelConfig {
        let store = &self.0.store;
        let theirs = store.frame_subagent_model(session).await.ok().flatten();
        // The conversation chose its own model for them, over Settings.
        if theirs.as_deref().is_some_and(|id| id.trim().is_empty()) {
            return own.clone();
        }
        let chosen = [
            theirs,
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
        crate::network::route_model(&mut model.proxy, &routes);
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
        self.ensure_not_frozen_branch(session).await?;
        let running = self.session(session);
        let live = running.live.lock().unwrap();
        // A timer's turn takes no guidance, as in the WebView: what is said
        // then is a turn of its own, after it, that the timer never takes.
        if !live.running || !live.guidable || live.timer_open {
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
        let subagent = options.subagent_model.as_deref().map(str::trim);
        if let Some(id) = subagent.filter(|id| !id.is_empty()) {
            ensure!(
                self.0
                    .model
                    .read()
                    .unwrap()
                    .settings()
                    .profiles
                    .iter()
                    .any(|profile| profile.id == id),
                "That model was removed; choose another"
            );
        }
        let saved = self.0.store.frame_subagent_model(session).await?;
        if saved.as_deref().map(str::trim) != subagent {
            self.0
                .store
                .set_frame_subagent_model(session, project, subagent)
                .await?;
        }
        // Written only when it changes, so a tier the WebView saved for
        // another reason is left as it was.
        let tier = self.0.store.frame_service_tier(session).await?;
        if options.fast != is_fast(tier.as_deref()) {
            self.0
                .store
                .set_frame_service_tier(session, project, options.fast.then_some(FAST_TIER))
                .await?;
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
        if remember == Remember::Everywhere {
            crate::runtime::set_global_grant(&self.0.store, &tool, true).await?;
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

    /// Tools allowed in every project, sorted.
    pub async fn global_grants(&self) -> Result<Vec<String>> {
        Ok(crate::runtime::global_grants(&self.0.store).await)
    }

    /// Takes back "always allow" for a tool in every project. Turns that
    /// start afterwards ask again.
    pub async fn revoke_global_grant(&self, tool: &str) -> Result<()> {
        ensure!(
            crate::runtime::set_global_grant(&self.0.store, tool, false).await?,
            "That tool is not allowed everywhere any more"
        );
        Ok(())
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
            auto_continue: turns.auto_continue,
            auto_continue_limit: turns.auto_continue_limit,
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
    /// it: the conversation's model first classifies the question, as the
    /// WebView's side chat does, then reads its latest turns, the earlier
    /// passages its scope looks up and `earlier`, the side exchanges so
    /// far, and has no tools. It can be asked while a turn runs, and reads
    /// what has been saved by then.
    pub async fn ask_aside(
        &self,
        project: &str,
        session: &str,
        earlier: &[AsideExchange],
        question: &str,
    ) -> Result<AsideAnswer> {
        self.ask_aside_about(project, session, None, earlier, question)
            .await
    }

    /// A question aside about one of the project's runs: as [`ask_aside`],
    /// with the run's live snapshot (status, timing, exit code, command,
    /// progress and its stored output tails) as the newest source, as the
    /// WebView asks from a run card.
    ///
    /// [`ask_aside`]: Self::ask_aside
    pub async fn ask_aside_about_run(
        &self,
        project: &str,
        session: &str,
        run: &str,
        earlier: &[AsideExchange],
        question: &str,
    ) -> Result<AsideAnswer> {
        self.ask_aside_about(project, session, Some(run), earlier, question)
            .await
    }

    async fn ask_aside_about(
        &self,
        project: &str,
        session: &str,
        run: Option<&str>,
        earlier: &[AsideExchange],
        question: &str,
    ) -> Result<AsideAnswer> {
        self.own_conversation(project, session).await?;
        // A question that would be refused is refused before the model is
        // asked anything about it.
        let question = crate::aside::checked_question(question)?;
        let snapshot = match run {
            None => None,
            Some(run) => {
                let record = self.0.store.get_run(run).await?.context("Run not found")?;
                ensure!(
                    record.project_id == project,
                    "Run does not belong to the selected project"
                );
                Some(crate::aside::run_snapshot(
                    &record,
                    chrono::Utc::now().timestamp(),
                ))
            }
        };
        let store = &self.0.store;
        let options = self.stored_options(session).await?;
        let mut model = self.0.model.read().unwrap().pick(&options.model)?.provider;
        // The request goes the way a turn would.
        let routes = crate::network::load(store).await;
        crate::network::route_model(&mut model.proxy, &routes);
        let messages = store
            .load_recent_turn_preview_messages(session, crate::aside::TURNS)
            .await?;
        let provider = (self.0.models)(model);
        // As the WebView's side chat does, the model first says what kind of
        // question it is, unless it is about a run, whose scope the card
        // chose. One it cannot classify is answered on a best-effort reading.
        let intent = match snapshot {
            Some(_) => None,
            None => Some(
                match wisp_app::side_question::classify(
                    provider.as_ref(),
                    question,
                    wisp_app::side_question::INTENT_TIMEOUT,
                )
                .await
                {
                    Ok(intent) => intent,
                    Err(reason) => {
                        tracing::warn!(
                            reason,
                            "Aside classification unavailable; using recent conversation"
                        );
                        wisp_app::side_question::Intent::fallback(crate::aside::terms(question))
                    }
                },
            ),
        };
        let wanted = match &intent {
            Some(intent) => crate::aside::lookup_terms(question, intent),
            None => crate::aside::terms(question),
        };
        // Passages from before the turns quoted whole that match the
        // question go along, labelled, as the WebView's evidence does.
        let history = store
            .load_recent_turn_preview_messages(session, crate::aside::HISTORY_TURNS)
            .await?;
        let before = history.len().saturating_sub(messages.len());
        let sources = crate::aside::sources(&wanted, &history[..before]);
        match crate::aside::ask(
            provider.as_ref(),
            &messages,
            snapshot.as_deref(),
            intent.map(|intent| intent.scope),
            &sources,
            earlier,
            question,
        )
        .await
        {
            Ok(text) => Ok(AsideAnswer { text, sources }),
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
        let mut view = crate::model_view::fold(&messages);
        view.epoch = self.0.store.frame_head_epoch(session).await?;
        // And what the latest request was made of, tools included, while
        // this host has it.
        view.sent = self.session(session).live.lock().unwrap().context.clone();
        Ok(view)
    }

    /// Has the conversation's model review the conversation as a
    /// reviewer: where a claim is not borne out by the tool output it came
    /// from. Nothing is added to the conversation; the report is kept with
    /// it, after what was read. It waits for a running turn to end,
    /// because half a turn cannot be judged.
    pub async fn review_conversation(&self, project: &str, session: &str) -> Result<Review> {
        self.own_conversation(project, session).await?;
        self.ensure_branch_changeable(session).await?;
        let messages = self.numbered_transcript(session).await?;
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
        self.ensure_branch_changeable(session).await?;
        let (_, next) = self.locate_turn(session, turn).await?;
        let mut messages = self.numbered_transcript(session).await?;
        if let Some(next) = next {
            messages.retain(|(seq, _)| *seq < next);
        }
        self.review_messages(session, messages).await
    }

    /// Keeps a review with the conversation, after the last message it
    /// read, in the WebView's shape, so the transcript shows it again. A
    /// review that cannot be kept is still shown now.
    pub(crate) async fn keep_review(&self, session: &str, through: i64, review: &Review) {
        let id = format!("review-{}", uuid::Uuid::new_v4().simple());
        if let Err(error) = self
            .0
            .store
            .upsert_session_review(session, &id, through, &review.report_json(&id))
            .await
        {
            tracing::warn!(%error, "Could not keep a review with its conversation");
        }
    }

    async fn review_messages(
        &self,
        session: &str,
        numbered: Vec<(i64, wisp_llm::Message)>,
    ) -> Result<Review> {
        let through = numbered.last().map_or(0, |(seq, _)| *seq);
        let messages: Vec<_> = numbered.into_iter().map(|(_, message)| message).collect();
        ensure!(
            !self.session(session).live.lock().unwrap().running,
            "Wait for the turn to end before you ask for a review"
        );
        let store = &self.0.store;
        let options = self.stored_options(session).await?;
        let own = self.0.model.read().unwrap().pick(&options.model)?.provider;
        let provider = self.reviewer(own).await;
        match crate::review::ask(provider.as_ref(), &messages).await {
            Ok(review) => {
                self.keep_review(session, through, &review).await;
                Ok(review)
            }
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
                    if !live.suggesting && !live.reviewing && !live.proposing {
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
        use crate::runtime::{
            AUTO_COMPACT, AUTO_COMPACT_PERCENT, AUTO_CONTINUE, AUTO_CONTINUE_LIMIT,
            AUTO_CONTINUE_RANGE, ITERATION_RANGE, MAX_ITERATIONS,
        };
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
        ensure!(
            AUTO_CONTINUE_RANGE.contains(&defaults.auto_continue_limit),
            "A turn carries on by itself between {} and {} times",
            AUTO_CONTINUE_RANGE.start(),
            AUTO_CONTINUE_RANGE.end()
        );
        let store = &self.0.store;
        store
            .set_setting(MAX_ITERATIONS, &defaults.max_iterations.to_string())
            .await?;
        store
            .set_setting(AUTO_CONTINUE, &defaults.auto_continue.to_string())
            .await?;
        store
            .set_setting(
                AUTO_CONTINUE_LIMIT,
                &defaults.auto_continue_limit.to_string(),
            )
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
        self.ensure_not_exploring(session, "delete").await?;
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

/// A file one turn of a conversation recorded as an artifact, for the
/// answer that closes the turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyArtifact {
    /// The turn's last tool call: the file belongs under the answer that
    /// closes the turn this call is in.
    pub call_id: String,
    pub artifact: String,
    /// The version this turn recorded.
    pub version: String,
    pub name: String,
    /// MIME type.
    pub kind: String,
    /// Where the file is in the project, when it is a project file.
    pub path: Option<String>,
    /// A later turn of the conversation wrote the file again.
    pub superseded: bool,
}

/// Marks each file a later turn of the same conversation wrote again;
/// `rows` are in the order the turns recorded them.
pub(crate) fn mark_superseded(mut rows: Vec<ReplyArtifact>) -> Vec<ReplyArtifact> {
    for at in 0..rows.len() {
        let artifact = &rows[at].artifact;
        let again = rows[at + 1..]
            .iter()
            .any(|later| later.artifact == *artifact && later.call_id != rows[at].call_id);
        rows[at].superseded = again;
    }
    rows
}

/// Where an output in the research journey came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JourneySource {
    /// The producing run and what it read; empty when no run is recorded.
    pub source: wisp_dto::ResearchJourneySource,
    /// Where the output is in the project, when it is a project file.
    pub path: Option<String>,
    /// The same for each of `source.inputs`, in their order.
    pub input_paths: Vec<Option<String>>,
}

/// A run as the research journey's run record shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JourneyRun {
    pub id: String,
    pub title: String,
    /// `submitted`, `started`, `succeeded`, `failed`, `cancelled`, `lost`.
    pub status: String,
    /// The execution context it ran on.
    pub context: String,
    pub exit_code: Option<i64>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub command: Option<String>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
}

/// The project path an artifact's logical key names, if it names one.
fn project_path(logical_key: Option<&str>) -> Option<String> {
    logical_key?
        .strip_prefix("path:")
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

/// One kept version of an artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactVersionRow {
    pub id: String,
    /// From 1, the first time the file was written.
    pub number: i64,
    /// MIME type.
    pub kind: String,
    pub size_bytes: Option<u64>,
    /// Unix seconds.
    pub created_at: i64,
}

/// Reasoning efforts a conversation may ask for. Empty is the model's default.
pub const EFFORTS: &[&str] = &[
    "", "none", "minimal", "low", "medium", "high", "xhigh", "max",
];

/// A subscription and who is signed in to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriptionRow {
    pub subscription: Subscription,
    pub account: Option<String>,
}

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
    /// Ask for OpenAI's faster priority processing, which costs more. Only
    /// a model whose service offers it ([`fast_available`]) is asked.
    pub fast: bool,
    /// The model this conversation's sub-agents work on, as the WebView
    /// keeps it: `None` follows Settings, an empty id this conversation's
    /// own model, any other the saved model it names.
    pub subagent_model: Option<String>,
}

/// The version of Wisp that is running, as releases are numbered.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The project the host was started on, which is always there.
pub const BUILT_IN_PROJECT: &str = crate::PROJECT;

/// The service tier fast mode asks for, as the WebView desktop saves it.
pub const FAST_TIER: &str = "priority";

/// Whether a model's service offers priority processing: OpenAI's own
/// API, its Responses API and the ChatGPT sign-ins. Another service that
/// speaks OpenAI's protocol is not sent a field it may refuse.
pub fn fast_available(provider: &wisp_llm::ProviderConfig) -> bool {
    use wisp_llm::ProviderKind as Kind;
    match provider.kind {
        Kind::OpenAiResponses | Kind::OpenAiCodex | Kind::OpenAiChatGpt => true,
        Kind::OpenAiCompatible => provider.base_url.contains("://api.openai.com"),
        Kind::Anthropic => false,
    }
}

/// Whether a conversation's saved service tier is fast mode. The WebView
/// saves an empty value to turn a model's default off.
fn is_fast(tier: Option<&str>) -> bool {
    matches!(tier.map(str::trim), Some("priority" | "fast"))
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
    /// Whether the conversation's model offers fast mode.
    pub fast_available: bool,
    /// A memory drafted after the latest turn, until it is saved, put
    /// away or the next turn starts.
    pub memory_proposal: Option<MemoryProposal>,
    /// Messages waiting for the turn to end, first to go first.
    pub queued: Vec<QueuedMessage>,
    /// Your command hooks that failed in the latest turn, or a project's
    /// hooks file that was not run.
    pub hook_failures: Vec<String>,
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
    /// Its process, once it has one.
    pub process_id: Option<u32>,
    /// Counts the interpreters started for this conversation and language,
    /// from 1; a restart is the next one.
    pub generation: u64,
    /// When it started and when it last ran code, in Unix milliseconds.
    pub started_at_ms: u64,
    pub last_activity_at_ms: u64,
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
            process_id: info.process_id,
            generation: info.generation,
            started_at_ms: info.started_at_ms,
            last_activity_at_ms: info.last_activity_at_ms,
        }
    }
}

/// An SSH host as Settings shows and edits it. Signing in is by key (or
/// whatever the user's SSH config and agent provide) or by a password kept
/// in the OS keyring, never in the database.
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
    /// Signs in with a password instead of a key.
    pub password_sign_in: bool,
    /// A password is saved for it. Read only.
    pub has_password: bool,
    /// A new password to save; none keeps the saved one. Never listed.
    pub password: Option<String>,
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
            password_sign_in: host.auth_method.as_deref() == Some("password"),
            has_password: false,
            password: None,
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
            auth_method: self.password_sign_in.then(|| "password".to_owned()),
            has_password: false,
            // Kept in the keyring by the host, never with the host here.
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

/// One executable of the local environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolPath {
    /// The key it is kept under, such as `python_executable`.
    pub key: &'static str,
    pub label: &'static str,
    /// The path saved for it; empty when it is looked for on the search path.
    pub saved: String,
    /// Where it is on the search path, if it is.
    pub found: Option<String>,
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
    /// When the run manager cleans up after the project's runs on servers.
    pub retention: RunRetention,
}

/// How many days a project's finished runs and unused server files are
/// kept before the run manager cleans them up, as the WebView's Project
/// settings keeps them. `None` keeps them until they are cleaned up by hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunRetention {
    /// A succeeded run's working folder, once its results were brought back.
    pub succeeded: Option<i64>,
    /// A failed or cancelled run's working folder.
    pub failed: Option<i64>,
    /// Files uploaded to or kept on a server that nothing refers to any more.
    pub orphaned: Option<i64>,
}

/// The longest a cleanup window can be, in days.
pub const RETENTION_DAYS_LIMIT: i64 = 3_650;

pub const PROJECT_NAME_LIMIT: usize = 120;
pub const PROJECT_DESCRIPTION_LIMIT: usize = 2_000;
pub const PROJECT_RULES_LIMIT: usize = 20_000;

/// The folder a new project called `name` is made in: its name with the
/// characters no system allows in a folder name made dashes, at most 80
/// characters, and no leading or trailing dots or spaces. Empty when
/// nothing is left.
pub fn project_folder_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let joined: String = words.join(" ").chars().take(80).collect();
    joined.trim().trim_matches('.').trim().to_owned()
}

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
    /// The variables and headers it names with no value saved, as one
    /// brought over from the WebView whose value the keychain would not
    /// give: each is entered again before it is sent.
    pub unset: Vec<String>,
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
    /// The environment variables a program is started with, as name and
    /// value; an empty value keeps the saved one, a name left out is
    /// forgotten, and `None` leaves them all as they are.
    pub env: Option<Vec<(String, String)>>,
    /// The same for the headers sent to an HTTP server.
    pub headers: Option<Vec<(String, String)>>,
    /// An HTTP server's own route; `None` leaves it as it is.
    pub proxy: Option<String>,
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
    ServiceKey {
        id: "infinisynapse_api_key",
        env: "INFINISYNAPSE_API_KEY",
        label: "InfiniSynapse API key",
        note: "The agent_infini CLI's data analysis over databases and knowledge bases. Saving it runs agent_infini init, which must be installed on this computer.",
        secret: true,
    },
    ServiceKey {
        id: "scimaster_api_key",
        env: "SCIMASTER_API_KEY",
        label: "SCIMaster API key",
        note: "Literature search with the sci CLI and the bear-* research skills. It is also written into ~/.scimaster/config.json, where sci reads it.",
        secret: true,
    },
];

/// The SciMaster CLI's settings with `key` as its API key, or without one:
/// what else is in them stays, and the CLI's defaults are filled in.
pub(crate) fn scimaster_config(raw: Option<&str>, key: Option<&str>) -> Result<String> {
    let mut root = raw
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let settings = root
        .as_object_mut()
        .context("SciMaster settings are an object")?;
    settings
        .entry("version")
        .or_insert_with(|| serde_json::json!(1));
    settings
        .entry("apiBaseUrl")
        .or_insert_with(|| serde_json::json!("https://scimaster.bohrium.com"));
    let defaults = settings
        .entry("defaults")
        .or_insert_with(|| serde_json::json!({}));
    if !defaults.is_object() {
        *defaults = serde_json::json!({});
    }
    if let Some(defaults) = defaults.as_object_mut() {
        defaults
            .entry("limit")
            .or_insert_with(|| serde_json::json!(10));
        defaults
            .entry("mode")
            .or_insert_with(|| serde_json::json!("low"));
    }
    match key.map(str::trim).filter(|key| !key.is_empty()) {
        Some(key) => settings.insert("apiKey".into(), serde_json::json!(key)),
        None => settings.remove("apiKey"),
    };
    Ok(serde_json::to_string_pretty(&root)?)
}

/// Writes the SciMaster CLI's settings file under `home` with `key`, or
/// takes the key out; nothing is made when there is no key and no file.
fn sync_scimaster(home: &std::path::Path, key: &str) -> Result<()> {
    let path = home.join(".scimaster").join("config.json");
    if key.is_empty() && !path.is_file() {
        return Ok(());
    }
    let current = std::fs::read_to_string(&path).ok();
    let merged = scimaster_config(current.as_deref(), Some(key))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(&path, merged).with_context(|| format!("Could not write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Runs `agent_infini init` with the key, as the WebView does when the key
/// is saved. The CLI is looked for in `home/.infini/bin` and, when
/// `search_path`, on the PATH.
async fn init_agent_infini(home: &std::path::Path, search_path: bool, key: &str) -> Result<()> {
    let exe = if cfg!(windows) {
        "agent_infini.exe"
    } else {
        "agent_infini"
    };
    let on_path = std::env::var_os("PATH")
        .filter(|_| search_path)
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(exe))
                .collect()
        })
        .unwrap_or_else(Vec::new);
    let Some(program) = on_path
        .into_iter()
        .chain([home.join(".infini").join("bin").join(exe)])
        .find(|path| path.is_file())
    else {
        let install = if cfg!(windows) {
            "irm https://infinisynapse.cn/cli-install/install.ps1 | iex"
        } else {
            "curl -fsSL https://infinisynapse.cn/cli-install/install.sh | bash"
        };
        bail!("agent_infini was not found. Install it with: {install}");
    };
    let mut command = tokio::process::Command::new(&program);
    command.arg("init").arg("--api-key").arg(key);
    wisp_tools::process::hide_console_async(&mut command);
    let out = command
        .output()
        .await
        .with_context(|| format!("Could not run {}", program.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let said = [out.stderr, out.stdout]
        .iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        // The key is never repeated back.
        .replace(key, "<redacted>");
    match said.is_empty() {
        true => bail!("agent_infini init failed with {}", out.status),
        false => bail!("agent_infini init failed: {said}"),
    }
}

/// Where the WebView keeps the list of credentials of your own.
const CUSTOM_CREDENTIALS: &str = "custom_credentials";

/// One credential of your own as the WebView keeps it; the value is in the
/// keychain under [`custom_secret`].
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CustomRow {
    id: String,
    name: String,
    env_var: String,
}

fn custom_secret(id: &str) -> String {
    format!("custom_credential:{id}")
}

fn valid_env_var(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('_' | 'A'..='Z' | 'a'..='z'))
        && chars.all(|ch| matches!(ch, '_' | 'A'..='Z' | 'a'..='z' | '0'..='9'))
}

/// The saved list, keeping only well-formed rows with one variable each,
/// as the WebView reads it.
async fn custom_rows(store: &wisp_store::Store) -> Vec<CustomRow> {
    let saved = store
        .get_setting(CUSTOM_CREDENTIALS)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let mut seen = HashSet::new();
    serde_json::from_str::<Vec<CustomRow>>(&saved)
        .unwrap_or_default()
        .into_iter()
        .filter(|row| {
            uuid::Uuid::parse_str(&row.id).is_ok()
                && !row.name.trim().is_empty()
                && valid_env_var(&row.env_var)
                && seen.insert(row.env_var.to_ascii_uppercase())
        })
        .collect()
}

/// A credential of your own, as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomCredential {
    pub id: String,
    pub name: String,
    pub env_var: String,
    pub saved: bool,
}

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
    /// Skills used and MCP tools called, most calls first, as the
    /// WebView's usage ranking counts them.
    pub tools: Vec<ToolUsage>,
}

/// A skill or an MCP server's tool, and how many times it was called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolUsage {
    /// `skill` or `mcp`.
    pub kind: String,
    pub name: String,
    pub calls: i64,
}

/// The skills used (`use_skill` with the skill it named) and the tools of
/// MCP servers called (named `server/tool`), from how often each tool was
/// called; this host's own tools are left out. Most calls first.
pub(crate) fn rank_tool_calls(
    counts: Vec<(String, Option<String>, i64)>,
    servers: &[crate::mcp::McpServer],
) -> Vec<ToolUsage> {
    let mut ranked: std::collections::BTreeMap<(String, String), i64> = Default::default();
    for (tool, skill, calls) in counts {
        let named = if tool == "use_skill" {
            skill.map(|skill| ("skill".to_owned(), skill))
        } else {
            servers.iter().find_map(|server| {
                tool.strip_prefix(&format!("{}__", server.slug()))
                    .map(|rest| ("mcp".to_owned(), format!("{}/{rest}", server.name.trim())))
            })
        };
        if let Some(key) = named {
            *ranked.entry(key).or_default() += calls;
        }
    }
    let mut ranked: Vec<ToolUsage> = ranked
        .into_iter()
        .map(|((kind, name), calls)| ToolUsage { kind, name, calls })
        .collect();
    ranked.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    ranked
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
    /// The whole project folder, the researcher's files included, as the
    /// WebView's Workspace files counts it; `workspace_more` when the
    /// folder was too large to walk to its end.
    pub workspace_bytes: u64,
    pub workspace_more: bool,
    /// The folder is not there (an unplugged disk, a moved folder).
    pub missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageView {
    pub data_dir: String,
    /// Conversations, settings and run records of every project.
    pub database_bytes: u64,
    pub favorites_bytes: u64,
    /// Python, R and Node.js installed into the data folder.
    pub tools_bytes: u64,
    /// Installed plugins.
    pub plugins_bytes: u64,
    /// The whole data folder; what the parts above leave is the rest.
    pub data_bytes: u64,
    pub projects: Vec<ProjectStorage>,
}

impl StorageView {
    /// What the data folder holds besides the parts listed.
    pub fn other_bytes(&self) -> u64 {
        self.data_bytes.saturating_sub(
            self.database_bytes + self.favorites_bytes + self.tools_bytes + self.plugins_bytes,
        )
    }
}

/// Entries one folder measurement may look at.
const SIZE_BUDGET: usize = 50_000;

/// The size of the regular files under `root`, links not followed. A very
/// large folder is measured up to a bound rather than for minutes.
fn folder_size(root: &std::path::Path) -> u64 {
    folder_size_bounded(root).0
}

/// [`folder_size`], and whether the folder holds more than was looked at.
fn folder_size_bounded(root: &std::path::Path) -> (u64, bool) {
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
                return (total, true);
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
    (total, false)
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
    /// In every project, until it is taken back. Kept in the WebView's
    /// global grants.
    Everywhere,
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
pub use crate::acp_turn::{AcpChoice, AcpCommand, AcpControls, AcpPick, AcpSetting, AcpValue};
pub use crate::approvals::{ApprovalScope, ProjectApprovals, ToolRule};
pub use crate::aside::Exchange as AsideExchange;
pub use crate::aside::{Answer as AsideAnswer, Source as AsideSource};
pub use crate::assistant::{is_assistant, ASSISTANT_FRAME_ID};
pub use crate::background_runs::BackgroundRunEnded;
pub use crate::browser::{BrowserAlerts, BrowserSettings, BrowserStatus};
pub use crate::browser_filters::{BrowserFilters, BrowserHostRule};
pub use crate::channels::{
    feishu::{FeishuBind, FeishuBindPoll, FeishuStatus},
    weixin::{QrMatrix, WeixinDestination, WeixinQr, WeixinScan, WeixinStatus},
    ChannelState,
};
pub use crate::compact::{CompactKind, Compacted};
pub use crate::delegation::{DelegatedBatch, DelegatedTask};
pub use crate::demos::{Demo, DemoInfo, DemoRole, DemoRow};
pub use crate::device_bridge::{DeviceBridge, DeviceBridgeState};
pub use crate::experts::{Expert, ExpertInput};
pub use crate::explorations::{FileChange, PromotionPreview, StartedExploration};
pub use crate::method_search::control::MethodSearchRunDetails;
pub use crate::method_search::prepare::{MethodSearchAuditReport, ProtectedFileAudit};
pub use crate::model_view::{ModelRowKind, ModelView, ModelViewRow, SentContext};
pub use crate::origin::FileOrigin;
pub use crate::papers::{
    Paper, PaperAudience, PaperCapsule, PaperCapsuleStatus, PaperCheck, PaperFinding, PaperInput,
    PaperItem, PaperItemInput, PaperItemKind, PaperLevel, PaperPolicy, PaperRerun,
    PaperRerunOutput, PaperRerunStatus, PaperRow, PaperSource, PaperSourceChoice, PaperSourceKind,
    PaperVersion, PaperVersionState,
};
pub use crate::pet::{Pet, PetLook, SHEET_HEIGHT, SHEET_WIDTH};
pub use crate::plugins::PluginRow;
pub use crate::project_archive::ArchiveProgress;
pub use crate::webview_import::{
    WebViewData, WebViewGroup, WebViewHold, WebViewImport, WebViewModel, WebViewOutcome,
    WebViewProject, LIST_GROUPS as WEBVIEW_LIST_GROUPS, SETTING_GROUPS as WEBVIEW_SETTING_GROUPS,
};
/// The file name a version's evidence capsule is offered under when saving
/// it, from the paper's title and the version's id, as the WebView offers.
pub use wisp_runs::publication_capsule::capsule_filename as paper_capsule_file_name;

/// Where a project's work keeps things on one SSH host.
/// One of the project's runs on an SSH host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRun {
    pub id: String,
    /// The conversation that started it, when one did.
    pub session: Option<String>,
    pub title: String,
    pub kind: String,
    /// `submitted`, `running`, `succeeded`, `failed` and the rest.
    pub status: String,
    /// When it was made, and ended, in seconds since 1970.
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostStorage {
    /// Where uploaded data goes there: a whole path or one under `~/`.
    pub data_root: String,
    /// Where run folders are made there, under the home folder.
    pub workdir_root: String,
    /// The project folder results from there come back into.
    pub results_dir: String,
    /// Chosen and kept, rather than the defaults.
    pub saved: bool,
}
pub use crate::project_sync::{ProjectSyncInfo, ProjectSyncResult, SyncSettings};
pub use crate::queue::QueuedMessage;
pub use crate::quick_actions::{QuickAction, QuickActionInput};
pub use crate::remote_access::{RemoteAccess, RemoteState};
pub use crate::remote_dir::RemoteDirectory;
pub use crate::remote_file::{RemoteFile, RemoteFileBody};
pub use crate::review::{Finding as ReviewFinding, Outcome as ReviewOutcome, Review};
pub use crate::run_log::RunLog;
/// How many times a turn may carry on by itself after a cut answer.
pub use crate::runtime::AUTO_CONTINUE_RANGE;
pub use crate::runtimes::{RuntimeObject, RuntimeObjectList, RuntimeOutput};
pub use crate::schedules::{ScheduleInput, ScheduleRow, ScheduleRun, MIN_INTERVAL_SECS};
pub use crate::subscriptions::{
    CodexAccount, CodexUsage, SignInChallenge, SignInState, SignInStatus, Subscription, UsageWindow,
};
pub use crate::trajectory::TurnTokens;
pub use crate::turn_memory::{MemoryProposal, MemorySuggestion, ProposalTrigger};
pub use crate::turn_undo::TurnFiles;
pub use crate::updates::UpdateCheck;
pub use crate::workflows::{
    AgentSettings as WorkflowAgentSettings, DraftedWorkflow, Workflow, WorkflowApproval,
    WorkflowInput, WorkflowTask, CAPABILITIES as WORKFLOW_CAPABILITIES,
    TASK_LIMIT as WORKFLOW_TASK_LIMIT,
};
/// Which model a task's agent works on.
pub use wisp_core::ModelTier;
/// The most tasks a workflow run as agents takes.
pub use wisp_core::MAX_DELEGATION_TASKS as WORKFLOW_AGENT_LIMIT;
/// A working set's estimated tokens by kind, and what each kind holds.
pub use wisp_core::{ContextToolDetail, ContextUsage, ContextUsageDetails};
pub use wisp_dto::AutoFailureAnalysisSettings;
pub use wisp_dto::{
    ArchiveFile, ArchiveFileChoice, ArchiveScript, ConfirmResearchArchive, ResearchArchive,
};
pub use wisp_dto::{LabKnowledgeBaseInfo, LabMemberInfo, LabStatus};
/// A file the project placed on an SSH host, from its record there.
pub use wisp_runs::remote_files::{RemoteFileState as StagedState, RemoteFileView as StagedFile};
/// The checks the WebView makes on a research archive's title, report and
/// scripts before it is frozen, for a client to say what is wrong first.
pub use wisp_store::research_archive_files::validate_content as research_archive_problem;

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
    /// Carry on by itself when an answer is cut at the model's output
    /// limit, at most this many times a turn.
    pub auto_continue: bool,
    pub auto_continue_limit: usize,
    /// New conversations run tools that change things without asking.
    pub full_access: bool,
    /// After an answer, ask the model for questions that could come next.
    pub follow_ups: bool,
    /// Conversations that have not chosen have their turns reviewed.
    pub auto_review: bool,
}

const DEFAULT_FULL_ACCESS: &str = "desktop.conversation.default_full_access";
/// The WebView's key for the tags the researcher gave skills.
const SKILL_TAGS: &str = "skill_tags";

/// Tags trimmed, without empty ones or repeats, in order.
fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    tags.into_iter()
        .map(|tag| tag.trim().to_owned())
        .filter(|tag| !tag.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The researcher's tags, by skill.
async fn skill_tags(store: &wisp_store::Store) -> std::collections::BTreeMap<String, Vec<String>> {
    store
        .get_setting(SKILL_TAGS)
        .await
        .ok()
        .flatten()
        .and_then(|raw| {
            serde_json::from_str::<std::collections::BTreeMap<String, Vec<String>>>(&raw).ok()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|(name, tags)| (name, normalize_tags(tags)))
        .filter(|(_, tags)| !tags.is_empty())
        .collect()
}
/// The WebView desktop's key for the model sub-agents work on.
pub(crate) const SUBAGENT_MODEL: &str = "subagent_model_id";
/// The saved model that describes pictures for a model that cannot see.
const PICTURE_MODEL: &str = "desktop.picture_model";

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

/// The pixels of `region` of a picture (PNG, JPEG, GIF's first frame, WebP
/// or BMP) as a PNG, as the WebView's canvas crop turns a region into one.
fn crop_png(bytes: &[u8], region: wisp_dto::image_pins::ImageRegion) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let picture = image::load_from_memory(bytes)
        .context("The picture could not be read")?
        .into_rgba8();
    let (left, top, width, height) = region.pixels(picture.width(), picture.height());
    let cut = image::imageops::crop_imm(&picture, left, top, width, height).to_image();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(
        cut.as_raw(),
        cut.width(),
        cut.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png)
}

pub(crate) fn image_type(path: &std::path::Path) -> Option<&'static str> {
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

/// A program and its arguments for the system's terminal to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLaunch {
    pub program: String,
    pub args: Vec<String>,
}

/// One word for a POSIX shell, whatever it holds.
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

pub(crate) fn model_choice_key(session: &str) -> String {
    format!("desktop.conversation.model.{session}")
}

pub(crate) fn full_access_key(session: &str) -> String {
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
        let unknown = error(&custom, "k");
        assert!(unknown.contains("Supported providers"), "{unknown}");
        assert!(unknown.contains("openai_chatgpt"));
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
