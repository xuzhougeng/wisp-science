//! Experts: personas a conversation can be started as. An expert is a name
//! and the instructions the model works under for the whole conversation.
//!
//! The list and each conversation's choice are kept where the WebView
//! desktop keeps its specialists (`specialists`, `frame_specialist:<id>`),
//! in its shape, so both read the same experts. Fields this host does not
//! use (an expert's own model, its skill and connector lists, the
//! reviewer's backend) are carried through untouched. The WebView's
//! one-shot built-ins (reviewer, reader, archivist, recap) are not
//! personas to converse as and are not offered here.

use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use wisp_store::Store;

const KEY: &str = "specialists";
pub(crate) const NAME_LIMIT: usize = 60;
pub(crate) const DESCRIPTION_LIMIT: usize = 300;
pub(crate) const INSTRUCTIONS_LIMIT: usize = 20_000;
/// Built-ins that draft one document and are never conversed with.
const ONE_SHOT: [&str; 4] = ["reviewer", "reader", "archivist", "recap"];
const ILLUSTRATOR: &str = "scientific_illustrator";
const HEADING: &str = "\n\n## Specialist: ";

/// The WebView desktop's rubric for its built-in illustrator, without the
/// image-model mode: this host has no image-generation tool, and says so.
const ILLUSTRATOR_RUBRIC: &str = "\
You are the Scientific Illustrator. Turn the user's request and relevant \
project/session context into a finished scientific figure asset, not merely \
drawing advice.\n\n\
Inspect referenced data and files before drawing. Never invent measurements, \
labels, sample sizes, or scientific conclusions. Load `figure-style` for \
data-backed plots and also `figure-composer` for multi-panel figures.\n\n\
Create the actual publication-ready figure as a descriptive `figures/*.svg` \
file using `write`, Python, or R. After writing the SVG, rasterize that exact \
SVG to a PNG preview, inspect the preview, fix visible problems in the SVG \
source, then re-render and re-inspect until the figure is legible and \
unclipped. The SVG is the primary deliverable; the PNG is only a QA preview. \
When the user explicitly asks for an image-model PNG, say that this desktop \
has no image-generation tool yet rather than substituting silently.\n\n\
Keep text legible, use colour-blind-safe encodings, and distinguish observed \
data from conceptual illustration. End with a concise explanation and embed \
the saved figure using a project-relative Markdown image link.";

/// One saved row, in the WebView desktop's shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Row {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    builtin: bool,
    /// Everything else the WebView desktop keeps on a specialist.
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

/// An expert as the desktop lists and edits it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expert {
    pub id: String,
    pub name: String,
    /// For the user: what the expert is for. The model is not shown it.
    pub description: String,
    /// What the model works under in a conversation started as this expert.
    pub instructions: String,
    /// Ships with Wisp: its instructions cannot be changed or removed.
    pub builtin: bool,
}

/// What the form sends: `id` empty adds an expert.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpertInput {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
}

fn illustrator() -> Row {
    Row {
        id: ILLUSTRATOR.into(),
        name: "Scientific Illustrator".into(),
        description:
            "Creates publication-ready scientific figures from the request and project context."
                .into(),
        instructions: ILLUSTRATOR_RUBRIC.into(),
        builtin: true,
        rest: Default::default(),
    }
}

async fn load(store: &Store) -> Vec<Row> {
    let mut rows: Vec<Row> = store
        .get_setting(KEY)
        .await
        .ok()
        .flatten()
        .and_then(|saved| serde_json::from_str(&saved).ok())
        .unwrap_or_default();
    // The illustrator is there whether or not anything was saved, and its
    // instructions are this build's.
    match rows.iter_mut().find(|row| row.id == ILLUSTRATOR) {
        Some(row) => {
            row.builtin = true;
            row.instructions = ILLUSTRATOR_RUBRIC.into();
        }
        None => rows.insert(0, illustrator()),
    }
    rows
}

async fn save(store: &Store, rows: &[Row]) -> Result<()> {
    store.set_setting(KEY, &serde_json::to_string(rows)?).await
}

fn shown(row: &Row) -> Option<Expert> {
    (!ONE_SHOT.contains(&row.id.as_str())).then(|| Expert {
        id: row.id.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        instructions: row.instructions.clone(),
        builtin: row.builtin,
    })
}

/// The experts a conversation can be started as, built-in first.
pub(crate) async fn list(store: &Store) -> Vec<Expert> {
    load(store).await.iter().filter_map(shown).collect()
}

pub(crate) async fn get(store: &Store, id: &str) -> Option<Expert> {
    list(store).await.into_iter().find(|expert| expert.id == id)
}

/// Adds an expert or changes one. A built-in keeps its instructions.
pub(crate) async fn upsert(store: &Store, input: ExpertInput) -> Result<Vec<Expert>> {
    let name = input.name.trim();
    let description = input.description.trim();
    let instructions = input.instructions.trim();
    ensure!(!name.is_empty(), "Give the expert a name");
    ensure!(
        name.chars().count() <= NAME_LIMIT,
        "A name is at most {NAME_LIMIT} characters"
    );
    ensure!(
        description.chars().count() <= DESCRIPTION_LIMIT,
        "A description is at most {DESCRIPTION_LIMIT} characters"
    );
    ensure!(
        instructions.chars().count() <= INSTRUCTIONS_LIMIT,
        "Instructions are at most {INSTRUCTIONS_LIMIT} characters"
    );
    let mut rows = load(store).await;
    let taken = rows
        .iter()
        .any(|row| row.id != input.id && row.name.trim().to_lowercase() == name.to_lowercase());
    ensure!(!taken, "Another expert is already called \"{name}\"");
    if input.id.is_empty() {
        ensure!(
            !instructions.is_empty(),
            "Write the instructions the expert works under"
        );
        rows.push(Row {
            // Never handed out again: a conversation may still name an
            // expert that was removed.
            id: format!("sp-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            name: name.into(),
            description: description.into(),
            instructions: instructions.into(),
            builtin: false,
            rest: Default::default(),
        });
    } else {
        let Some(row) = rows
            .iter_mut()
            .find(|row| row.id == input.id && !ONE_SHOT.contains(&row.id.as_str()))
        else {
            bail!("That expert is no longer there");
        };
        row.name = name.into();
        row.description = description.into();
        if !row.builtin {
            ensure!(
                !instructions.is_empty(),
                "Write the instructions the expert works under"
            );
            row.instructions = instructions.into();
        }
    }
    save(store, &rows).await?;
    Ok(rows.iter().filter_map(shown).collect())
}

/// Removes an expert of the user's own. Conversations started as it keep
/// the instructions they were given.
pub(crate) async fn remove(store: &Store, id: &str) -> Result<Vec<Expert>> {
    let mut rows = load(store).await;
    let Some(row) = rows.iter().find(|row| row.id == id) else {
        bail!("That expert is no longer there");
    };
    ensure!(
        !row.builtin,
        "An expert that ships with Wisp cannot be removed"
    );
    rows.retain(|row| row.id != id);
    save(store, &rows).await?;
    Ok(rows.iter().filter_map(shown).collect())
}

pub(crate) fn frame_key(session: &str) -> String {
    format!("frame_specialist:{session}")
}

/// The expert a conversation was started as, if it was and it still exists.
pub(crate) async fn of_conversation(store: &Store, session: &str) -> Option<Expert> {
    let id = store
        .get_setting(&frame_key(session))
        .await
        .ok()
        .flatten()?;
    get(store, id.trim()).await
}

/// Adds the expert's instructions to a system prompt that has none yet.
/// They are written once, when the conversation starts, and stay as they
/// were then: changing or removing the expert later does not rewrite a
/// conversation that is under way.
pub(crate) fn seed(prompt: &mut String, expert: &Expert) {
    if prompt.contains(HEADING) || expert.instructions.trim().is_empty() {
        return;
    }
    prompt.push_str(HEADING);
    prompt.push_str(expert.name.trim());
    prompt.push('\n');
    prompt.push_str(expert.instructions.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (tempfile::TempDir, Store) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(&temp.path().join("wisp.sqlite")).await.unwrap();
        (temp, store)
    }

    fn input(id: &str, name: &str, instructions: &str) -> ExpertInput {
        ExpertInput {
            id: id.into(),
            name: name.into(),
            description: "For the lab".into(),
            instructions: instructions.into(),
        }
    }

    #[tokio::test]
    async fn experts_are_added_changed_and_removed_and_the_built_in_stays() {
        let (_temp, store) = store().await;
        let start = list(&store).await;
        assert_eq!(start.len(), 1);
        assert!(start[0].builtin && start[0].id == ILLUSTRATOR);

        let refused = |input: ExpertInput| {
            let store = &store;
            async move { upsert(store, input).await.unwrap_err().to_string() }
        };
        assert!(refused(input("", "  ", "x")).await.contains("name"));
        assert!(refused(input("", "Stats", " "))
            .await
            .contains("instructions"));
        assert!(refused(input("", &"n".repeat(NAME_LIMIT + 1), "x"))
            .await
            .contains("at most"));
        assert!(refused(input("", "scientific illustrator", "x"))
            .await
            .contains("already called"));
        assert!(refused(input("ghost", "Stats", "x"))
            .await
            .contains("no longer"));

        let added = upsert(
            &store,
            input("", " Statistician ", " Report effect sizes. "),
        )
        .await
        .unwrap();
        let stats = added
            .iter()
            .find(|e| e.name == "Statistician")
            .unwrap()
            .clone();
        assert!(stats.id.starts_with("sp-") && !stats.builtin);
        assert_eq!(stats.instructions, "Report effect sizes.");
        // Changed in place.
        let changed = upsert(&store, input(&stats.id, "Statistician", "Use medians."))
            .await
            .unwrap();
        assert_eq!(changed.len(), 2);
        assert_eq!(
            get(&store, &stats.id).await.unwrap().instructions,
            "Use medians."
        );
        // The built-in can be renamed, never rewritten or removed.
        upsert(&store, input(ILLUSTRATOR, "Illustrator", "Draw badly."))
            .await
            .unwrap();
        let illustrator = get(&store, ILLUSTRATOR).await.unwrap();
        assert_eq!(illustrator.name, "Illustrator");
        assert!(illustrator
            .instructions
            .starts_with("You are the Scientific Illustrator"));
        assert!(remove(&store, ILLUSTRATOR).await.is_err());

        assert_eq!(remove(&store, &stats.id).await.unwrap().len(), 1);
        assert!(remove(&store, &stats.id).await.is_err());
    }

    #[tokio::test]
    async fn the_webview_desktops_rows_are_read_and_what_it_keeps_is_kept() {
        let (_temp, store) = store().await;
        let saved = serde_json::json!([
            {"id":"reviewer","name":"Reviewer","instructions":"trace","builtin":true,
             "model_id":"m2","review_backend":{"kind":"http_model","profile_id":"m2"}},
            {"id":"sp-1","name":"Statistician","icon":"chart","color":"clay",
             "description":"numbers","instructions":"Report medians.","model_id":"m9",
             "skills":["stats"],"connectors":null,"builtin":false}
        ]);
        store.set_setting(KEY, &saved.to_string()).await.unwrap();
        // The reviewer is not a persona; the user's expert is.
        let names: Vec<_> = list(&store).await.into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["Scientific Illustrator", "Statistician"]);
        assert!(upsert(&store, input("reviewer", "R", "x")).await.is_err());

        upsert(&store, input("sp-1", "Statistician", "Report means."))
            .await
            .unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&store.get_setting(KEY).await.unwrap().unwrap()).unwrap();
        let rows = raw.as_array().unwrap();
        let row = |id: &str| rows.iter().find(|row| row["id"] == id).unwrap();
        // Its model and skills, and the reviewer's backend, are as they were.
        assert_eq!(row("sp-1")["instructions"], "Report means.");
        assert_eq!(row("sp-1")["model_id"], "m9");
        assert_eq!(row("sp-1")["skills"], serde_json::json!(["stats"]));
        assert_eq!(row("sp-1")["icon"], "chart");
        assert_eq!(row("reviewer")["review_backend"]["profile_id"], "m2");
    }

    #[tokio::test]
    async fn a_conversations_expert_is_read_by_the_webviews_key() {
        let (_temp, store) = store().await;
        assert_eq!(of_conversation(&store, "s1").await, None);
        store
            .set_setting("frame_specialist:s1", ILLUSTRATOR)
            .await
            .unwrap();
        assert_eq!(of_conversation(&store, "s1").await.unwrap().id, ILLUSTRATOR);
        // One that was removed since is simply no expert.
        store
            .set_setting("frame_specialist:s2", "sp-gone")
            .await
            .unwrap();
        assert_eq!(of_conversation(&store, "s2").await, None);
    }

    #[test]
    fn instructions_are_written_into_a_prompt_once() {
        let expert = Expert {
            id: "sp-1".into(),
            name: " Statistician ".into(),
            description: "not for the model".into(),
            instructions: " Report medians. ".into(),
            builtin: false,
        };
        let mut prompt = String::from("You are Wisp.");
        seed(&mut prompt, &expert);
        seed(&mut prompt, &expert);
        assert_eq!(
            prompt,
            "You are Wisp.\n\n## Specialist: Statistician\nReport medians."
        );
        // Another expert later does not replace the one it started as.
        let other = Expert {
            name: "Other".into(),
            ..expert.clone()
        };
        seed(&mut prompt, &other);
        assert!(!prompt.contains("Other"));
        // Nothing to say: nothing is written.
        let mut plain = String::from("You are Wisp.");
        seed(
            &mut plain,
            &Expert {
                instructions: "  ".into(),
                ..expert
            },
        );
        assert_eq!(plain, "You are Wisp.");
    }
}
