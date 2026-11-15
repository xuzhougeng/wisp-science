//! Models used through a subscription instead of an API key: Sign in with
//! ChatGPT, ChatGPT through the Codex endpoint, and SuperGrok (xAI).
//!
//! Ported from the WebView host's `src-tauri/src/codex_login.rs` and the
//! subscription parts of its `models.rs`. The OAuth flows are `wisp-llm`'s;
//! the signed-in credentials are kept in the system keychain under the
//! same names, so a sign-in made in either desktop serves both. A
//! subscription model's access token is refreshed before each turn when it
//! is about to expire, and the rotated token is written back. Not ported:
//! the WebView's pool of several Codex accounts and its usage meter.

use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use wisp_llm::{
    chatgpt_auth::{self, ChatGptCredentials},
    codex_auth::{self, AuthorizationCallback, BrowserCallback, CodexCredentials},
    xai_auth::{self, XaiCredentials},
};

use crate::desktop::SecretStore;

/// A subscription a model can be used through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subscription {
    /// Sign in with ChatGPT: OpenAI's Responses API on the plan.
    ChatGpt,
    /// ChatGPT through the Codex endpoint (the older way).
    Codex,
    /// SuperGrok, through xAI.
    Xai,
}

impl Subscription {
    pub const ALL: [Subscription; 3] = [
        Subscription::ChatGpt,
        Subscription::Codex,
        Subscription::Xai,
    ];

    /// The provider name a model profile carries, the WebView's.
    pub fn provider(self) -> &'static str {
        match self {
            Subscription::ChatGpt => "openai_chatgpt",
            Subscription::Codex => "openai_codex",
            Subscription::Xai => "xai_oauth",
        }
    }

    pub fn of_provider(provider: &str) -> Option<Self> {
        match provider.trim() {
            "openai_chatgpt" | "chatgpt" => Some(Subscription::ChatGpt),
            "openai_codex" | "openai-codex" | "codex" => Some(Subscription::Codex),
            "xai_oauth" | "xai" => Some(Subscription::Xai),
            _ => None,
        }
    }

    fn secret(self) -> &'static str {
        match self {
            Subscription::ChatGpt => chatgpt_auth::SUBSCRIPTION_SECRET,
            Subscription::Codex => codex_auth::SUBSCRIPTION_SECRET,
            Subscription::Xai => xai_auth::SUBSCRIPTION_SECRET,
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Subscription::ChatGpt => chatgpt_auth::DEFAULT_BASE_URL,
            Subscription::Codex => codex_auth::DEFAULT_BASE_URL,
            Subscription::Xai => xai_auth::DEFAULT_BASE_URL,
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Subscription::ChatGpt => chatgpt_auth::DEFAULT_MODEL,
            Subscription::Codex => codex_auth::DEFAULT_MODEL,
            Subscription::Xai => xai_auth::DEFAULT_MODEL,
        }
    }

    /// The context window a new model of it is saved with.
    pub fn context_window(self) -> u64 {
        match self {
            Subscription::Xai => 256_000,
            _ => codex_auth::CONTEXT_WINDOW,
        }
    }

    /// Only the Codex flow can be finished on another device with a code;
    /// xAI offers nothing else.
    pub fn device_only(self) -> bool {
        self == Subscription::Xai
    }

    pub fn offers_device_code(self) -> bool {
        self != Subscription::ChatGpt
    }
}

/// Signed-in credentials of one subscription.
#[derive(Clone)]
pub(crate) enum Credentials {
    ChatGpt(ChatGptCredentials),
    Codex(CodexCredentials),
    Xai(XaiCredentials),
}

impl Credentials {
    pub(crate) fn subscription(&self) -> Subscription {
        match self {
            Credentials::ChatGpt(_) => Subscription::ChatGpt,
            Credentials::Codex(_) => Subscription::Codex,
            Credentials::Xai(_) => Subscription::Xai,
        }
    }

    pub(crate) fn account(&self) -> String {
        match self {
            Credentials::ChatGpt(creds) => creds.display_account(),
            Credentials::Codex(creds) => creds.account_id.clone(),
            Credentials::Xai(creds) => creds.display_account(),
        }
    }

    pub(crate) fn access(&self) -> &str {
        match self {
            Credentials::ChatGpt(creds) => &creds.access_token,
            Credentials::Codex(creds) => &creds.access_token,
            Credentials::Xai(creds) => &creds.access_token,
        }
    }

    fn to_json(&self) -> String {
        match self {
            Credentials::ChatGpt(creds) => creds.to_json(),
            Credentials::Codex(creds) => creds.to_json(),
            Credentials::Xai(creds) => creds.to_json(),
        }
    }
}

/// The saved sign-in of `subscription`, when there is one.
pub(crate) fn load(secrets: &dyn SecretStore, subscription: Subscription) -> Option<Credentials> {
    let raw = secrets.get(subscription.secret())?;
    match subscription {
        Subscription::ChatGpt => ChatGptCredentials::from_json(&raw).map(Credentials::ChatGpt),
        Subscription::Codex => CodexCredentials::from_json(&raw).map(Credentials::Codex),
        Subscription::Xai => XaiCredentials::from_json(&raw).map(Credentials::Xai),
    }
}

pub(crate) fn store(secrets: &dyn SecretStore, creds: &Credentials) -> Result<()> {
    secrets.set(creds.subscription().secret(), &creds.to_json())
}

/// Forgets the sign-in; the keychain has no delete, and empty reads as none.
pub(crate) fn forget(secrets: &dyn SecretStore, subscription: Subscription) -> Result<()> {
    secrets.set(subscription.secret(), "")
}

/// The access token for a turn, refreshed first when it is about to
/// expire; a rotated token is written back.
pub(crate) async fn fresh_access(
    secrets: &dyn SecretStore,
    subscription: Subscription,
    client: &reqwest::Client,
) -> Result<String> {
    let saved = load(secrets, subscription).with_context(|| {
        format!(
            "Sign in to {} again in Settings, Models",
            name(subscription)
        )
    })?;
    let now = codex_auth::now_ms();
    let fresh = match saved.clone() {
        Credentials::ChatGpt(creds) => chatgpt_auth::refresh_if_due(client, creds, now)
            .await
            .map(Credentials::ChatGpt),
        Credentials::Codex(creds) => codex_auth::refresh_if_due(client, creds, now)
            .await
            .map(Credentials::Codex),
        Credentials::Xai(creds) => xai_auth::refresh_if_due(client, creds, now)
            .await
            .map(Credentials::Xai),
    }
    .map_err(anyhow::Error::msg)?;
    if fresh.access() != saved.access() {
        store(secrets, &fresh)?;
    }
    Ok(fresh.access().to_owned())
}

pub(crate) fn name(subscription: Subscription) -> &'static str {
    match subscription {
        Subscription::ChatGpt => "ChatGPT",
        Subscription::Codex => "ChatGPT (Codex)",
        Subscription::Xai => "SuperGrok",
    }
}

/// Where a sign-in stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInStatus {
    Waiting,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInState {
    pub status: SignInStatus,
    pub message: String,
    /// The account signed in to, once it is.
    pub account: String,
}

/// What the user does to sign in: open `url`, and on a device page enter
/// `user_code`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInChallenge {
    pub id: String,
    pub url: String,
    /// Empty for a sign-in in this computer's browser.
    pub user_code: String,
    pub message: String,
}

struct Attempt {
    subscription: Subscription,
    client: reqwest::Client,
    cancel: Arc<AtomicBool>,
    done: AtomicBool,
    state: Mutex<SignInState>,
    creds: Mutex<Option<Credentials>>,
    verifier: String,
    expected: String,
}

impl Attempt {
    fn set(&self, status: SignInStatus, message: impl Into<String>) {
        let mut state = self.state.lock().unwrap();
        state.status = status;
        state.message = message.into();
    }

    fn succeed(&self, creds: Credentials) {
        let account = creds.account();
        *self.creds.lock().unwrap() = Some(creds);
        let mut state = self.state.lock().unwrap();
        state.status = SignInStatus::Done;
        state.message = format!("Signed in as {account}");
        state.account = account;
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The sign-ins under way on this host.
#[derive(Default)]
pub(crate) struct Attempts(Mutex<HashMap<String, Arc<Attempt>>>);

impl Attempts {
    fn get(&self, id: &str) -> Result<Arc<Attempt>> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("That sign-in has expired; start again")
    }

    fn keep(&self, id: String, attempt: Arc<Attempt>) {
        let mut attempts = self.0.lock().unwrap();
        // A handful at most; older ones are abandoned.
        if attempts.len() > 8 {
            for old in attempts.values() {
                old.cancel.store(true, Ordering::Relaxed);
            }
            attempts.clear();
        }
        attempts.insert(id, attempt);
    }

    pub(crate) fn state(&self, id: &str) -> Result<SignInState> {
        Ok(self.get(id)?.state.lock().unwrap().clone())
    }

    pub(crate) fn cancel(&self, id: &str) {
        if let Some(attempt) = self.0.lock().unwrap().remove(id) {
            attempt.cancel.store(true, Ordering::Relaxed);
            attempt.done.store(true, Ordering::SeqCst);
        }
    }

    /// The credentials of a finished sign-in, taken out of it.
    pub(crate) fn take(&self, id: &str) -> Result<Credentials> {
        let attempt = self.get(id)?;
        let creds = attempt
            .creds
            .lock()
            .unwrap()
            .clone()
            .context("Finish signing in first")?;
        self.cancel(id);
        Ok(creds)
    }

    #[cfg(test)]
    pub(crate) fn finished(&self, creds: Credentials) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let attempt = Arc::new(Attempt {
            subscription: creds.subscription(),
            client: reqwest::Client::new(),
            cancel: Arc::default(),
            done: AtomicBool::new(true),
            state: Mutex::new(SignInState {
                status: SignInStatus::Waiting,
                message: String::new(),
                account: String::new(),
            }),
            creds: Mutex::default(),
            verifier: String::new(),
            expected: String::new(),
        });
        attempt.succeed(creds);
        self.keep(id.clone(), attempt);
        id
    }

    #[cfg(test)]
    pub(crate) fn browser_for_test(&self, subscription: Subscription, expected: &str) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.keep(
            id.clone(),
            Arc::new(Attempt {
                subscription,
                client: reqwest::Client::new(),
                cancel: Arc::default(),
                done: AtomicBool::new(false),
                state: Mutex::new(SignInState {
                    status: SignInStatus::Waiting,
                    message: String::new(),
                    account: String::new(),
                }),
                creds: Mutex::default(),
                verifier: "verifier".into(),
                expected: expected.into(),
            }),
        );
        id
    }

    /// Starts signing in to `subscription`: in this computer's browser
    /// (the answer comes to a fixed localhost address, or the redirect is
    /// pasted), or with a code on another device.
    pub(crate) async fn start(
        &self,
        subscription: Subscription,
        device: bool,
        client: reqwest::Client,
        secrets: &dyn SecretStore,
    ) -> Result<SignInChallenge> {
        let device = device || subscription.device_only();
        if device && !subscription.offers_device_code() {
            bail!("Sign in with ChatGPT has no device code; use the browser");
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiting = |message: &str| {
            Mutex::new(SignInState {
                status: SignInStatus::Waiting,
                message: message.into(),
                account: String::new(),
            })
        };
        if device {
            let attempt = Arc::new(Attempt {
                subscription,
                client: client.clone(),
                cancel: cancel.clone(),
                done: AtomicBool::new(false),
                state: waiting("Enter the code on the sign-in page, then come back."),
                creds: Mutex::default(),
                verifier: String::new(),
                expected: String::new(),
            });
            let (challenge, poll) = match subscription {
                Subscription::Xai => {
                    let endpoint = xai_auth::discover_token_endpoint(&client)
                        .await
                        .map_err(anyhow::Error::msg)?;
                    let code = xai_auth::start_device_login(&client)
                        .await
                        .map_err(anyhow::Error::msg)?;
                    let challenge = SignInChallenge {
                        id: id.clone(),
                        url: code.verification_uri_complete.clone(),
                        user_code: code.user_code.clone(),
                        message: attempt.state.lock().unwrap().message.clone(),
                    };
                    let polling = attempt.clone();
                    let poll = tokio::spawn(async move {
                        let found = xai_auth::poll_device_until_complete(
                            &polling.client,
                            code,
                            &endpoint,
                            &polling.cancel,
                        )
                        .await
                        .map(Credentials::Xai);
                        settle(&polling, found);
                    });
                    (challenge, poll)
                }
                _ => {
                    let code = codex_auth::start_device_login(&client)
                        .await
                        .map_err(anyhow::Error::msg)?;
                    let challenge = SignInChallenge {
                        id: id.clone(),
                        url: codex_auth::DEVICE_VERIFICATION_URI.into(),
                        user_code: code.user_code.clone(),
                        message: attempt.state.lock().unwrap().message.clone(),
                    };
                    let polling = attempt.clone();
                    let poll = tokio::spawn(async move {
                        let found = codex_auth::poll_device_until_complete(
                            &polling.client,
                            code,
                            &polling.cancel,
                        )
                        .await
                        .map(Credentials::Codex);
                        settle(&polling, found);
                    });
                    (challenge, poll)
                }
            };
            drop(poll);
            self.keep(id, attempt);
            return Ok(challenge);
        }
        let pkce = codex_auth::generate_pkce();
        let state = codex_auth::random_state();
        let url = match subscription {
            Subscription::ChatGpt => chatgpt_auth::authorize_url(
                &pkce,
                &state,
                &codex_auth::random_state(),
                host_id(secrets)?,
            ),
            _ => codex_auth::authorize_url(&pkce, &state),
        };
        let attempt = Arc::new(Attempt {
            subscription,
            client,
            cancel: cancel.clone(),
            done: AtomicBool::new(false),
            state: waiting("Finish signing in in the browser, or paste the address it ends on."),
            creds: Mutex::default(),
            verifier: pkce.verifier,
            expected: state.clone(),
        });
        let listening = attempt.clone();
        tokio::spawn(async move {
            let heard = tokio::task::spawn_blocking(move || {
                codex_auth::wait_for_browser_callback(&state, &cancel)
            })
            .await;
            match heard {
                Ok(BrowserCallback::Code(callback)) => exchange(&listening, callback).await,
                Ok(BrowserCallback::BindFailed(message)) => {
                    listening.set(SignInStatus::Waiting, message)
                }
                Ok(BrowserCallback::TimedOut) => {
                    listening.set(SignInStatus::Failed, "Signing in timed out")
                }
                Ok(BrowserCallback::Cancelled) | Err(_) => {}
            }
        });
        let challenge = SignInChallenge {
            id: id.clone(),
            url,
            user_code: String::new(),
            message: attempt.state.lock().unwrap().message.clone(),
        };
        self.keep(id, attempt);
        Ok(challenge)
    }

    /// Finishes a browser sign-in with the address the browser ended on,
    /// for when it could not reach this computer.
    pub(crate) async fn redirect(&self, id: &str, redirect: &str) -> Result<SignInState> {
        let attempt = self.get(id)?;
        if attempt.verifier.is_empty() {
            bail!("Paste the address only for a sign-in in the browser");
        }
        let callback = match attempt.subscription {
            Subscription::ChatGpt => {
                chatgpt_auth::callback_from_redirect(redirect, &attempt.expected)
                    .map_err(anyhow::Error::msg)?
            }
            _ => {
                let (code, state) = codex_auth::parse_authorization_input(redirect);
                let code = code.context("Paste the address the browser ended on, or its code")?;
                if state.is_some_and(|state| state != attempt.expected) {
                    bail!("That address belongs to another sign-in");
                }
                AuthorizationCallback {
                    code,
                    client_id: None,
                }
            }
        };
        exchange(&attempt, callback).await;
        let state = attempt.state.lock().unwrap().clone();
        Ok(state)
    }
}

/// Records how a device sign-in ended, unless it was settled already.
fn settle(attempt: &Attempt, found: Result<Credentials, String>) {
    if attempt.done.swap(true, Ordering::SeqCst) {
        return;
    }
    match found {
        Ok(creds) => attempt.succeed(creds),
        Err(_) if attempt.cancel.load(Ordering::Relaxed) => {}
        Err(problem) => attempt.set(SignInStatus::Failed, problem),
    }
}

async fn exchange(attempt: &Attempt, callback: AuthorizationCallback) {
    if attempt.done.swap(true, Ordering::SeqCst) {
        return;
    }
    let found = match attempt.subscription {
        Subscription::ChatGpt => {
            chatgpt_auth::exchange_authorization_code(&attempt.client, &callback, &attempt.verifier)
                .await
                .map(Credentials::ChatGpt)
        }
        _ => codex_auth::exchange_authorization_code(
            &attempt.client,
            &callback.code,
            &attempt.verifier,
            codex_auth::REDIRECT_URI,
        )
        .await
        .map(Credentials::Codex),
    };
    match found {
        Ok(creds) => attempt.succeed(creds),
        Err(problem) => {
            attempt.done.store(false, Ordering::SeqCst);
            attempt.set(SignInStatus::Failed, problem);
        }
    }
}

/// This installation's id for Sign in with ChatGPT, made once and kept in
/// the keychain under the WebView's name.
fn host_id(secrets: &dyn SecretStore) -> Result<uuid::Uuid> {
    if let Some(saved) = secrets.get(chatgpt_auth::HOST_ID_SECRET) {
        if let Ok(id) = uuid::Uuid::parse_str(saved.trim()) {
            return Ok(id);
        }
    }
    let id = uuid::Uuid::new_v4();
    secrets.set(chatgpt_auth::HOST_ID_SECRET, &id.to_string())?;
    Ok(id)
}
