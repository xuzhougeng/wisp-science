//! The research assistant: one never-ending conversation that belongs to no
//! project. It reports recorded activity across projects, remembers the
//! researcher's plan, and hands work to project conversations. It never does
//! the work itself — its turns get only the tools below: no files, shell,
//! kernels, browser or MCP.
//!
//! The conversation lives in the hidden `ASSISTANT_PROJECT_ID` project as the
//! fixed frame `ASSISTANT_FRAME_ID`, so it reuses the ordinary turn pipeline,
//! paging and archive-first compaction. Plan items live in `assistant_tasks`,
//! outside the context, so compaction never loses them.

use crate::{create_session_frame, send_message_inner, AppState};
use async_trait::async_trait;
use chrono::{Local, NaiveDate};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tauri::{AppHandle, Manager, State};
use wisp_llm::ToolSchema;
use wisp_store::{AssistantTask, Store, ASSISTANT_PROJECT_ID};
use wisp_tools::{Registry, Tool, ToolEnv, ToolResult};

pub(crate) const ASSISTANT_FRAME_ID: &str = "research-assistant";
/// Output ceilings: a week of digests across many projects stays readable.
const MAX_ACTIVITY_DAYS: i64 = 7;
const MAX_ACTIVITY_CHARS: usize = 60_000;
const MAX_RESULT_CHARS: usize = 6_000;

pub(crate) const ASSISTANT_SYSTEM: &str = "\
You are the researcher's research assistant in Wisp Science. This is one long-running conversation that \
belongs to no project. You keep track of the research and organize it; you never do the research work \
yourself.\n\n\
What you do:\n\
- Report: `research_activity` reads what was recorded across projects (runs, outputs, notes, decisions, \
conversations and daily recaps). State only what the records show; never invent results, numbers or causes. \
Quiet days are simply quiet.\n\
- Remember the plan: when the researcher says what they intend to do, save each item with `research_plan` \
(add). Mark items done or dropped when told, or when the records clearly show it. Before planning or \
reporting a day, list the plan — it also carries unfinished items forward.\n\
- Dispatch: when asked to get something done in a project, find it with `research_projects`, then call \
`dispatch_to_project` with a complete, self-contained instruction; that project's own agent does the work in \
a new conversation there. Pass `plan_item_id` when the work comes from a plan item.\n\
- Follow up: `project_session_result` tells whether a dispatched conversation is still running and what it \
answered.\n\n\
Rules:\n\
- You cannot read or write files, run code, commands or analyses, or search literature. When the researcher \
asks for such work, dispatch it to the right project (ask which one if unclear) instead of attempting it.\n\
- Never claim work was done unless a record or a dispatched conversation's answer shows it.\n\
- The latest message carries the current local date and time; resolve \"today\" and \"yesterday\" from it, \
never from older messages.\n\
- Be brief: short lists, the project name on each item, no filler. Reply in the researcher's language.";

/// Injected before each assistant turn's message (not persisted): a
/// never-ending conversation has no fixed "today" to anchor dates to.
pub(crate) fn now_note() -> String {
    format!("Local time: {}", Local::now().format("%Y-%m-%d %A %H:%M"))
}

pub(crate) fn tools(app: &AppHandle) -> Registry {
    let store = app.state::<AppState>().store.clone();
    // No built-ins: the assistant has no files, shell or images to work with.
    let mut tools = Registry::builtins().filtered(&[]);
    tools.add(Box::new(ProjectsTool {
        store: store.clone(),
    }));
    tools.add(Box::new(ActivityTool {
        store: store.clone(),
    }));
    tools.add(Box::new(PlanTool {
        store: store.clone(),
    }));
    tools.add(Box::new(DispatchTool { app: app.clone() }));
    tools.add(Box::new(SessionResultTool { app: app.clone() }));
    tools
}

/// Create the hidden project and its one conversation on first use.
pub(crate) async fn ensure(store: &Store, app_data: &std::path::Path) -> Result<(), String> {
    let err = |error: anyhow::Error| error.to_string();
    if store
        .get_project(ASSISTANT_PROJECT_ID)
        .await
        .map_err(err)?
        .is_none()
    {
        let dir = app_data.join("assistant");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create the assistant folder: {e}"))?;
        store
            .create_project(
                ASSISTANT_PROJECT_ID,
                "Research assistant",
                &dir.to_string_lossy(),
            )
            .await
            .map_err(err)?;
    }
    if store
        .frame_project_id(ASSISTANT_FRAME_ID)
        .await
        .map_err(err)?
        .is_none()
    {
        let model_id = crate::models::active_profile_id(store).await;
        store
            .create_frame(
                ASSISTANT_FRAME_ID,
                ASSISTANT_PROJECT_ID,
                "OPERON",
                &model_id,
            )
            .await
            .map_err(err)?;
    }
    Ok(())
}

/// Bind this window to the assistant's conversation. The window's previous
/// project is restored on close; reopening keeps the first one remembered.
#[tauri::command]
pub(crate) async fn open_research_assistant(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<String, String> {
    let label = window.label().to_string();
    let bound = state.require_active(&label).map(|ap| ap.id);
    let restore = match bound {
        Ok(id) if wisp_store::is_assistant_project_id(&id) => None,
        Ok(id) => Some(Some(id)),
        Err(_) if crate::is_blank_window_label(&label) => Some(None),
        Err(error) => return Err(error),
    };
    ensure(&state.store, &state.app_data).await?;
    let (ap, _, _) =
        crate::project_commands::load_active_project(state.inner(), ASSISTANT_PROJECT_ID).await?;
    state.set_active(&label, ap);
    state.set_active_frame(&label, Some(ASSISTANT_FRAME_ID.into()));
    if let Some(restore) = restore {
        state
            .assistant_windows
            .write()
            .unwrap()
            .entry(label)
            .or_insert(restore);
    }
    Ok(ASSISTANT_FRAME_ID.into())
}

/// Return the window to what it showed before. A running assistant turn
/// keeps going; only the window binding changes.
#[tauri::command]
pub(crate) async fn close_research_assistant(
    state: State<'_, AppState>,
    window: crate::workspace_surface::WorkspaceSurface,
) -> Result<(), String> {
    let label = window.label();
    let Some(restore) = state.assistant_windows.write().unwrap().remove(label) else {
        return Ok(());
    };
    if state.active_frame(label).as_deref() == Some(ASSISTANT_FRAME_ID) {
        state.set_active_frame(label, None);
    }
    match restore {
        Some(project_id) => {
            let _ = crate::project_commands::set_active_project(state.inner(), label, &project_id)
                .await;
        }
        None => {
            state.active.write().unwrap().remove(label);
        }
    }
    Ok(())
}

fn clip(text: &str, chars: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_string(),
    }
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// `YYYY-MM-DD`, or `today` / `yesterday` / `tomorrow`; absent means today.
fn parse_day(value: Option<&str>, today: NaiveDate) -> Result<NaiveDate, String> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None | Some("today") => Ok(today),
        Some("yesterday") => Ok(today - chrono::Days::new(1)),
        Some("tomorrow") => Ok(today + chrono::Days::new(1)),
        Some(day) => NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|_| format!("'{day}' is not a date; use YYYY-MM-DD")),
    }
}

/// Projects the researcher can see: no internal projects, none hidden by
/// privacy mode. `(id, name, description, updated_at)`.
async fn visible_projects(store: &Store) -> Result<Vec<(String, String, String, i64)>, String> {
    let privacy = crate::privacy_mode::load(store).await?;
    let hidden: HashSet<String> = if privacy.active {
        privacy.project_ids.into_iter().collect()
    } else {
        HashSet::new()
    };
    Ok(store
        .list_projects()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|p| !hidden.contains(&p.0))
        .map(|p| (p.0, p.1, p.6, p.4))
        .collect())
}

/// The assistant's sidebars must resolve the authoritative privacy setting
/// before they expose project names or request any calendar history.
#[tauri::command]
pub(crate) async fn get_research_assistant_projects(
    state: State<'_, AppState>,
) -> Result<Vec<wisp_dto::ProjectSummary>, String> {
    let visible: HashSet<_> = visible_projects(&state.store)
        .await?
        .into_iter()
        .map(|project| project.0)
        .collect();
    Ok(crate::project_commands::list_projects(state)
        .await?
        .into_iter()
        .filter(|project| visible.contains(&project.id))
        .collect())
}

async fn visible_plan(
    store: &Store,
    day: &str,
) -> Result<Vec<wisp_dto::ResearchAssistantPlanItem>, String> {
    let day = NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| "Use a calendar date in YYYY-MM-DD format".to_string())?
        .to_string();
    let names: HashMap<_, _> = visible_projects(store)
        .await?
        .into_iter()
        .map(|project| (project.0, project.1))
        .collect();
    Ok(store
        .assistant_tasks(&day, &day)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|task| {
            task.project_id
                .as_ref()
                .is_none_or(|id| names.contains_key(id))
        })
        .map(|task| wisp_dto::ResearchAssistantPlanItem {
            project_name: task
                .project_id
                .as_ref()
                .and_then(|id| names.get(id).cloned()),
            id: task.id,
            day: task.day,
            title: task.title,
            project_id: task.project_id,
            session_id: task.session_id,
            status: task.status,
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn get_research_assistant_plan(
    state: State<'_, AppState>,
    day: String,
) -> Result<Vec<wisp_dto::ResearchAssistantPlanItem>, String> {
    visible_plan(&state.store, &day).await
}

fn local_date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

struct ProjectsTool {
    store: Store,
}

#[async_trait]
impl Tool for ProjectsTool {
    fn name(&self) -> &str {
        "research_projects"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "research_projects",
            "List the researcher's projects (id, name, last activity date, description). Use the id with dispatch_to_project or research_activity.",
            json!({"type": "object", "properties": {}}),
        )
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn run(&self, _args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match visible_projects(&self.store).await {
            Ok(projects) if projects.is_empty() => ToolResult::ok("No projects yet."),
            Ok(projects) => ToolResult::ok(
                projects
                    .iter()
                    .map(|(id, name, description, updated)| {
                        let mut line =
                            format!("- {name} (id: {id}, last active {})", local_date(*updated));
                        if !description.trim().is_empty() {
                            line.push_str(&format!(": {}", clip(description, 160)));
                        }
                        line
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(error) => ToolResult::fail(error),
        }
    }
}

struct ActivityTool {
    store: Store,
}

/// Recorded activity per local day and project in `[from, until]`: the
/// day's recap when one was kept, plus the same digest a recap is drafted from.
async fn activity(
    store: &Store,
    from: NaiveDate,
    until: NaiveDate,
    project_id: Option<&str>,
) -> Result<String, String> {
    if until < from {
        return Err("'until' is before 'from'".into());
    }
    if (until - from).num_days() >= MAX_ACTIVITY_DAYS {
        return Err(format!(
            "Ask for at most {MAX_ACTIVITY_DAYS} days at a time."
        ));
    }
    let projects: Vec<_> = visible_projects(store)
        .await?
        .into_iter()
        .filter(|p| project_id.is_none_or(|id| id == p.0))
        .collect();
    if let (Some(id), true) = (project_id, projects.is_empty()) {
        return Err(format!("No visible project has id '{id}'."));
    }
    let mut days = Vec::new();
    let mut day = from;
    while day <= until {
        let (start, end) = crate::research_recap::local_day(day)
            .ok_or_else(|| format!("{day} has no local midnight"))?;
        let mut rows = Vec::new();
        for (id, name, _, _) in &projects {
            let recap = store
                .research_recaps(id, start, end)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|r| r.status != "dismissed");
            let digest = crate::research_recap::day_digest(store, id, start, end).await?;
            if recap.is_none() && digest.is_none() {
                continue;
            }
            let texts = |items: &[wisp_dto::ResearchRecapItem]| {
                items.iter().map(|i| i.text.clone()).collect::<Vec<_>>()
            };
            rows.push(json!({
                "project": name,
                "project_id": id,
                "recap": recap.map(|r| json!({
                    "status": r.status, "headline": r.headline,
                    "done": texts(&r.done), "findings": texts(&r.findings),
                    "issues": texts(&r.issues), "next": texts(&r.next),
                })),
                "records": digest.map(|(input, _)| input),
            }));
        }
        days.push(json!({"date": day.to_string(), "projects": rows}));
        day = day + chrono::Days::new(1);
    }
    let text = json!({"days": days}).to_string();
    Ok(if text.chars().count() > MAX_ACTIVITY_CHARS {
        format!(
            "{}\n(truncated: ask for fewer days or one project)",
            clip(&text, MAX_ACTIVITY_CHARS)
        )
    } else {
        text
    })
}

#[async_trait]
impl Tool for ActivityTool {
    fn name(&self) -> &str {
        "research_activity"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "research_activity",
            "Read recorded research activity per local day across projects: runs (status, errors), outputs, notes/decisions/findings, the researcher's requests in conversations, and the day's recap when one exists. Projects with nothing recorded are omitted.",
            json!({
                "type": "object",
                "properties": {
                    "from": {"type": "string", "description": "First day, YYYY-MM-DD, 'today' or 'yesterday'. Default today."},
                    "until": {"type": "string", "description": "Last day (inclusive), at most 7 days after 'from'. Default 'from'."},
                    "project_id": {"type": "string", "description": "Only this project."}
                }
            }),
        )
    }

    fn read_only(&self) -> bool {
        true
    }

    fn preview(&self, args: &Value) -> String {
        let from = args.get("from").and_then(Value::as_str).unwrap_or("today");
        match args.get("until").and_then(Value::as_str) {
            Some(until) => format!("{from} – {until}"),
            None => from.to_string(),
        }
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let today = today();
        let from = match parse_day(args.get("from").and_then(Value::as_str), today) {
            Ok(day) => day,
            Err(error) => return ToolResult::fail(error),
        };
        let until = match args.get("until").and_then(Value::as_str) {
            Some(value) => match parse_day(Some(value), today) {
                Ok(day) => day,
                Err(error) => return ToolResult::fail(error),
            },
            None => from,
        };
        let project = args.get("project_id").and_then(Value::as_str);
        match activity(&self.store, from, until, project).await {
            Ok(text) => ToolResult::ok(text),
            Err(error) => ToolResult::fail(error),
        }
    }
}

struct PlanTool {
    store: Store,
}

async fn plan(store: &Store, args: &Value, today: NaiveDate) -> Result<String, String> {
    let err = |error: anyhow::Error| error.to_string();
    let now = chrono::Utc::now().timestamp();
    match args.get("action").and_then(Value::as_str).unwrap_or("list") {
        "add" => {
            let day = parse_day(args.get("day").and_then(Value::as_str), today)?;
            let project_id = args
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty());
            if let Some(id) = project_id {
                if !visible_projects(store).await?.iter().any(|p| p.0 == id) {
                    return Err(format!("No visible project has id '{id}'."));
                }
            }
            let titles: Vec<String> = args
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if titles.is_empty() {
                return Err("'items' needs at least one non-empty item".into());
            }
            let mut ids = Vec::new();
            for title in titles {
                let task = AssistantTask {
                    id: uuid::Uuid::new_v4().to_string(),
                    day: day.to_string(),
                    title,
                    project_id: project_id.map(str::to_owned),
                    session_id: None,
                    status: "open".into(),
                    created_at: now,
                    updated_at: now,
                };
                store.add_assistant_task(&task).await.map_err(err)?;
                ids.push(task.id);
            }
            Ok(format!(
                "Saved {} item(s) for {day}: {}",
                ids.len(),
                ids.join(", ")
            ))
        }
        "update" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
            let status = args
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !wisp_store::ASSISTANT_TASK_STATUSES.contains(&status) {
                return Err("'status' must be open, done or dropped".into());
            }
            if store
                .set_assistant_task_status(id, status, now)
                .await
                .map_err(err)?
            {
                Ok(format!("Marked {id} {status}."))
            } else {
                Err(format!("No plan item has id '{id}'."))
            }
        }
        "list" => {
            let from = parse_day(args.get("from").and_then(Value::as_str), today)?;
            let until = match args.get("until").and_then(Value::as_str) {
                Some(value) => parse_day(Some(value), today)?,
                None => from,
            };
            let names: HashMap<String, String> = visible_projects(store)
                .await?
                .into_iter()
                .map(|p| (p.0, p.1))
                .collect();
            let tasks = store
                .assistant_tasks(&from.to_string(), &until.to_string())
                .await
                .map_err(err)?;
            if tasks.is_empty() {
                return Ok(format!("Nothing planned for {from}–{until}."));
            }
            Ok(tasks
                .iter()
                .map(|t| {
                    let mut line = format!("- [{}] {} {} (id: {})", t.status, t.day, t.title, t.id);
                    if let Some(project) = &t.project_id {
                        let name = names.get(project).map_or("hidden project", String::as_str);
                        line.push_str(&format!(" · project: {name}"));
                    }
                    if let Some(session) = &t.session_id {
                        line.push_str(&format!(" · dispatched to session {session}"));
                    }
                    line
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        other => Err(format!(
            "Unknown action '{other}'; use add, list or update."
        )),
    }
}

#[async_trait]
impl Tool for PlanTool {
    fn name(&self) -> &str {
        "research_plan"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "research_plan",
            "The researcher's dated plan, kept outside this conversation so it is never forgotten. 'add' saves items for a day; 'list' shows a day range plus earlier items still open; 'update' marks an item open, done or dropped.",
            json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["add", "list", "update"]},
                    "items": {"type": "array", "items": {"type": "string"}, "description": "add: one short item per entry."},
                    "day": {"type": "string", "description": "add: YYYY-MM-DD, 'today' or 'tomorrow'. Default today."},
                    "project_id": {"type": "string", "description": "add: the project the items belong to, if any."},
                    "from": {"type": "string", "description": "list: first day. Default today."},
                    "until": {"type": "string", "description": "list: last day (inclusive). Default 'from'."},
                    "id": {"type": "string", "description": "update: the item id."},
                    "status": {"type": "string", "enum": ["open", "done", "dropped"], "description": "update: new status."}
                },
                "required": ["action"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        args.get("action")
            .and_then(Value::as_str)
            .unwrap_or("list")
            .to_string()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        match plan(&self.store, args, today()).await {
            Ok(text) => ToolResult::ok(text),
            Err(error) => ToolResult::fail(error),
        }
    }
}

/// Create the project conversation and record it in the plan. The turn
/// itself is started by the caller. Returns `(session_id, plan_item_id)`.
async fn prepare_dispatch(
    store: &Store,
    project_id: &str,
    instruction: &str,
    title: Option<&str>,
    plan_item_id: Option<&str>,
    today: NaiveDate,
) -> Result<(String, String), String> {
    let err = |error: anyhow::Error| error.to_string();
    if instruction.trim().is_empty() {
        return Err("'instruction' cannot be empty".into());
    }
    if !visible_projects(store)
        .await?
        .iter()
        .any(|p| p.0 == project_id)
    {
        return Err(format!(
            "No visible project has id '{project_id}'. Call research_projects first."
        ));
    }
    let title = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| clip(instruction.lines().next().unwrap_or_default(), 80));
    let session_id = create_session_frame(store, project_id).await?;
    // A title lists the conversation in its project before the first turn lands.
    store
        .rename_session(&session_id, project_id, &title)
        .await
        .map_err(err)?;
    let now = chrono::Utc::now().timestamp();
    if let Some(id) = plan_item_id.filter(|id| !id.trim().is_empty()) {
        if store
            .link_assistant_task(id, project_id, &session_id, now)
            .await
            .map_err(err)?
        {
            return Ok((session_id, id.to_string()));
        }
    }
    let task = AssistantTask {
        id: uuid::Uuid::new_v4().to_string(),
        day: today.to_string(),
        title,
        project_id: Some(project_id.into()),
        session_id: Some(session_id.clone()),
        status: "open".into(),
        created_at: now,
        updated_at: now,
    };
    store.add_assistant_task(&task).await.map_err(err)?;
    Ok((session_id, task.id))
}

struct DispatchTool {
    app: AppHandle,
}

#[async_trait]
impl Tool for DispatchTool {
    fn name(&self) -> &str {
        "dispatch_to_project"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "dispatch_to_project",
            "Start a new conversation in a project and send it an instruction; that project's agent does the work with its own tools, in the background. Returns the new session id. The work is added to today's plan (or linked to plan_item_id).",
            json!({
                "type": "object",
                "properties": {
                    "project_id": {"type": "string", "description": "From research_projects."},
                    "instruction": {"type": "string", "description": "Complete, self-contained task for the project agent: goal, inputs, expected output."},
                    "title": {"type": "string", "description": "Short conversation title. Default: the instruction's first line."},
                    "plan_item_id": {"type": "string", "description": "The plan item this work comes from."}
                },
                "required": ["project_id", "instruction"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        args.get("title")
            .or_else(|| args.get("instruction"))
            .and_then(Value::as_str)
            .map(|text| clip(text, 80))
            .unwrap_or_default()
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let state = self.app.state::<AppState>();
        let project_id = args
            .get("project_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let instruction = args
            .get("instruction")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (session_id, item_id) = match prepare_dispatch(
            &state.store,
            project_id,
            instruction,
            args.get("title").and_then(Value::as_str),
            args.get("plan_item_id").and_then(Value::as_str),
            today(),
        )
        .await
        {
            Ok(ids) => ids,
            Err(error) => return ToolResult::fail(error),
        };
        // Provenance for the user and the project agent: this turn was not typed.
        let message = format!("[From the research assistant]\n\n{}", instruction.trim());
        let app = self.app.clone();
        let session = session_id.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            if let Err(error) = send_message_inner(
                state.inner(),
                app.clone(),
                "main",
                Some(session.clone()),
                message,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                crate::TurnOrigin::Desktop,
            )
            .await
            {
                tracing::warn!(target: "wisp", %error, session_id = %session, "assistant dispatch failed");
            }
        });
        ToolResult::ok(format!(
            "Started session {session_id} in the project (plan item {item_id}). It runs in the background; check it with project_session_result."
        ))
    }
}

/// Status and answer of a project conversation, for the assistant only.
async fn session_result(store: &Store, session_id: &str, running: bool) -> Result<String, String> {
    let reference = store
        .get_session_reference(session_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("No project conversation has id '{session_id}'."))?;
    let privacy = crate::privacy_mode::load(store).await?;
    if privacy.active && privacy.project_ids.contains(&reference.project_id) {
        return Err(format!("No project conversation has id '{session_id}'."));
    }
    let status = if running {
        "running (it may be waiting for the researcher's approval in that project)"
    } else {
        "idle"
    };
    let answer = crate::channels::last_assistant_text(store, session_id)
        .await
        .map(|text| clip(&text, MAX_RESULT_CHARS))
        .unwrap_or_else(|| "(no answer yet)".into());
    Ok(format!(
        "Project: {}\nConversation: {}\nStatus: {status}\nLatest answer:\n{answer}",
        reference.project_name, reference.title
    ))
}

struct SessionResultTool {
    app: AppHandle,
}

#[async_trait]
impl Tool for SessionResultTool {
    fn name(&self) -> &str {
        "project_session_result"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "project_session_result",
            "Check a project conversation (e.g. one started with dispatch_to_project): whether it is still running and its latest answer.",
            json!({
                "type": "object",
                "properties": {"session_id": {"type": "string"}},
                "required": ["session_id"]
            }),
        )
    }

    fn read_only(&self) -> bool {
        true
    }

    async fn run(&self, args: &Value, _env: &dyn ToolEnv) -> ToolResult {
        let state = self.app.state::<AppState>();
        let session_id = args
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let running = state.running_turns.lock().await.contains(session_id);
        match session_result(&state.store, session_id, running).await {
            Ok(text) => ToolResult::ok(text),
            Err(error) => ToolResult::fail(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: std::path::PathBuf,
        store: Store,
    }

    impl Fixture {
        async fn open() -> Self {
            let dir = std::env::temp_dir().join(format!("wisp-assistant-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let store = Store::open(&dir.join("wisp.sqlite")).await.unwrap();
            Self { dir, store }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[tokio::test]
    async fn sidebar_plan_preserves_dates_statuses_and_project_context() {
        let f = Fixture::open().await;
        f.store.create_project("p1", "RNA-seq", "").await.unwrap();
        for (id, date, status, project_id) in [
            ("carried", "2026-10-01", "open", Some("p1")),
            ("old-done", "2026-10-01", "done", Some("p1")),
            ("completed", "2026-10-02", "done", Some("p1")),
            ("dropped", "2026-10-02", "dropped", None),
            ("future", "2026-10-03", "open", None),
        ] {
            f.store
                .add_assistant_task(&AssistantTask {
                    id: id.into(),
                    day: date.into(),
                    title: format!("Plan {id}"),
                    project_id: project_id.map(str::to_string),
                    session_id: project_id.map(|_| "session-p1".into()),
                    status: status.into(),
                    created_at: 1,
                    updated_at: 1,
                })
                .await
                .unwrap();
        }
        let rows = visible_plan(&f.store, "2026-10-02").await.unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["carried", "completed", "dropped"]
        );
        assert_eq!(rows[0].day, "2026-10-01");
        assert_eq!(rows[0].project_name.as_deref(), Some("RNA-seq"));
        assert_eq!(rows[0].session_id.as_deref(), Some("session-p1"));
        assert_eq!(rows[1].status, "done");
        assert_eq!(rows[2].status, "dropped");
        // The actual command payload round-trips through the shared UI contract.
        let wire = serde_json::to_value(&rows).unwrap();
        assert_eq!(
            serde_json::from_value::<Vec<wisp_dto::ResearchAssistantPlanItem>>(wire).unwrap(),
            rows
        );
        assert!(visible_plan(&f.store, "2026-02-30").await.is_err());
        assert!(visible_plan(&f.store, "tomorrow").await.is_err());
    }

    #[tokio::test]
    async fn sidebar_plan_excludes_hidden_and_missing_projects_including_task_titles() {
        let f = Fixture::open().await;
        f.store
            .create_project("hidden", "Private research", "")
            .await
            .unwrap();
        f.store
            .create_project("visible", "Visible research", "")
            .await
            .unwrap();
        crate::privacy_mode::save(&f.store, true, &["hidden".into()])
            .await
            .unwrap();
        for project_id in [Some("hidden"), Some("missing"), Some("visible"), None] {
            let id = project_id.unwrap_or("global");
            f.store
                .add_assistant_task(&AssistantTask {
                    id: id.into(),
                    day: "2026-10-02".into(),
                    title: format!("Title for {id}"),
                    project_id: project_id.map(str::to_string),
                    session_id: project_id.map(|p| format!("session-{p}")),
                    status: "open".into(),
                    created_at: 1,
                    updated_at: 1,
                })
                .await
                .unwrap();
        }
        let rows = visible_plan(&f.store, "2026-10-02").await.unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["global", "visible"]
        );
        let wire = serde_json::to_string(&rows).unwrap();
        assert!(!wire.contains("hidden"));
        assert!(!wire.contains("missing"));
        assert_eq!(visible_projects(&f.store).await.unwrap().len(), 1);
        crate::privacy_mode::save(&f.store, false, &[])
            .await
            .unwrap();
        assert_eq!(visible_plan(&f.store, "2026-10-02").await.unwrap().len(), 3);
    }

    #[test]
    fn relative_and_explicit_days_parse() {
        let today = day("2026-10-02");
        assert_eq!(parse_day(None, today).unwrap(), today);
        assert_eq!(
            parse_day(Some("yesterday"), today).unwrap(),
            day("2026-10-01")
        );
        assert_eq!(
            parse_day(Some("2026-09-30"), today).unwrap(),
            day("2026-09-30")
        );
        assert!(parse_day(Some("last week"), today).is_err());
    }

    #[tokio::test]
    async fn the_assistant_keeps_one_hidden_conversation() {
        let f = Fixture::open().await;
        ensure(&f.store, &f.dir).await.unwrap();
        ensure(&f.store, &f.dir).await.unwrap();
        assert_eq!(
            f.store
                .frame_project_id(ASSISTANT_FRAME_ID)
                .await
                .unwrap()
                .as_deref(),
            Some(ASSISTANT_PROJECT_ID)
        );
        assert!(f.store.list_projects().await.unwrap().is_empty());
        assert!(create_session_frame(&f.store, ASSISTANT_PROJECT_ID)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn dispatch_opens_a_titled_project_conversation_and_plans_it() {
        let f = Fixture::open().await;
        f.store.create_project("p1", "RNA-seq", "").await.unwrap();
        let today = day("2026-10-02");
        let added = plan(
            &f.store,
            &json!({"action": "add", "items": ["Rerun DE analysis"], "project_id": "p1"}),
            today,
        )
        .await
        .unwrap();
        let item = added.rsplit(": ").next().unwrap().to_string();

        let (session, linked) = prepare_dispatch(
            &f.store,
            "p1",
            "Rerun the DE analysis with batch as a covariate.\nReport the top genes.",
            None,
            Some(&item),
            today,
        )
        .await
        .unwrap();
        assert_eq!(linked, item);
        let reference = f
            .store
            .get_session_reference(&session)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reference.project_id, "p1");
        assert_eq!(
            reference.title,
            "Rerun the DE analysis with batch as a covariate."
        );
        let listed = plan(&f.store, &json!({"action": "list"}), today)
            .await
            .unwrap();
        assert!(listed.contains("project: RNA-seq"), "{listed}");
        assert!(listed.contains(&session), "{listed}");

        // Neither a missing nor a privacy-hidden project can receive work.
        assert!(prepare_dispatch(&f.store, "nope", "x", None, None, today)
            .await
            .is_err());
        crate::privacy_mode::save(&f.store, true, &["p1".into()])
            .await
            .unwrap();
        assert!(prepare_dispatch(&f.store, "p1", "x", None, None, today)
            .await
            .is_err());
        assert!(session_result(&f.store, &session, false).await.is_err());
    }

    #[tokio::test]
    async fn session_result_reports_status_and_the_final_answer() {
        let f = Fixture::open().await;
        f.store.create_project("p1", "RNA-seq", "").await.unwrap();
        let (session, _) = prepare_dispatch(
            &f.store,
            "p1",
            "Summarize QC",
            None,
            None,
            day("2026-10-02"),
        )
        .await
        .unwrap();
        let text = session_result(&f.store, &session, true).await.unwrap();
        assert!(text.contains("Status: running"), "{text}");
        assert!(text.contains("(no answer yet)"), "{text}");
        ensure(&f.store, &f.dir).await.unwrap();
        assert!(session_result(&f.store, ASSISTANT_FRAME_ID, false)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn activity_is_bounded_and_skips_quiet_projects() {
        let f = Fixture::open().await;
        f.store.create_project("p1", "RNA-seq", "").await.unwrap();
        let today = day("2026-10-02");
        let text = activity(&f.store, today, today, None).await.unwrap();
        assert_eq!(text, r#"{"days":[{"date":"2026-10-02","projects":[]}]}"#);
        assert!(activity(&f.store, today, day("2026-10-09"), None)
            .await
            .is_err());
        assert!(activity(&f.store, today, today, Some("nope"))
            .await
            .is_err());
    }
}
