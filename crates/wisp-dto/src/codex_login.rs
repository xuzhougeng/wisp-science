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

/// One saved ChatGPT account. Exactly one is `active`: its credentials back
/// every ChatGPT model.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CodexAccount {
    pub account_id: String,
    pub email: String,
    pub plan_type: String,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CodexUsageWindow {
    /// 0–100.
    pub used_percent: f64,
    pub window_seconds: i64,
    /// Unix seconds; 0 when unknown.
    pub reset_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CodexAccountUsage {
    pub account_id: String,
    pub plan_type: String,
    pub limit_reached: bool,
    /// Short (5-hour) window.
    pub primary: Option<CodexUsageWindow>,
    /// Weekly window.
    pub secondary: Option<CodexUsageWindow>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CodexImportResult {
    pub imported: usize,
    pub accounts: Vec<CodexAccount>,
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

    #[test]
    fn account_pool_payloads_round_trip_without_credentials() {
        let wire = serde_json::json!({
            "imported": 1,
            "accounts": [{"account_id": "acct-1", "email": "fixture@example.test", "plan_type": "plus", "active": true}],
        });
        let result: CodexImportResult = serde_json::from_value(wire.clone()).unwrap();
        assert!(result.accounts[0].active);
        assert_eq!(serde_json::to_value(&result).unwrap(), wire);
        let usage = serde_json::json!({
            "account_id": "acct-1",
            "plan_type": "plus",
            "limit_reached": false,
            "primary": {"used_percent": 23.5, "window_seconds": 18000, "reset_at": 1781276043},
            "secondary": null,
        });
        let parsed: CodexAccountUsage = serde_json::from_value(usage.clone()).unwrap();
        assert_eq!(parsed.primary.as_ref().unwrap().used_percent, 23.5);
        assert_eq!(serde_json::to_value(parsed).unwrap(), usage);
        for text in [wire.to_string(), usage.to_string()] {
            assert!(!text.contains("token"));
        }
    }
}
