//! Subscription sign-in responses shared by the WebView and native settings.
//! OAuth credentials remain in the host and keyring, never in these payloads.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodexLoginChallenge {
    pub login_id: String,
    pub method: String,
    pub url: String,
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodexLoginSnapshot {
    pub status: String,
    pub message: String,
    pub account_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodexSubscriptionStatus {
    pub signed_in: bool,
    pub account_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sign_in_fixtures_keep_credentials_out_of_responses() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../contracts/native-settings/v1/codex-login.json"
        ))
        .unwrap();
        for key in ["browser", "device"] {
            let challenge: CodexLoginChallenge =
                serde_json::from_value(fixture[key].clone()).unwrap();
            assert_eq!(challenge.method, key);
            assert!(!challenge.login_id.is_empty());
            assert_eq!(serde_json::to_value(challenge).unwrap(), fixture[key]);
        }
        let status: CodexSubscriptionStatus =
            serde_json::from_value(fixture["saved_account"].clone()).unwrap();
        assert!(status.signed_in);
        for key in ["pending", "success", "expired"] {
            let snapshot: CodexLoginSnapshot =
                serde_json::from_value(fixture[key].clone()).unwrap();
            assert_eq!(serde_json::to_value(snapshot).unwrap(), fixture[key]);
        }
        for command in [
            "codex_subscription_status",
            "start_codex_login",
            "codex_login_status",
            "submit_codex_login_redirect",
            "cancel_codex_login",
            "save_codex_login",
        ] {
            assert!(crate::native_settings::COMMANDS.contains(&command));
        }
        for forbidden in ["access_token", "refresh_token", "id_token", "verifier"] {
            assert!(!fixture.to_string().contains(forbidden));
        }
    }
}
