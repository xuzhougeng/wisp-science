//! Subscription sign-in: ChatGPT Plus/Pro (Codex) and SuperGrok (xAI).
//!
//! Codex browser login listens on the Codex client's fixed localhost callback.
//! Device-code login works when that callback cannot reach this machine
//! (SSH, WSL, a remote browser); xAI only offers device code. The commands keep
//! their Codex names for native hosts; `provider: "xai"` selects xAI.
//! Tokens are stored in the OS keyring.

use crate::models::{self, ModelProfile, DEFAULT_CONTEXT_WINDOW};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::State;
use wisp_llm::codex_auth::{
    self, authorize_url, generate_pkce, parse_authorization_input, random_state, BrowserCallback,
    CodexCredentials, REDIRECT_URI,
};
use wisp_llm::xai_auth::{self, XaiCredentials};

pub use wisp_dto::codex_login::{CodexLoginChallenge, CodexLoginSnapshot, CodexSubscriptionStatus};

#[derive(Clone)]
enum Credentials {
    Codex(CodexCredentials),
    Xai(XaiCredentials),
}

impl Credentials {
    fn account(&self) -> String {
        match self {
            Self::Codex(creds) => creds.account_id.clone(),
            Self::Xai(creds) => creds.display_account(),
        }
    }
}

fn is_xai(provider: &Option<String>) -> Result<bool, String> {
    match provider.as_deref().map(str::trim) {
        None | Some("") | Some("codex") | Some("openai_codex") => Ok(false),
        Some("xai") | Some("xai_oauth") => Ok(true),
        Some(other) => Err(format!("Unknown subscription provider: {other}")),
    }
}

struct LoginSession {
    cancel: Arc<AtomicBool>,
    done: AtomicBool,
    status: Mutex<String>,
    message: Mutex<String>,
    creds: Mutex<Option<Credentials>>,
    verifier: Mutex<String>,
    state: Mutex<String>,
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<LoginSession>>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<LoginSession>>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session(id: &str) -> Result<Arc<LoginSession>, String> {
    sessions()
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or_else(|| "Codex sign-in expired. Start again.".into())
}

fn snapshot(session: &LoginSession) -> CodexLoginSnapshot {
    let status = session.status.lock().unwrap().clone();
    let account_id = session
        .creds
        .lock()
        .unwrap()
        .as_ref()
        .map(Credentials::account)
        .unwrap_or_default();
    CodexLoginSnapshot {
        status,
        message: session.message.lock().unwrap().clone(),
        account_id,
    }
}

fn is_status(session: &LoginSession, status: &str) -> bool {
    session.status.lock().unwrap().as_str() == status
}

fn set_status(session: &LoginSession, status: &str, message: impl Into<String>) {
    *session.status.lock().unwrap() = status.to_string();
    *session.message.lock().unwrap() = message.into();
}

async fn finish_with_code(session: &LoginSession, code: &str, verifier: &str, redirect_uri: &str) {
    if session.done.swap(true, Ordering::SeqCst) {
        return;
    }
    let client = codex_auth::http_client(crate::llm_proxy().as_deref());
    match codex_auth::exchange_authorization_code(&client, code, verifier, redirect_uri).await {
        Ok(creds) => {
            let account = creds.account_id.clone();
            *session.creds.lock().unwrap() = Some(Credentials::Codex(creds));
            set_status(
                session,
                "success",
                format!("Signed in to ChatGPT account {account}"),
            );
            session.cancel.store(true, Ordering::Relaxed);
        }
        Err(error) => {
            session.done.store(false, Ordering::SeqCst);
            set_status(session, "error", error);
        }
    }
}

#[tauri::command]
pub async fn codex_subscription_status(
    provider: Option<String>,
) -> Result<CodexSubscriptionStatus, String> {
    let saved = if is_xai(&provider)? {
        models::load_global_xai().map(Credentials::Xai)
    } else {
        models::load_global_codex().map(Credentials::Codex)
    };
    Ok(match saved {
        Some(creds) => CodexSubscriptionStatus {
            signed_in: true,
            account_id: creds.account(),
        },
        None => CodexSubscriptionStatus {
            signed_in: false,
            account_id: String::new(),
        },
    })
}

#[tauri::command]
pub async fn start_codex_login(
    method: String,
    provider: Option<String>,
) -> Result<CodexLoginChallenge, String> {
    let xai = is_xai(&provider)?;
    let method = match method.trim() {
        _ if xai => "device",
        "device" | "device_code" => "device",
        "browser" | "" => "browser",
        other => return Err(format!("Unknown Codex sign-in method: {other}")),
    };
    let login_id = uuid::Uuid::new_v4().simple().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    let session = Arc::new(LoginSession {
        cancel: cancel.clone(),
        done: AtomicBool::new(false),
        status: Mutex::new("pending".into()),
        message: Mutex::new(String::new()),
        creds: Mutex::new(None),
        verifier: Mutex::new(String::new()),
        state: Mutex::new(String::new()),
    });
    let challenge = if xai {
        let client = codex_auth::http_client(crate::llm_proxy().as_deref());
        let token_endpoint = xai_auth::discover_token_endpoint(&client).await?;
        let device = xai_auth::start_device_login(&client).await?;
        set_status(
            &session,
            "pending",
            "Approve the sign-in on the xAI page. Enter the code if it asks for one.",
        );
        let polling = session.clone();
        let device_for_poll = device.clone();
        tokio::spawn(async move {
            let client = codex_auth::http_client(crate::llm_proxy().as_deref());
            let result = xai_auth::poll_device_until_complete(
                &client,
                device_for_poll,
                &token_endpoint,
                &cancel,
            )
            .await;
            match result {
                Ok(creds) => {
                    if polling.done.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    let account = creds.display_account();
                    *polling.creds.lock().unwrap() = Some(Credentials::Xai(creds));
                    set_status(&polling, "success", format!("Signed in to {account}"));
                }
                Err(error) => {
                    if cancel.load(Ordering::Relaxed) || is_status(&polling, "success") {
                        return;
                    }
                    set_status(&polling, "error", error);
                }
            }
        });
        CodexLoginChallenge {
            login_id: login_id.clone(),
            method: method.into(),
            url: device.verification_uri_complete,
            user_code: device.user_code,
            verification_uri: device.verification_uri,
            message: session.message.lock().unwrap().clone(),
        }
    } else if method == "device" {
        let client = codex_auth::http_client(crate::llm_proxy().as_deref());
        let device = codex_auth::start_device_login(&client).await?;
        set_status(
            &session,
            "pending",
            "Enter the one-time code on the ChatGPT device page.",
        );
        let polling = session.clone();
        let device_for_poll = device.clone();
        tokio::spawn(async move {
            let client = codex_auth::http_client(crate::llm_proxy().as_deref());
            match codex_auth::poll_device_until_complete(&client, device_for_poll, &cancel).await {
                Ok(creds) => {
                    if polling.done.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    let account = creds.account_id.clone();
                    *polling.creds.lock().unwrap() = Some(Credentials::Codex(creds));
                    set_status(
                        &polling,
                        "success",
                        format!("Signed in to ChatGPT account {account}"),
                    );
                }
                Err(error) => {
                    if cancel.load(Ordering::Relaxed) || is_status(&polling, "success") {
                        return;
                    }
                    set_status(&polling, "error", error);
                }
            }
        });
        CodexLoginChallenge {
            login_id: login_id.clone(),
            method: method.into(),
            url: codex_auth::DEVICE_VERIFICATION_URI.into(),
            user_code: device.user_code,
            verification_uri: codex_auth::DEVICE_VERIFICATION_URI.into(),
            message: session.message.lock().unwrap().clone(),
        }
    } else {
        let pkce = generate_pkce();
        let state = random_state();
        let url = authorize_url(&pkce, &state);
        *session.verifier.lock().unwrap() = pkce.verifier;
        *session.state.lock().unwrap() = state.clone();
        set_status(
            &session,
            "pending",
            "Finish sign-in in the browser, or paste the redirect URL.",
        );
        let waiting = session.clone();
        tokio::spawn(async move {
            let expected = state;
            let callback = tokio::task::spawn_blocking(move || {
                codex_auth::wait_for_browser_callback(&expected, &cancel)
            })
            .await;
            match callback {
                Ok(BrowserCallback::Code(code)) => {
                    let verifier = waiting.verifier.lock().unwrap().clone();
                    finish_with_code(&waiting, &code, &verifier, REDIRECT_URI).await;
                }
                Ok(BrowserCallback::BindFailed(message)) => {
                    if is_status(&waiting, "pending") {
                        set_status(&waiting, "pending", message);
                    }
                }
                Ok(BrowserCallback::TimedOut) => {
                    if is_status(&waiting, "pending") {
                        set_status(&waiting, "error", "ChatGPT sign-in timed out.");
                    }
                }
                Ok(BrowserCallback::Cancelled) | Err(_) => {}
            }
        });
        CodexLoginChallenge {
            login_id: login_id.clone(),
            method: method.into(),
            url,
            user_code: String::new(),
            verification_uri: String::new(),
            message: session.message.lock().unwrap().clone(),
        }
    };
    let mut guard = sessions().lock().unwrap();
    if guard.len() > 8 {
        guard.clear();
    }
    guard.insert(login_id, session);
    Ok(challenge)
}

#[tauri::command]
pub async fn codex_login_status(login_id: String) -> Result<CodexLoginSnapshot, String> {
    Ok(snapshot(session(&login_id)?.as_ref()))
}

#[tauri::command]
pub async fn submit_codex_login_redirect(
    login_id: String,
    redirect: String,
) -> Result<CodexLoginSnapshot, String> {
    let session = session(&login_id)?;
    let (code, pasted_state) = parse_authorization_input(&redirect);
    let Some(code) = code else {
        return Err("Paste the redirect URL or the authorization code.".into());
    };
    let expected = session.state.lock().unwrap().clone();
    if let Some(state) = pasted_state {
        if !expected.is_empty() && state != expected {
            return Err("That redirect belongs to a different sign-in attempt.".into());
        }
    }
    let verifier = session.verifier.lock().unwrap().clone();
    if verifier.is_empty() {
        return Err("Paste the redirect URL only for browser sign-in.".into());
    }
    finish_with_code(&session, &code, &verifier, REDIRECT_URI).await;
    Ok(snapshot(session.as_ref()))
}

#[tauri::command]
pub fn cancel_codex_login(login_id: String) -> Result<(), String> {
    if let Some(session) = sessions().lock().unwrap().remove(&login_id) {
        session.cancel.store(true, Ordering::Relaxed);
        set_status(&session, "error", "Sign-in cancelled");
    }
    Ok(())
}

#[tauri::command]
pub async fn save_codex_login(
    state: State<'_, crate::AppState>,
    login_id: String,
    model: String,
    label: String,
    profile_id: Option<String>,
    api_url: Option<String>,
    use_saved: Option<bool>,
    provider: Option<String>,
) -> Result<Vec<ModelProfile>, String> {
    let xai = is_xai(&provider)?;
    let creds = if use_saved.unwrap_or(false) {
        let client = codex_auth::http_client(crate::llm_proxy().as_deref());
        if xai {
            let Some(saved) = models::load_global_xai() else {
                return Err("No SuperGrok subscription is signed in on this machine.".into());
            };
            Credentials::Xai(xai_auth::refresh_if_due(&client, saved, codex_auth::now_ms()).await?)
        } else {
            let Some(saved) = models::load_global_codex() else {
                return Err("No ChatGPT subscription is signed in on this machine.".into());
            };
            Credentials::Codex(
                codex_auth::refresh_if_due(&client, saved, codex_auth::now_ms()).await?,
            )
        }
    } else {
        let session = session(&login_id)?;
        let creds = session
            .creds
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "Finish sign-in before saving the model.".to_string())?;
        if matches!(creds, Credentials::Xai(_)) != xai {
            return Err("That sign-in belongs to a different subscription.".into());
        }
        sessions().lock().unwrap().remove(&login_id);
        creds
    };
    let (provider, default_model, vendor, default_url) = if xai {
        (
            "xai_oauth",
            xai_auth::DEFAULT_MODEL,
            "Grok",
            xai_auth::DEFAULT_BASE_URL,
        )
    } else {
        (
            "openai_codex",
            codex_auth::DEFAULT_MODEL,
            "ChatGPT",
            codex_auth::DEFAULT_BASE_URL,
        )
    };
    let model = {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            default_model.to_string()
        } else {
            trimmed.to_string()
        }
    };
    let label = {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            format!("{vendor} {model}")
        } else {
            trimmed.to_string()
        }
    };
    let api_url = {
        let trimmed = api_url.unwrap_or_default();
        let trimmed = trimmed.trim();
        if trimmed.is_empty() {
            default_url.to_string()
        } else {
            trimmed.to_string()
        }
    };
    if xai {
        xai_auth::validate_xai_url(&api_url)?;
    }
    let profile_id = profile_id.unwrap_or_default();
    let profile = if profile_id.is_empty() {
        ModelProfile {
            id: String::new(),
            label,
            provider: provider.into(),
            api_url,
            endpoint_suffix: String::new(),
            model,
            has_api_key: false,
            active: false,
            max_tokens: 0,
            context_window: DEFAULT_CONTEXT_WINDOW,
            reasoning_effort: String::new(),
            service_tier: String::new(),
            user_agent: String::new(),
            send_user_agent: true,
            send_session_id: None,
            session_header_name: String::new(),
            supports_vision: true,
            use_for_vision: false,
            use_for_image_generation: false,
            image_generation_capable: false,
            image_size: String::new(),
            image_quality: String::new(),
            image_aspect_ratio: String::new(),
            image_resolution: String::new(),
            use_for_video_generation: false,
            video_duration_secs: None,
            video_aspect_ratio: None,
            video_resolution: None,
        }
    } else {
        let Some(mut existing) = models::profile_owned(&state.store, &profile_id).await else {
            return Err("Model profile not found.".into());
        };
        existing.provider = provider.into();
        existing.api_url = api_url;
        existing.model = model;
        existing.label = label;
        existing.endpoint_suffix.clear();
        existing
    };
    let saved_id = profile.id.clone();
    let use_for_vision = profile.use_for_vision;
    let use_for_image = profile.use_for_image_generation;
    let use_for_video = profile.use_for_video_generation;
    models::save_model(
        state.clone(),
        profile,
        None,
        Some(use_for_vision),
        Some(use_for_image),
        Some(use_for_video),
    )
    .await?;
    let id = if saved_id.is_empty() {
        state
            .store
            .get_setting(models::ACTIVE_KEY)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    } else {
        saved_id
    };
    if id.is_empty() {
        return Err("Could not save the subscription model.".into());
    }
    match &creds {
        Credentials::Codex(creds) => models::store_codex_credentials(&id, creds)?,
        Credentials::Xai(creds) => models::store_xai_credentials(&id, creds)?,
    }
    crate::clear_idle_agents(&state).await;
    Ok(models::decorated_models(&state.store).await)
}
