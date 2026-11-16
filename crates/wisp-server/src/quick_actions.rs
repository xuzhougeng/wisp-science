//! Quick actions: requests the user asks often, kept under a name and put
//! into the composer with one click, or workflows of the library started
//! as agents on what was typed, as the WebView's quick actions start
//! theirs.
//!
//! The WebView's actions name its own workflow templates, so these are
//! kept under a key of this desktop's own. The WebView's are never
//! changed; they are read only to bring over, with the workflows they
//! start, those this desktop can hold as they are.

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use wisp_store::Store;

pub(crate) const KEY: &str = "desktop.quick_actions";
pub(crate) const NAME_LIMIT: usize = 60;
pub(crate) const PROMPT_LIMIT: usize = 8_000;
pub(crate) const LIMIT: usize = 40;

/// A named request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickAction {
    pub id: String,
    pub name: String,
    /// What is put into the composer, or, with a workflow, told to its
    /// tasks before what was typed. Optional with a workflow.
    pub prompt: String,
    /// The library's workflow it starts as agents; empty puts the prompt
    /// into the composer instead.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow: String,
}

/// What the form sends: `id` empty adds an action.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickActionInput {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub workflow: String,
}

/// What a desktop that never saved any starts with. They are the user's
/// from then on: changed or removed like any other.
pub(crate) fn starters() -> Vec<QuickAction> {
    [
        (
            "literature",
            "Research the literature",
            "Search the literature on the question below. Use the literature tools to find \
             the relevant papers, read what you can reach, and report what is established, \
             what is contested and what is unknown, each with its sources. Say which claims \
             you could not verify.\n\nQuestion: ",
        ),
        (
            "design",
            "Design the analysis",
            "Look at the data in this project and propose an analysis plan for the question \
             below: what to measure, which comparisons, which controls and confounders, and \
             what result would change the conclusion. Do not run anything yet.\n\nQuestion: ",
        ),
        (
            "interpretations",
            "Compare interpretations",
            "Give the three most plausible interpretations of the result below. For each, \
             say what it predicts that the others do not, and which check on the data in \
             this project would tell them apart.\n\nResult: ",
        ),
        (
            "summary",
            "Summarise where we are",
            "Summarise this conversation so far: what was asked, what was done, what was \
             found, what is still uncertain, and the next step you would take.",
        ),
    ]
    .into_iter()
    .map(|(id, name, prompt)| QuickAction {
        id: format!("starter-{id}"),
        name: name.into(),
        prompt: prompt.into(),
        workflow: String::new(),
    })
    .collect()
}

pub(crate) async fn list(store: &Store) -> Vec<QuickAction> {
    match store.get_setting(KEY).await.ok().flatten() {
        // Saved, even as an empty list: the user removed them all.
        Some(saved) => serde_json::from_str(&saved).unwrap_or_default(),
        None => starters(),
    }
}

pub(crate) async fn save(store: &Store, actions: &[QuickAction]) -> Result<()> {
    store
        .set_setting(KEY, &serde_json::to_string(actions)?)
        .await
}

/// Adds an action or changes one.
pub(crate) async fn upsert(store: &Store, input: QuickActionInput) -> Result<Vec<QuickAction>> {
    let name = input.name.trim();
    // What follows a prompt is typed after it: its end is kept as written.
    let prompt = input.prompt.trim_start();
    ensure!(!name.is_empty(), "Give the action a name");
    ensure!(
        name.chars().count() <= NAME_LIMIT,
        "A name is at most {NAME_LIMIT} characters"
    );
    let workflow = input.workflow.trim();
    ensure!(
        !prompt.trim().is_empty() || !workflow.is_empty(),
        "Write what the action asks, or choose a workflow it starts"
    );
    ensure!(
        workflow.is_empty()
            || crate::workflows::list(store)
                .await
                .iter()
                .any(|known| known.id == workflow),
        "That workflow is no longer there; choose another"
    );
    ensure!(
        prompt.chars().count() <= PROMPT_LIMIT,
        "A request is at most {PROMPT_LIMIT} characters"
    );
    let mut actions = list(store).await;
    let taken = actions.iter().any(|action| {
        action.id != input.id && action.name.trim().to_lowercase() == name.to_lowercase()
    });
    ensure!(!taken, "Another action is already called \"{name}\"");
    if input.id.is_empty() {
        ensure!(
            actions.len() < LIMIT,
            "There are already {LIMIT} quick actions; remove one first"
        );
        actions.push(QuickAction {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            prompt: prompt.into(),
            workflow: workflow.into(),
        });
    } else {
        let Some(action) = actions.iter_mut().find(|action| action.id == input.id) else {
            bail!("That action is no longer there");
        };
        action.name = name.into();
        action.prompt = prompt.into();
        action.workflow = workflow.into();
    }
    save(store, &actions).await?;
    Ok(actions)
}

pub(crate) async fn remove(store: &Store, id: &str) -> Result<Vec<QuickAction>> {
    let mut actions = list(store).await;
    let before = actions.len();
    actions.retain(|action| action.id != id);
    ensure!(actions.len() < before, "That action is no longer there");
    save(store, &actions).await?;
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str, name: &str, prompt: &str) -> QuickActionInput {
        QuickActionInput {
            id: id.into(),
            name: name.into(),
            prompt: prompt.into(),
            workflow: String::new(),
        }
    }

    #[tokio::test]
    async fn actions_start_with_a_few_and_are_the_users_to_change_and_remove() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        let start = list(&store).await;
        assert_eq!(start.len(), 4);
        assert!(start.iter().all(|action| action.id.starts_with("starter-")));

        let refused = |input: QuickActionInput| {
            let store = &store;
            async move { upsert(store, input).await.unwrap_err().to_string() }
        };
        assert!(refused(input("", " ", "x")).await.contains("name"));
        assert!(refused(input("", "Mine", "  ")).await.contains("asks"));
        assert!(refused(input("", "summarise WHERE we are", "x"))
            .await
            .contains("already called"));
        assert!(refused(input("ghost", "Mine", "x"))
            .await
            .contains("no longer"));

        // A request that ends with a place to type keeps it.
        let added = upsert(&store, input("", " QC report ", "  Check the QC of: "))
            .await
            .unwrap();
        assert_eq!(added.len(), 5);
        let mine = added.last().unwrap().clone();
        assert_eq!(
            (mine.name.as_str(), mine.prompt.as_str()),
            ("QC report", "Check the QC of: ")
        );
        // A starter is changed like any other.
        let changed = upsert(&store, input("starter-summary", "Recap", "Recap this."))
            .await
            .unwrap();
        assert!(changed
            .iter()
            .any(|a| a.name == "Recap" && a.prompt == "Recap this."));
        // Removed for good: the starters do not come back.
        for action in list(&store).await {
            remove(&store, &action.id).await.unwrap();
        }
        assert!(list(&store).await.is_empty());
        assert!(remove(&store, &mine.id).await.is_err());
    }

    #[tokio::test]
    async fn an_action_may_start_a_workflow_of_the_library_instead() {
        use crate::workflows::{Workflow, WorkflowTask};
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        let library = crate::workflows::upsert(
            &store,
            Workflow {
                name: "Evidence review".into(),
                goal: "Weigh the evidence".into(),
                tasks: vec![WorkflowTask {
                    title: "Search".into(),
                    instructions: "Find the papers.".into(),
                    after: Vec::new(),
                    capabilities: Vec::new(),
                    agent: Default::default(),
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let review = library[0].id.clone();
        // With a workflow, what it asks may be left out.
        let saved = upsert(
            &store,
            QuickActionInput {
                workflow: review.clone(),
                ..input("", "Review it", "")
            },
        )
        .await
        .unwrap();
        let action = saved.last().unwrap();
        assert_eq!(
            (action.workflow.as_str(), action.prompt.as_str()),
            (review.as_str(), "")
        );
        // Only a workflow the library has.
        let refused = upsert(
            &store,
            QuickActionInput {
                workflow: "gone".into(),
                ..input("", "Other", "")
            },
        )
        .await
        .unwrap_err();
        assert!(refused.to_string().contains("no longer there"), "{refused}");
        // Changed back to a request, it needs one.
        assert!(upsert(&store, input(&action.id, "Review it", ""))
            .await
            .is_err());
        // An action saved before workflows were offered reads as a request.
        let old: QuickAction =
            serde_json::from_str(r#"{"id":"a","name":"Old","prompt":"Do it."}"#).unwrap();
        assert!(old.workflow.is_empty());
    }
}
