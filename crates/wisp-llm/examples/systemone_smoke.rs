//! Manual live check of the System One decision protocol against a real server.
//! Run with `TYPESAFE_API_KEY=... cargo run -p wisp-llm --example systemone_smoke`,
//! or point it at another server with `WISP_DECISION_API_URL` (plus
//! `WISP_DECISION_API_KEY` / `WISP_DECISION_MODEL` / `WISP_DECISION_THINK_MODE`).
use std::collections::BTreeMap;
use wisp_llm::systemone::{evaluate, DecisionModel, NoulCriteria, Question};

#[tokio::main]
async fn main() {
    let Some(cfg) = DecisionModel::from_env(|k| std::env::var(k).ok()) else {
        println!("set TYPESAFE_API_KEY or WISP_DECISION_API_URL");
        return;
    };
    let questions = BTreeMap::from([
        (
            "irreversible".to_string(),
            Question::Noul {
                instructions: "Does `action` commit an irreversible external change?".into(),
                criteria: Some(NoulCriteria {
                    yes: "Sends, posts, purchases, deletes, or submits something".into(),
                    no: "Only reads, navigates, or edits a local draft".into(),
                }),
            },
        ),
        (
            "kind".to_string(),
            Question::Choice {
                instructions: "What kind of desktop action is `action`?".into(),
                criteria: BTreeMap::from([
                    (
                        "navigate".to_string(),
                        Some("Opens, scrolls, or switches views".to_string()),
                    ),
                    (
                        "edit".to_string(),
                        Some("Changes local content".to_string()),
                    ),
                    (
                        "commit".to_string(),
                        Some("Sends or submits to an external party".to_string()),
                    ),
                ]),
            },
        ),
        (
            "risk".to_string(),
            Question::Score {
                instructions: "How costly is `action` if it was a mistake?".into(),
                criteria: vec![
                    "Harmless".into(),
                    "Annoying to undo".into(),
                    "Cannot be undone".into(),
                ],
            },
        ),
    ]);
    let state = serde_json::json!({
        "app": "Mail",
        "window": "New Message — To: lab@example.org",
        "action": "click the Send button",
    });
    match evaluate(&cfg, &state, &questions).await {
        Ok(decisions) => println!("{decisions:#?}"),
        Err(error) => println!("{error}"),
    }
}
