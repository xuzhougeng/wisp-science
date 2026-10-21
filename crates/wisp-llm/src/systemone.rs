//! "System One" decision protocol (`POST /v1/systemone`): TypeSafe's Jev
//! models, StartLux-Decision, or any server that speaks it.
//!
//! Not a chat [`Provider`](crate::Provider): no messages, tools, text, or
//! streaming. The caller sends one `state` plus typed questions and gets one
//! calibrated probability answer per question back. A [`DecisionModel`] names
//! the endpoint; its [`ProviderConfig`] is reused for the URL, key, model,
//! proxy, and request identity, and its `kind`, `max_tokens`, and reasoning
//! fields are ignored.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{http_client, LlmError, ProviderConfig, Result};

/// TypeSafe, the endpoint an empty base URL means.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
/// TypeSafe's alias for its current Jev model.
pub const TYPESAFE_MODEL: &str = "jev-latest";
/// TypeSafe answers in ~100 ms; the official SDK waits 10 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// One configured System One endpoint.
#[derive(Debug, Clone)]
pub struct DecisionModel {
    /// URL (root, `/v1`, or the full path), key, model, proxy, and request
    /// identity. An empty key sends no `Authorization` header, for a
    /// self-hosted server without keys; an empty model lets the server pick.
    pub http: ProviderConfig,
    /// `think_mode` for servers that think when unsure (StartLux AutoThink:
    /// `auto`, `never`, `always`). Empty omits it, so servers that do not
    /// know the field never see it.
    pub think_mode: String,
    /// Per request. A thinking server can take minutes on a hard question.
    pub timeout: Duration,
}

impl DecisionModel {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            http: ProviderConfig::openai(base_url.trim(), api_key.trim(), model.trim()),
            think_mode: String::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// TypeSafe Jev with the user's key.
    pub fn typesafe(api_key: &str) -> Self {
        Self::new(DEFAULT_BASE_URL, api_key, TYPESAFE_MODEL)
    }

    pub fn with_proxy(mut self, proxy: Option<String>) -> Self {
        self.http.proxy = proxy;
        self
    }

    /// The headless configuration. `WISP_DECISION_API_URL` selects any System One
    /// server (`WISP_DECISION_API_KEY`, `WISP_DECISION_MODEL`,
    /// `WISP_DECISION_THINK_MODE`, `WISP_DECISION_TIMEOUT_SECS` refine it);
    /// otherwise `TYPESAFE_API_KEY` selects TypeSafe. `None` without either.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let get = |name: &str| var(name).map(|v| v.trim().to_string()).unwrap_or_default();
        let url = get("WISP_DECISION_API_URL");
        if url.is_empty() {
            let key = get("TYPESAFE_API_KEY");
            return (!key.is_empty()).then(|| Self::typesafe(&key));
        }
        let mut model = Self::new(
            &url,
            &get("WISP_DECISION_API_KEY"),
            &get("WISP_DECISION_MODEL"),
        );
        model.think_mode = get("WISP_DECISION_THINK_MODE");
        if let Ok(secs) = get("WISP_DECISION_TIMEOUT_SECS").parse::<u64>() {
            model.timeout = Duration::from_secs(secs.clamp(1, 600));
        }
        Some(model)
    }

    /// Where requests go, for approval prompts: the endpoint's host.
    pub fn host(&self) -> String {
        let url = endpoint(&self.http.base_url);
        url.split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or(&url)
            .to_string()
    }
}

/// One typed question. The map key it is stored under is for the caller only
/// and is never shown to the model, so `instructions` must be self-contained.
/// Backticked paths (`` `order.charges` ``) refer to fields of an object state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    /// Yes/no.
    Noul {
        instructions: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    /// One label out of a set; a `None` description leaves the label bare.
    Choice {
        instructions: String,
        criteria: BTreeMap<String, Option<String>>,
    },
    /// 2–10 ordered levels, lowest first; the index is the level number.
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: String,
    #[serde(rename = "false")]
    pub no: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    /// Probability of "yes", 0–1.
    Noul { noul: f64 },
    Choice {
        /// The most probable label; `None` when the server may answer "none
        /// of these" (StartLux `allow_none`, on by default on Bamboo) and did.
        choice: Option<String>,
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: f64,
    },
    Score {
        /// Probability-weighted mean level; can fall between levels.
        score: f64,
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        legend: BTreeMap<String, String>,
        confidence: f64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DecisionUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Decisions {
    /// The versioned model that answered (e.g. `jev-1.13.0`), even when the
    /// request used an alias.
    #[serde(default)]
    pub model: String,
    /// Keyed by the caller's question IDs.
    pub answers: BTreeMap<String, Answer>,
    #[serde(default)]
    pub usage: DecisionUsage,
}

#[derive(Serialize)]
struct Request<'a> {
    #[serde(skip_serializing_if = "str::is_empty")]
    model: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    think_mode: &'a str,
    state: &'a Value,
    questions: &'a BTreeMap<String, Question>,
}

fn endpoint(base_url: &str) -> String {
    let base = match base_url.trim().trim_end_matches('/') {
        "" => DEFAULT_BASE_URL,
        base => base,
    };
    if base.ends_with("/v1/systemone") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/systemone")
    } else {
        format!("{base}/v1/systemone")
    }
}

/// Ask every question about `state` in one parallel pass. `state` is a
/// string, an array of strings, or an object of name/value pairs; an empty
/// model lets the server pick its default. HTTP errors come back as
/// [`LlmError::Api`] (422 names the invalid field), so [`crate::is_retriable`]
/// applies unchanged. No retries here.
pub async fn evaluate(
    model: &DecisionModel,
    state: &Value,
    questions: &BTreeMap<String, Question>,
) -> Result<Decisions> {
    let cfg = &model.http;
    let body = Request {
        model: cfg.model.trim(),
        think_mode: model.think_mode.trim(),
        state,
        questions,
    };
    let mut request = cfg.request_headers(http_client(cfg).post(endpoint(&cfg.base_url)));
    if !cfg.api_key.trim().is_empty() {
        request = request.bearer_auth(cfg.api_key.trim());
    }
    let resp = request.timeout(model.timeout).json(&body).send().await?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(LlmError::Api { status, body: text });
    }
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn endpoint_accepts_root_v1_or_full_path() {
        assert_eq!(endpoint(""), "https://api.typesafe.ai/v1/systemone");
        assert_eq!(endpoint("http://h/"), "http://h/v1/systemone");
        assert_eq!(endpoint("http://h/v1"), "http://h/v1/systemone");
        assert_eq!(endpoint("http://h/v1/systemone"), "http://h/v1/systemone");
    }

    /// Serve one canned response; return the raw request it received.
    async fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0; 4096];
            // Read until the headers and the full Content-Length body arrived.
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if raw.len() >= end + 4 + len || n == 0 {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&raw).into_owned()
        });
        (url, task)
    }

    #[tokio::test]
    async fn evaluate_round_trips_all_three_question_types() {
        let (url, server) = serve_once(
            "200 OK",
            r#"{"model":"jev-1.13.0","answers":{
                "irreversible":{"type":"noul","noul":0.93},
                "kind":{"type":"choice","choice":"submit","probabilities":{"other":0.1,"submit":0.9},"confidence":0.8},
                "risk":{"type":"score","score":1.4,"legend":{"0":"none","1":"some","2":"high"},"probabilities":{"0":0.1,"1":0.4,"2":0.5},"confidence":0.5}
            },"usage":{"input_tokens":120,"output_tokens":25}}"#,
        )
        .await;
        let cfg = DecisionModel::new(&url, "sk-test", "jev-latest");
        let questions = BTreeMap::from([
            (
                "irreversible".to_string(),
                Question::Noul {
                    instructions: "Does `action` commit an irreversible change?".into(),
                    criteria: Some(NoulCriteria {
                        yes: "Sends, deletes, purchases, or submits".into(),
                        no: "Can be undone".into(),
                    }),
                },
            ),
            (
                "kind".to_string(),
                Question::Choice {
                    instructions: "What does `action` do?".into(),
                    criteria: BTreeMap::from([
                        ("submit".to_string(), Some("Submits a form".to_string())),
                        ("other".to_string(), None),
                    ]),
                },
            ),
            (
                "risk".to_string(),
                Question::Score {
                    instructions: "How risky is `action`?".into(),
                    criteria: vec!["none".into(), "some".into(), "high".into()],
                },
            ),
        ]);
        let state = json!({"action": "click Send in Mail"});

        let got = evaluate(&cfg, &state, &questions).await.unwrap();

        let raw = server.await.unwrap();
        assert!(raw.starts_with("POST /v1/systemone HTTP/1.1"), "{raw}");
        assert!(raw
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test"));
        let sent: Value = serde_json::from_str(&raw[raw.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(sent["model"], "jev-latest");
        assert!(sent.get("think_mode").is_none(), "{sent}");
        assert_eq!(sent["state"], state);
        assert_eq!(sent["questions"]["irreversible"]["type"], "noul");
        assert_eq!(
            sent["questions"]["irreversible"]["criteria"]["true"],
            "Sends, deletes, purchases, or submits"
        );
        assert_eq!(sent["questions"]["kind"]["criteria"]["other"], Value::Null);
        assert_eq!(sent["questions"]["risk"]["criteria"][2], "high");

        assert_eq!(got.model, "jev-1.13.0");
        assert_eq!(got.answers["irreversible"], Answer::Noul { noul: 0.93 });
        assert!(
            matches!(&got.answers["kind"], Answer::Choice { choice, .. } if choice.as_deref() == Some("submit"))
        );
        assert!(
            matches!(&got.answers["risk"], Answer::Score { score, legend, .. } if *score == 1.4 && legend["2"] == "high")
        );
        assert_eq!(
            got.usage,
            DecisionUsage {
                input_tokens: 120,
                output_tokens: 25
            }
        );
    }

    #[tokio::test]
    async fn evaluate_surfaces_http_errors_as_api_errors() {
        let (url, _server) = serve_once(
            "422 Unprocessable Entity",
            r#"{"error":"questions.kind.criteria"}"#,
        )
        .await;
        let cfg = DecisionModel::new(&url, "sk-test", "");
        let err = evaluate(&cfg, &json!("x"), &BTreeMap::new())
            .await
            .unwrap_err();
        assert!(
            matches!(err, LlmError::Api { status: 422, ref body } if body.contains("questions.kind"))
        );
    }

    #[tokio::test]
    async fn keyless_startlux_server_gets_think_mode_and_may_answer_none() {
        let (url, server) = serve_once(
            "200 OK",
            r#"{"model":"StartLux-Decision-4B","answers":{
                "team":{"type":"choice","choice":null,"confidence":0.7,"probabilities":{"billing":0.2,"shipping":0.1},"none_probability":0.7}
            },"usage":{"input_tokens":50,"output_tokens":12},"timing_ms":{"wall":40}}"#,
        )
        .await;
        let mut cfg = DecisionModel::new(&url, " ", "");
        cfg.think_mode = "never".into();
        let questions = BTreeMap::from([(
            "team".to_string(),
            Question::Choice {
                instructions: "Which team owns `ticket`?".into(),
                criteria: BTreeMap::from([
                    ("billing".to_string(), None),
                    ("shipping".to_string(), None),
                ]),
            },
        )]);

        let got = evaluate(&cfg, &json!({"ticket": "x"}), &questions)
            .await
            .unwrap();

        let raw = server.await.unwrap();
        assert!(
            !raw.to_ascii_lowercase().contains("authorization:"),
            "{raw}"
        );
        let sent: Value = serde_json::from_str(&raw[raw.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(sent["think_mode"], "never");
        assert!(sent.get("model").is_none(), "{sent}");
        assert_eq!(got.model, "StartLux-Decision-4B");
        assert!(matches!(
            &got.answers["team"],
            Answer::Choice { choice: None, confidence, .. } if *confidence == 0.7
        ));
    }

    #[test]
    fn from_env_prefers_a_named_server_over_the_typesafe_key() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert!(DecisionModel::from_env(env(&[])).is_none());
        assert!(DecisionModel::from_env(env(&[("TYPESAFE_API_KEY", "  ")])).is_none());

        let typesafe = DecisionModel::from_env(env(&[("TYPESAFE_API_KEY", "ts-key")])).unwrap();
        assert_eq!(typesafe.http.base_url, DEFAULT_BASE_URL);
        assert_eq!(typesafe.http.model, TYPESAFE_MODEL);
        assert_eq!(typesafe.http.api_key, "ts-key");
        assert_eq!(typesafe.timeout, DEFAULT_TIMEOUT);

        let startlux = DecisionModel::from_env(env(&[
            ("TYPESAFE_API_KEY", "ts-key"),
            ("WISP_DECISION_API_URL", "https://api.bamboonode.dev"),
            ("WISP_DECISION_API_KEY", "bk-key"),
            ("WISP_DECISION_THINK_MODE", "auto"),
            ("WISP_DECISION_TIMEOUT_SECS", "9000"),
        ]))
        .unwrap();
        assert_eq!(startlux.http.api_key, "bk-key");
        assert_eq!(startlux.http.model, "");
        assert_eq!(startlux.think_mode, "auto");
        assert_eq!(startlux.timeout, Duration::from_secs(600));
        assert_eq!(startlux.host(), "api.bamboonode.dev");
        assert_eq!(DecisionModel::new("", "k", "").host(), "api.typesafe.ai");
        assert_eq!(
            DecisionModel::new("http://127.0.0.1:8090/v1", "", "").host(),
            "127.0.0.1:8090"
        );
    }
}
