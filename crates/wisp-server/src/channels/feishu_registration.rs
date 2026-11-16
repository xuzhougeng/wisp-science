//! Feishu/Lark one-click app registration over the OAuth device flow, a
//! port of the WebView's `src-tauri/src/channels/feishu_registration.rs`
//! whose accounts hosts can be pointed at a fake in tests.
//!
//! The registration endpoint deliberately returns useful JSON bodies for
//! pending/slow-down states even when the HTTP status is non-success. Never
//! call `error_for_status` here. Device codes and client secrets stay in the
//! backend and must not be logged.

use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::Instant;

const FEISHU_ACCOUNTS: &str = "https://accounts.feishu.cn";
const LARK_ACCOUNTS: &str = "https://accounts.larksuite.com";

/// Where Feishu's and Lark's accounts services are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accounts {
    pub feishu: String,
    pub lark: String,
}

impl Default for Accounts {
    fn default() -> Self {
        Self {
            feishu: FEISHU_ACCOUNTS.into(),
            lark: LARK_ACCOUNTS.into(),
        }
    }
}

impl Accounts {
    fn base(&self, international: bool) -> &str {
        if international {
            &self.lark
        } else {
            &self.feishu
        }
    }
}
const REGISTRATION_PATH: &str = "/oauth/v1/app/registration";
const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_EXPIRY: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
struct BeginResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    interval: u64,
    #[serde(default)]
    expire_in: u64,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
struct UserInfo {
    #[serde(default)]
    open_id: String,
    #[serde(default)]
    tenant_brand: String,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
struct PollResponse {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    user_info: UserInfo,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Pending,
    SlowDown,
    SwitchToLark,
    Success,
    Denied,
    Expired,
    Fatal,
}

fn decide(response: &PollResponse, switched_to_lark: bool) -> Decision {
    if !response.client_id.is_empty() && !response.client_secret.is_empty() {
        return Decision::Success;
    }
    if response.user_info.tenant_brand == "lark" && !switched_to_lark {
        return Decision::SwitchToLark;
    }
    match response.error.as_str() {
        "" | "authorization_pending" => Decision::Pending,
        "slow_down" => Decision::SlowDown,
        "access_denied" => Decision::Denied,
        "expired_token" => Decision::Expired,
        _ => Decision::Fatal,
    }
}

fn registration_client() -> Result<Client> {
    Client::builder()
        .user_agent("wisp-science")
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to create Feishu registration HTTP client")
}

pub struct RegistrationStart {
    pub flow: RegistrationFlow,
    pub verification_uri: String,
    pub expires_in_seconds: u64,
}

pub struct RegistrationFlow {
    http: Client,
    accounts: Accounts,
    device_code: String,
    international: bool,
    switched_to_lark: bool,
    poll_interval: Duration,
    expires_at: Instant,
    next_poll_at: Instant,
}

impl RegistrationFlow {
    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    pub async fn begin(accounts: Accounts, international: bool) -> Result<RegistrationStart> {
        let http = registration_client()?;
        let response = http
            .post(format!(
                "{}{}",
                accounts.base(international),
                REGISTRATION_PATH
            ))
            .form(&[
                ("action", "begin"),
                ("archetype", "PersonalAgent"),
                ("auth_method", "client_secret"),
                ("request_user_info", "open_id"),
            ])
            .send()
            .await
            .context("Could not ask Feishu for a QR code")?
            .json::<BeginResponse>()
            .await
            .context("Feishu's answer could not be read")?;
        if !response.error.is_empty() {
            bail!(
                "{}",
                registration_error(&response.error, &response.error_description)
            );
        }
        if response.device_code.is_empty() || response.verification_uri_complete.is_empty() {
            bail!("Feishu's answer has no device code or address");
        }
        if !response.verification_uri_complete.starts_with("https://") {
            bail!("Feishu gave an address that is not https");
        }
        let poll_interval = Duration::from_secs(response.interval.max(1));
        let poll_interval = if response.interval == 0 {
            DEFAULT_INTERVAL
        } else {
            poll_interval
        };
        let expiry = if response.expire_in == 0 {
            DEFAULT_EXPIRY
        } else {
            Duration::from_secs(response.expire_in)
        };
        Ok(RegistrationStart {
            verification_uri: response.verification_uri_complete,
            expires_in_seconds: expiry.as_secs(),
            flow: Self {
                http,
                accounts,
                device_code: response.device_code,
                international,
                switched_to_lark: false,
                poll_interval,
                expires_at: Instant::now() + expiry,
                // The reference SDK performs the first poll immediately.
                next_poll_at: Instant::now(),
            },
        })
    }

    pub async fn poll(&mut self) -> Result<RegistrationPoll> {
        let now = Instant::now();
        if now >= self.expires_at {
            return Ok(RegistrationPoll::Expired);
        }
        if now < self.next_poll_at {
            return Ok(RegistrationPoll::Pending {
                retry_after: self.next_poll_at - now,
            });
        }

        // At most one regional hand-off can happen for a device code. If the
        // first response identifies a Lark tenant, retry immediately on the
        // international accounts host, matching phantty/the official SDK.
        for _ in 0..2 {
            let base = if self.switched_to_lark {
                self.accounts.lark.clone()
            } else {
                self.accounts.base(self.international).to_owned()
            };
            let response = self
                .http
                .post(format!("{base}{REGISTRATION_PATH}"))
                .form(&[
                    ("action", "poll"),
                    ("device_code", self.device_code.as_str()),
                ])
                .send()
                .await
                .context("Could not check the Feishu scan")?
                // Do not gate on HTTP status: device-flow pending states may
                // intentionally be returned with a 4xx response.
                .json::<PollResponse>()
                .await
                .context("Feishu's answer about the scan could not be read")?;

            match decide(&response, self.switched_to_lark) {
                Decision::Success => {
                    return Ok(RegistrationPoll::Success {
                        app_id: response.client_id,
                        app_secret: response.client_secret,
                        international: self.international,
                        owner_open_id: response.user_info.open_id,
                    });
                }
                Decision::SwitchToLark => {
                    self.switched_to_lark = true;
                    self.international = true;
                    continue;
                }
                Decision::Pending => {
                    self.next_poll_at = Instant::now() + self.poll_interval;
                    return Ok(RegistrationPoll::Pending {
                        retry_after: self.poll_interval,
                    });
                }
                Decision::SlowDown => {
                    self.poll_interval += Duration::from_secs(5);
                    self.next_poll_at = Instant::now() + self.poll_interval;
                    return Ok(RegistrationPoll::Pending {
                        retry_after: self.poll_interval,
                    });
                }
                Decision::Denied => return Ok(RegistrationPoll::Denied),
                Decision::Expired => return Ok(RegistrationPoll::Expired),
                Decision::Fatal => {
                    return Err(anyhow!(registration_error(
                        &response.error,
                        &response.error_description
                    )));
                }
            }
        }
        Err(anyhow!("Could not switch to Lark for this scan"))
    }
}

pub enum RegistrationPoll {
    Pending {
        retry_after: Duration,
    },
    Success {
        app_id: String,
        app_secret: String,
        international: bool,
        owner_open_id: String,
    },
    Denied,
    Expired,
}

fn registration_error(code: &str, description: &str) -> String {
    let label = match code {
        "access_denied" => "Feishu authorization was cancelled",
        "expired_token" => "The Feishu QR code expired",
        "slow_down" => "Feishu asks to check the scan less often",
        "authorization_pending" => "Waiting for the scan to be confirmed in Feishu",
        _ => "Feishu authorization failed",
    };
    if description.trim().is_empty() {
        if code.is_empty() {
            label.to_string()
        } else {
            format!("{label} ({code})")
        }
    } else {
        format!("{label}: {}", description.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_decisions_match_device_flow_states() {
        assert_eq!(decide(&PollResponse::default(), false), Decision::Pending);
        assert_eq!(
            decide(
                &PollResponse {
                    error: "slow_down".into(),
                    ..Default::default()
                },
                false
            ),
            Decision::SlowDown
        );
        assert_eq!(
            decide(
                &PollResponse {
                    error: "access_denied".into(),
                    ..Default::default()
                },
                false
            ),
            Decision::Denied
        );
        assert_eq!(
            decide(
                &PollResponse {
                    error: "expired_token".into(),
                    ..Default::default()
                },
                false
            ),
            Decision::Expired
        );
    }

    #[test]
    fn credentials_win_over_stale_pending_error() {
        assert_eq!(
            decide(
                &PollResponse {
                    client_id: "cli_1".into(),
                    client_secret: "secret".into(),
                    error: "authorization_pending".into(),
                    ..Default::default()
                },
                false
            ),
            Decision::Success
        );
    }

    #[test]
    fn lark_tenant_switches_only_once() {
        let response = PollResponse {
            user_info: UserInfo {
                tenant_brand: "lark".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(decide(&response, false), Decision::SwitchToLark);
        assert_eq!(decide(&response, true), Decision::Pending);
    }

    #[test]
    fn parses_realistic_poll_shape() {
        let response: PollResponse = serde_json::from_str(
            r#"{"client_id":"cli_9","client_secret":"sec_9","user_info":{"open_id":"ou_1","tenant_brand":"feishu"},"extra":1}"#,
        )
        .unwrap();
        assert_eq!(response.client_id, "cli_9");
        assert_eq!(response.user_info.tenant_brand, "feishu");
        assert_eq!(response.user_info.open_id, "ou_1");
        assert_eq!(decide(&response, false), Decision::Success);
    }
}
