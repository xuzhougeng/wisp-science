//! Bringing the WebView desktop's projects to this host. The WebView's
//! database is never written: its projects are listed through a read-only
//! connection, and to bring one over that connection first copies the whole
//! database, as one consistent file, into this host's data folder, from
//! which everything is then read. A project the WebView keeps in its own
//! database is copied from it and goes on using its folder where it is; a
//! project that keeps its database in its folder is opened where it is, as
//! a folder package is, so both desktops then share it. The WebView's
//! settings that mean the same here come too, by group, never over one
//! made here, and so do those of each project copied and its
//! conversations. Its saved models come with their keys, which go from
//! the keychain to the keychain under this desktop's names.
//!
//! Its lists come by group as well, each only while this desktop has none
//! of its own and each item only as it is: its own MCP servers and its
//! decision model, their secrets going from the keychain to the keychain;
//! its workflows and the quick actions that start them; and its
//! favorites. What this desktop cannot hold as the WebView has it stays
//! behind, and the reply says why.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::desktop::SecretStore;
use crate::mcp::{McpServer, McpTransport};
use crate::quick_actions::QuickAction;
use crate::workflows::{AgentSettings, Workflow, WorkflowApproval, WorkflowTask};
use crate::Server;
use wisp_dto::{DecisionModelSettings, DecisionPreset, DECISION_PRESETS};
use wisp_store::LibraryRecord;

/// The folder in this host's data folder the copy is made in; emptied
/// before and after every import.
const SCRATCH: &str = "webview-import";

/// The WebView's saved models, kept as one JSON list.
const MODELS: &str = "model_profiles";
/// The id of the WebView's everyday model.
const ACTIVE_MODEL: &str = "active_model_id";

/// The WebView's settings that mean the same here, in the groups they
/// are offered by: a group's name and its keys. The decision model, whose
/// keys are kept under other names, comes as one of [`LIST_GROUPS`].
pub const SETTING_GROUPS: &[(&str, &[&str])] = &[
    ("network", &["network_settings", "proxy_url"]),
    ("skills", &["disabled_skills", "skill_tags"]),
    ("connectors", &["disabled_connectors"]),
    ("experts", &["specialists"]),
    ("acp_agents", &["acp_agent_profiles"]),
    ("hooks", &["command_hooks"]),
    ("credentials", &["custom_credentials"]),
    ("browser", &["browser_url_filters"]),
    ("review", &["auto_review_default_enabled"]),
    ("failure_analysis", &["auto_failure_analysis"]),
    ("compaction", &["auto_compact", "auto_compact_percent"]),
    ("continuing", &["auto_continue", "auto_continue_limit"]),
    ("memory", &["memory_enabled"]),
    ("follow_ups", &["follow_up_questions"]),
    ("update_check", &["update_check_enabled"]),
    ("pet", &["pet_enabled", "pet_directory"]),
    ("subagent_model", &["subagent_model_id"]),
];

/// The WebView's lists that come by what they hold, in the order they are
/// offered: its own MCP servers, the decision model, its workflows, its
/// quick actions and its favorites. Each comes only while this desktop
/// has none of its own.
pub const LIST_GROUPS: &[&str] = &[
    "mcp_servers",
    "decision_model",
    "workflows",
    "quick_actions",
    "favorites",
];

/// Where the WebView keeps its MCP servers, its decision model, its
/// workflows, its quick actions and its experts.
const WEBVIEW_MCP: &str = "mcp_connections";
const WEBVIEW_DECISION: &str = "decision_model";
const WEBVIEW_WORKFLOWS: &str = "workflow_templates";
const WEBVIEW_ACTIONS: &str = "quick_actions";
const WEBVIEW_EXPERTS: &str = "specialists";
/// The WebView's own workflows and quick action, made by its code and
/// not the user's; this desktop has its own.
const BUILT_IN_WORKFLOWS: &[&str] = &[
    "literature_evidence_review",
    "roundtable",
    "data_driven_research_design",
    "develop_computational_method",
];
const BUILT_IN_ACTION: &str = "literature_research";
/// The WebView's favorites, beside its database.
const LIBRARY: &str = "library.sqlite";

/// Why one of the WebView's servers, workflows, quick actions or
/// favorites does not come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebViewHold {
    /// Its entry in the WebView's data could not be read.
    Unreadable,
    /// A program the WebView starts in a folder of its own; servers here
    /// start where the app does.
    OwnFolder,
    /// A server the WebView signs in to with OAuth, which is not done here.
    OAuth,
    /// One of its tasks runs an activity on a host instead of an agent.
    Activity,
    /// One of its tasks is bound to skills.
    Skills,
    /// One of its tasks runs on a model of its own.
    NamedModel,
    /// One of its tasks is done by an ACP or other outside agent.
    Executor,
    /// One of its tasks is limited in tool calls or cost.
    Budget,
    /// One of its tasks may do what agents here are not given: this.
    Capability(String),
    /// One of its tasks is done as an expert that is not here: its id.
    Expert(String),
    /// A quick action switched off in the WebView.
    Off,
    /// A quick action that starts one of the WebView's own workflows.
    BuiltInWorkflow,
    /// A quick action whose workflow does not come.
    NoWorkflow,
    /// One here, or one brought before it, has its name.
    NameTaken,
    /// The list here has no room for it.
    Full,
    /// This desktop refuses it as written, for this reason.
    Refused(String),
}

/// One of [`LIST_GROUPS`]: what of it comes, or came, and what stays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebViewGroup {
    pub group: String,
    /// The names of what comes, in order; for the decision model its
    /// service.
    pub items: Vec<String>,
    /// Of those, the ones that come without a secret the WebView kept
    /// for them, as the keychain did not give it: entered again here.
    pub without_secret: Vec<String>,
    /// What stays behind, by name, and why.
    pub held: Vec<(String, WebViewHold)>,
    /// Favorites saved from a conversation that is not here; their link
    /// to it is dropped, unless it comes along first.
    pub unlinked: usize,
}

/// A model the WebView saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewModel {
    pub id: String,
    pub name: String,
    /// The model's id at its provider.
    pub model: String,
    pub provider: String,
    /// Its key is in the keychain, or it is a subscription signed in to.
    pub has_key: bool,
    /// The WebView's everyday model.
    pub active: bool,
    /// A saved model with its id is already here.
    pub here: bool,
}

/// One saved model as the WebView keeps it; what is not needed here is
/// left out, and anything missing is empty.
#[derive(Default, Deserialize)]
#[serde(default)]
struct WebViewProfile {
    id: String,
    label: String,
    provider: String,
    api_url: String,
    endpoint_suffix: String,
    model: String,
    max_tokens: u64,
    context_window: u64,
    reasoning_effort: String,
    proxy_url: Option<String>,
}

impl WebViewProfile {
    /// The fields this desktop saves a model with: the endpoint with its
    /// path, and the limits only when the WebView gave them.
    fn fields(&self) -> crate::desktop::ModelFields {
        let base = self.api_url.trim().trim_end_matches('/');
        let suffix = self.endpoint_suffix.trim().trim_matches('/');
        let limit = |value: u64| {
            if value == 0 {
                String::new()
            } else {
                value.to_string()
            }
        };
        crate::desktop::ModelFields {
            provider: self.provider.trim().to_owned(),
            api_url: if suffix.is_empty() {
                base.to_owned()
            } else {
                format!("{base}/{suffix}")
            },
            model: self.model.trim().to_owned(),
            max_context: limit(self.context_window),
            max_tokens: limit(self.max_tokens),
            reasoning_effort: self.reasoning_effort.trim().to_owned(),
            proxy: self
                .proxy_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        }
    }
}

/// Where the WebView keeps a saved model's key in the keychain.
fn webview_key(id: &str) -> String {
    format!("model_key:{id}")
}

/// The WebView's saved models, each entry read on its own so one it
/// cannot read does not hide the rest, and its everyday model's id.
async fn webview_profiles(pool: &SqlitePool) -> Result<(Vec<WebViewProfile>, String)> {
    if !has_table(pool, "settings").await? {
        return Ok((Vec::new(), String::new()));
    }
    let read = |key: &'static str| async move {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key=?")
            .bind(key)
            .fetch_optional(pool)
            .await
    };
    let profiles = read(MODELS)
        .await?
        .and_then(|raw| serde_json::from_str::<Vec<serde_json::Value>>(&raw).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<WebViewProfile>(entry).ok())
        .filter(|profile| !profile.id.trim().is_empty() && !profile.model.trim().is_empty())
        .collect();
    Ok((profiles, read(ACTIVE_MODEL).await?.unwrap_or_default()))
}

/// What the WebView has that can come.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebViewData {
    pub projects: Vec<WebViewProject>,
    pub models: Vec<WebViewModel>,
    /// The groups of [`SETTING_GROUPS`] the WebView has made and this
    /// desktop has not, in that order.
    pub settings: Vec<String>,
    /// The groups of [`LIST_GROUPS`] the WebView has and this desktop
    /// has none of, in that order, each with what of it would come.
    pub groups: Vec<WebViewGroup>,
}

/// How an import went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebViewOutcome {
    pub projects: Vec<WebViewImport>,
    /// The ids of the saved models brought over.
    pub models: Vec<String>,
    /// The groups of settings brought over.
    pub settings: Vec<String>,
    /// The groups of [`LIST_GROUPS`] chosen, each with what came and
    /// what stayed.
    pub groups: Vec<WebViewGroup>,
}

/// A project of the WebView desktop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewProject {
    pub id: String,
    pub name: String,
    /// Its folder, where the WebView has it.
    pub folder: String,
    /// Its conversations; `None` when it keeps its own database, which is
    /// not opened to count them.
    pub conversations: Option<usize>,
    /// It keeps its database in its folder.
    pub own_database: bool,
    pub starred: bool,
    /// A project with its id is already here.
    pub here: bool,
    /// Its folder is on this computer.
    pub folder_found: bool,
}

/// How bringing one project over went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebViewImport {
    pub id: String,
    pub name: String,
    /// Why it was not brought over; `None` when it was.
    pub error: Option<String>,
}

/// A connection that only reads the WebView's database.
async fn read_only(database: &Path) -> Result<SqlitePool> {
    ensure!(
        database.is_file(),
        "No WebView database at {}",
        database.display()
    );
    let options = SqliteConnectOptions::new()
        .filename(database)
        .read_only(true)
        .busy_timeout(std::time::Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .context("Cannot read the WebView's database")
}

async fn has_table(pool: &SqlitePool, table: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
    )
    .bind(table)
    .fetch_one(pool)
    .await?)
}

/// The projects in a WebView database, its hidden ones left out, starred
/// first and then by name. Nothing is written.
async fn projects(pool: &SqlitePool) -> Result<Vec<WebViewProject>> {
    ensure!(
        has_table(pool, "projects").await?,
        "This is not a Wisp database"
    );
    let own: HashSet<String> = if has_table(pool, "project_locations").await? {
        sqlx::query_scalar("SELECT project_id FROM project_locations")
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let starred: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('projects') WHERE name='starred')",
    )
    .fetch_one(pool)
    .await?;
    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(&format!(
        "SELECT id, COALESCE(name,''), COALESCE(workspace_dir,''), {} FROM projects \
         WHERE id NOT LIKE 'assistant:%' AND id NOT LIKE 'scratch:%'",
        if starred { "starred" } else { "0" }
    ))
    .fetch_all(pool)
    .await?;
    let mut found = Vec::with_capacity(rows.len());
    for (id, name, folder, starred) in rows {
        let own_database = own.contains(&id);
        // Conversations as the list counts them: one begun, or named.
        let conversations = if own_database {
            None
        } else {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM frames f WHERE f.project_id=? AND f.parent_frame_id=f.id \
                 AND f.exploration_id IS NULL AND (f.title IS NOT NULL OR EXISTS( \
                 SELECT 1 FROM messages m WHERE m.frame_id=f.id AND m.role='user'))",
            )
            .bind(&id)
            .fetch_one(pool)
            .await
            .ok()
            .map(|count| count.max(0) as usize)
        };
        found.push(WebViewProject {
            folder_found: !folder.is_empty() && Path::new(&folder).is_dir(),
            id,
            name,
            folder,
            conversations,
            own_database,
            starred,
            here: false,
        });
    }
    found.sort_by(|a, b| {
        b.starred
            .cmp(&a.starred)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(found)
}

/// The WebView's values for the keys of [`SETTING_GROUPS`] and of its
/// lists.
async fn setting_values(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    if !has_table(pool, "settings").await? {
        return Ok(values);
    }
    let lists = [
        WEBVIEW_MCP,
        WEBVIEW_DECISION,
        WEBVIEW_WORKFLOWS,
        WEBVIEW_ACTIONS,
    ];
    for key in SETTING_GROUPS
        .iter()
        .flat_map(|(_, keys)| keys.iter())
        .chain(lists.iter())
    {
        let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key=?")
            .bind(key)
            .fetch_optional(pool)
            .await?;
        if let Some(value) = value {
            values.insert((*key).to_owned(), value);
        }
    }
    Ok(values)
}

/// The groups that can come: the WebView has made one of their settings
/// and this desktop none.
async fn offered(
    store: &wisp_store::Store,
    values: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let mut groups = Vec::new();
    for (group, keys) in SETTING_GROUPS {
        if !keys.iter().any(|key| values.contains_key(*key)) {
            continue;
        }
        let mut here = false;
        for key in *keys {
            here |= store.get_setting(key).await?.is_some();
        }
        if !here {
            groups.push((*group).to_owned());
        }
    }
    Ok(groups)
}

/// The settings that belong to one project or to one of its
/// conversations: their keys end in its id after a colon, or name the
/// conversation's ACP options.
async fn project_settings(pool: &SqlitePool, project: &str) -> Result<Vec<(String, String)>> {
    if !has_table(pool, "settings").await? {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT key, value FROM settings WHERE substr(key, -length(?1) - 1) = ':' || ?1 \
         UNION \
         SELECT s.key, s.value FROM settings s JOIN frames f ON f.project_id = ?1 \
         WHERE substr(s.key, -length(f.id) - 1) = ':' || f.id \
            OR substr(s.key, 1, length(f.id) + 20) = 'acp.config_options.' || f.id || '.'",
    )
    .bind(project)
    .fetch_all(pool)
    .await?)
}

/// Brings one project over from the copy of the WebView's database.
async fn bring(
    store: &wisp_store::Store,
    copy: &wisp_store::Store,
    rows: &SqlitePool,
    data: &Path,
    scratch: &Path,
    project: &WebViewProject,
) -> Result<()> {
    ensure!(
        store.get_project(&project.id).await?.is_none(),
        "It is already here"
    );
    let folder = PathBuf::from(&project.folder);
    ensure!(
        !project.folder.is_empty() && folder.is_dir(),
        "Its folder is not on this computer: {}",
        project.folder
    );
    let folder = dunce::canonicalize(&folder)?;
    ensure!(
        !folder.starts_with(data),
        "Keep project files separate from the host data directory"
    );
    if project.own_database {
        crate::project_archive::import_project_folder(store, data, &folder, None)
            .await
            .map_err(anyhow::Error::msg)?;
    } else {
        let records = scratch.join(format!("project-{}.sqlite", uuid::Uuid::new_v4()));
        copy.export_project_database(&project.id, &records).await?;
        store
            .import_project_database(&records, &project.id, &folder)
            .await?;
        let _ = std::fs::remove_file(&records);
        // A copy leaves its settings behind; they come here unless set.
        for (key, value) in project_settings(rows, &project.id).await? {
            if store.get_setting(&key).await?.is_none() {
                store.set_setting(&key, &value).await?;
            }
        }
    }
    if project.starred {
        let _ = store.set_project_starred(&project.id, true).await;
    }
    Ok(())
}

/// What of one of [`LIST_GROUPS`] would come, each item with what it is
/// here, and what stays.
struct Planned<T> {
    group: &'static str,
    items: Vec<(String, T)>,
    without_secret: Vec<String>,
    held: Vec<(String, WebViewHold)>,
    unlinked: usize,
}

impl<T> Planned<T> {
    fn new(group: &'static str) -> Self {
        Self {
            group,
            items: Vec::new(),
            without_secret: Vec::new(),
            held: Vec::new(),
            unlinked: 0,
        }
    }

    fn hold(&mut self, name: impl Into<String>, why: WebViewHold) {
        self.held.push((name.into(), why));
    }

    /// The WebView has nothing of it.
    fn is_empty(&self) -> bool {
        self.items.is_empty() && self.held.is_empty()
    }

    fn shown(&self) -> WebViewGroup {
        WebViewGroup {
            group: self.group.to_owned(),
            items: self.items.iter().map(|(name, _)| name.clone()).collect(),
            without_secret: self.without_secret.clone(),
            held: self.held.clone(),
            unlinked: self.unlinked,
        }
    }
}

/// The entries of a list the WebView keeps as JSON, each to be read on its
/// own so one that cannot be read does not hide the rest.
fn entries(raw: Option<&String>) -> Vec<Value> {
    raw.and_then(|raw| serde_json::from_str::<Vec<Value>>(raw).ok())
        .unwrap_or_default()
}

/// The name an entry gives itself, to say which one stays.
fn entry_name(entry: &Value) -> String {
    entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// One of the WebView's MCP servers as it keeps it: names of variables
/// and headers only, their values in the keychain.
#[derive(Deserialize)]
struct WebViewServer {
    id: String,
    name: String,
    enabled: bool,
    transport: WebViewTransport,
    #[serde(default)]
    proxy_url: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum WebViewTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<WebViewSlot>,
        #[serde(default)]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: Vec<WebViewSlot>,
        #[serde(default)]
        auth: Option<String>,
    },
}

/// A variable or header the WebView names. `value` is only there in a
/// database the WebView has not opened since it moved values into the
/// keychain.
#[derive(Default, Deserialize)]
#[serde(default)]
struct WebViewSlot {
    name: String,
    value: Option<String>,
}

/// One of the WebView's servers as it is kept here, with the values of
/// its variables or headers the keychain gave, and the names it did not
/// give a value for.
struct PlannedServer {
    server: McpServer,
    values: BTreeMap<String, String>,
    missing: Vec<String>,
}

/// One of the WebView's servers as this desktop keeps it, or why it
/// cannot be. A value is read from the WebView's keychain entry, or from
/// a database older than those; one there is not or that a form would
/// refuse is missing.
fn plan_server(
    entry: WebViewServer,
    secrets: &dyn SecretStore,
) -> std::result::Result<PlannedServer, WebViewHold> {
    use wisp_store::mcp_secrets::{env_secret_name, header_secret_name};
    let id = entry.id.trim().to_owned();
    if id.is_empty() {
        return Err(WebViewHold::Unreadable);
    }
    let (transport, slots, program) = match entry.transport {
        WebViewTransport::Stdio { cwd: Some(cwd), .. } if !cwd.trim().is_empty() => {
            return Err(WebViewHold::OwnFolder)
        }
        WebViewTransport::Stdio {
            command, args, env, ..
        } => (McpTransport::Stdio { command, args }, env, true),
        WebViewTransport::Http { url, headers, auth } => {
            match auth.as_deref().map(str::trim).unwrap_or_default() {
                "" | "none" => {}
                "oauth" => return Err(WebViewHold::OAuth),
                _ => return Err(WebViewHold::Unreadable),
            }
            (McpTransport::Http { url }, headers, false)
        }
    };
    let what = if program { "variable" } else { "header" };
    let mut names = BTreeSet::new();
    for slot in &slots {
        let name = slot.name.trim();
        if name.is_empty() {
            continue;
        }
        let valid = if program {
            crate::mcp::valid_env_name(name)
        } else {
            crate::mcp::valid_header_name(name)
        };
        if !valid {
            return Err(WebViewHold::Refused(format!(
                "\"{name}\" cannot be the name of a {what}"
            )));
        }
        names.insert(name.to_owned());
    }
    if names.len() > 32 {
        return Err(WebViewHold::Refused(format!("Keep to 32 {what}s")));
    }
    let mut values = BTreeMap::new();
    let mut missing = Vec::new();
    for name in &names {
        let kept = if program {
            env_secret_name(&id, name)
        } else {
            header_secret_name(&id, name)
        };
        let usable = |value: String| {
            let value = value.trim().to_owned();
            (!value.is_empty() && crate::mcp::valid_value(&value)).then_some(value)
        };
        let value = secrets.get(&kept).and_then(usable).or_else(|| {
            slots
                .iter()
                .find(|slot| slot.name.trim() == name)
                .and_then(|slot| slot.value.clone())
                .and_then(usable)
        });
        match value {
            Some(value) => {
                values.insert(name.clone(), value);
            }
            None => missing.push(name.clone()),
        }
    }
    let proxy = crate::network::own_route("Route", &entry.proxy_url)
        .map_err(|error| WebViewHold::Refused(error.to_string()))?;
    let names: Vec<String> = names.into_iter().collect();
    let (env, headers) = if program {
        (names, Vec::new())
    } else {
        (Vec::new(), names)
    };
    let server = McpServer {
        id,
        name: entry.name.trim().to_owned(),
        enabled: entry.enabled,
        transport,
        env,
        headers,
        proxy,
    };
    server
        .validate()
        .map_err(|error| WebViewHold::Refused(error.to_string()))?;
    Ok(PlannedServer {
        server,
        values,
        missing,
    })
}

/// The WebView's MCP servers that come, in its order: a server named
/// like one before it stays.
fn plan_servers(raw: Option<&String>, secrets: &dyn SecretStore) -> Planned<PlannedServer> {
    let mut plan: Planned<PlannedServer> = Planned::new("mcp_servers");
    for entry in entries(raw) {
        let name = entry_name(&entry);
        let planned = serde_json::from_value::<WebViewServer>(entry)
            .map_err(|_| WebViewHold::Unreadable)
            .and_then(|entry| plan_server(entry, secrets));
        match planned {
            Err(why) => plan.hold(name, why),
            Ok(planned) => {
                let slug = planned.server.slug();
                if plan
                    .items
                    .iter()
                    .any(|(_, other)| other.server.slug() == slug)
                {
                    plan.hold(name, WebViewHold::NameTaken);
                    continue;
                }
                if !planned.missing.is_empty() {
                    plan.without_secret.push(planned.server.name.clone());
                }
                plan.items.push((planned.server.name.clone(), planned));
            }
        }
    }
    plan
}

/// Where the WebView keeps a decision model service's key: TypeSafe's
/// under the name it had before the decision model was a setting.
fn webview_decision_key(preset: &str) -> String {
    if preset == "typesafe" {
        "typesafe_api_key".into()
    } else {
        format!("decision_api_key:{preset}")
    }
}

/// The WebView's decision model as it reads it, with the keys the
/// keychain gives for its services; `None` when it has none: never
/// saved, and no TypeSafe key that switches it on.
fn plan_decision(
    raw: Option<&String>,
    secrets: &dyn SecretStore,
) -> Option<Planned<(DecisionModelSettings, Vec<(String, String)>)>> {
    let keys: Vec<(String, String)> = DECISION_PRESETS
        .iter()
        .filter_map(|preset| {
            let key = secrets.get(&webview_decision_key(preset.id))?;
            let key = crate::decision_model::usable_key(&key)?.to_owned();
            Some((preset.id.to_owned(), key))
        })
        .collect();
    let saved = raw
        .and_then(|raw| serde_json::from_str::<DecisionModelSettings>(raw).ok())
        .filter(|saved| DecisionPreset::find(&saved.preset).is_some());
    let settings = match saved {
        Some(saved) => saved,
        // As the WebView reads it: TypeSafe, on when its key is there.
        None => {
            let mut first = DecisionModelSettings::from_preset(DecisionPreset::find("typesafe")?);
            first.enabled = keys.iter().any(|(preset, _)| preset == "typesafe");
            if raw.is_none() && !first.enabled {
                return None;
            }
            first
        }
    };
    let mut plan = Planned::new("decision_model");
    let name = settings.preset.clone();
    match crate::decision_model::checked(settings) {
        Err(error) => plan.hold(name, WebViewHold::Refused(error.to_string())),
        Ok(settings) => {
            let needed = DecisionPreset::find(&settings.preset).is_some_and(|preset| {
                settings.enabled
                    && preset.key_required
                    && !keys.iter().any(|(kept, _)| kept == preset.id)
            });
            if needed {
                plan.without_secret.push(name.clone());
            }
            plan.items.push((name, (settings, keys)));
        }
    }
    Some(plan)
}

/// One of the WebView's workflow templates as it keeps it. Every field
/// it can have is named, so one this reading does not know refuses it
/// rather than being lost.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebViewTemplate {
    id: String,
    name: String,
    /// Shown in the WebView's library only; workflows here have none.
    #[serde(default, rename = "description")]
    _description: serde::de::IgnoredAny,
    proposal: WebViewProposal,
    #[serde(default)]
    builtin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebViewProposal {
    goal: String,
    #[serde(default)]
    context: String,
    approval_policy: WorkflowApproval,
    tasks: Vec<WebViewTask>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebViewTask {
    id: String,
    instruction: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    task_kind: wisp_core::WorkflowTaskKind,
    #[serde(default)]
    run_activity: Option<Value>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    skill_ids: Vec<String>,
    #[serde(default)]
    specialist_id: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    isolated: bool,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    model_tier: Option<wisp_core::ModelTier>,
    #[serde(default)]
    executor: Option<WebViewExecutor>,
    #[serde(default)]
    budget: Option<WebViewBudget>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebViewExecutor {
    kind: String,
    #[serde(default)]
    profile_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebViewBudget {
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    max_tool_calls: Option<u32>,
    #[serde(default)]
    max_cost_microunits: Option<u64>,
}

/// A WebView template as a workflow of this desktop that does what it
/// does, or why there is none: every task an agent, with only what agents
/// here are given, as an expert that is in `experts`. Tasks are put in an
/// order in which each needs only tasks before it, the template's order
/// kept where it can be, and titled from their ids.
fn workflow_of(
    template: WebViewTemplate,
    experts: &HashSet<String>,
) -> std::result::Result<Workflow, WebViewHold> {
    let proposal = template.proposal;
    let tasks = &proposal.tasks;
    for task in tasks {
        if task.task_kind != wisp_core::WorkflowTaskKind::Agent || task.run_activity.is_some() {
            return Err(WebViewHold::Activity);
        }
        if !task.skill_ids.is_empty() {
            return Err(WebViewHold::Skills);
        }
        if task
            .model_id
            .as_deref()
            .is_some_and(|model| !model.trim().is_empty())
        {
            return Err(WebViewHold::NamedModel);
        }
        match &task.executor {
            None => {}
            Some(executor)
                if executor.kind == "native"
                    && executor
                        .profile_id
                        .as_deref()
                        .is_none_or(|profile| profile.trim().is_empty()) => {}
            Some(executor) if matches!(executor.kind.as_str(), "acp" | "external") => {
                return Err(WebViewHold::Executor)
            }
            Some(_) => return Err(WebViewHold::Unreadable),
        }
        if task.budget.as_ref().is_some_and(|budget| {
            budget.max_tool_calls.is_some() || budget.max_cost_microunits.is_some()
        }) {
            return Err(WebViewHold::Budget);
        }
        if let Some(capability) = task
            .capabilities
            .iter()
            .map(|capability| capability.trim())
            .find(|capability| !crate::workflows::CAPABILITIES.contains(capability))
        {
            return Err(WebViewHold::Capability(capability.to_owned()));
        }
        if let Some(expert) = task
            .specialist_id
            .as_deref()
            .map(str::trim)
            .filter(|expert| !expert.is_empty() && !experts.contains(*expert))
        {
            return Err(WebViewHold::Expert(expert.to_owned()));
        }
    }
    let unique: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
    if unique.len() != tasks.len() {
        return Err(WebViewHold::Unreadable);
    }
    let mut order: Vec<usize> = Vec::with_capacity(tasks.len());
    while order.len() < tasks.len() {
        let before = order.len();
        for (index, task) in tasks.iter().enumerate() {
            let ready = !order.contains(&index)
                && task
                    .depends_on
                    .iter()
                    .all(|needed| order.iter().any(|placed| &tasks[*placed].id == needed));
            if ready {
                order.push(index);
            }
        }
        // A task that needs one that is not there, or a circle.
        if order.len() == before {
            return Err(WebViewHold::Unreadable);
        }
    }
    let place = |id: &str| order.iter().position(|index| tasks[*index].id == id);
    let converted = order
        .iter()
        .map(|&index| {
            let task = &tasks[index];
            WorkflowTask {
                title: crate::workflows::task_title(&task.id),
                instructions: task.instruction.clone(),
                after: task.depends_on.iter().filter_map(|id| place(id)).collect(),
                capabilities: task
                    .capabilities
                    .iter()
                    .map(|capability| capability.trim().to_owned())
                    .collect(),
                agent: AgentSettings {
                    expert: task
                        .specialist_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|expert| !expert.is_empty())
                        .map(str::to_owned),
                    model: task.model_tier,
                    output_schema: task.output_schema.clone(),
                    isolated: task.isolated,
                    max_tokens: task.budget.as_ref().and_then(|budget| budget.max_tokens),
                    timeout_secs: task.timeout_secs,
                },
            }
        })
        .collect();
    let id = template.id.trim().to_owned();
    if id.is_empty() {
        return Err(WebViewHold::Unreadable);
    }
    crate::workflows::checked(&Workflow {
        id,
        name: template.name,
        goal: proposal.goal,
        tasks: converted,
        context: proposal.context,
        approval: proposal.approval_policy,
    })
    .map_err(|error| WebViewHold::Refused(error.to_string()))
}

/// The WebView's own workflows that come after the workflows `here`, in
/// its order; its built-in ones are its own and are left out.
fn plan_workflows(
    raw: Option<&String>,
    here: &[Workflow],
    experts: &HashSet<String>,
) -> Planned<Workflow> {
    let mut plan = Planned::new("workflows");
    let ids: HashSet<&str> = here.iter().map(|workflow| workflow.id.as_str()).collect();
    let mut names: HashSet<String> = here
        .iter()
        .map(|workflow| workflow.name.to_lowercase())
        .collect();
    let mut room = crate::workflows::LIMIT.saturating_sub(here.len());
    for entry in entries(raw) {
        let name = entry_name(&entry);
        let Ok(template) = serde_json::from_value::<WebViewTemplate>(entry) else {
            plan.hold(name, WebViewHold::Unreadable);
            continue;
        };
        if template.builtin || BUILT_IN_WORKFLOWS.contains(&template.id.trim()) {
            continue;
        }
        let workflow = match workflow_of(template, experts) {
            Ok(workflow) => workflow,
            Err(why) => {
                plan.hold(name, why);
                continue;
            }
        };
        if ids.contains(workflow.id.as_str()) || !names.insert(workflow.name.to_lowercase()) {
            plan.hold(workflow.name, WebViewHold::NameTaken);
        } else if room == 0 {
            plan.hold(workflow.name, WebViewHold::Full);
        } else {
            room -= 1;
            plan.items.push((workflow.name.clone(), workflow));
        }
    }
    plan
}

/// One of the WebView's quick actions as it keeps it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebViewAction {
    id: String,
    name: String,
    /// Shown in the WebView only, as is its icon.
    #[serde(default, rename = "description")]
    _description: serde::de::IgnoredAny,
    #[serde(default, rename = "icon")]
    _icon: serde::de::IgnoredAny,
    /// What it runs on: the passage selected, the only kind there is.
    context: String,
    workflow_template_id: String,
    #[serde(default = "switched_on")]
    enabled: bool,
    #[serde(default)]
    sort_order: i64,
    #[serde(default)]
    builtin: bool,
}

fn switched_on() -> bool {
    true
}

/// The WebView's own quick actions that come after the actions `here`, in
/// its order: each starts its workflow, which must be one of `workflows`.
fn plan_actions(
    raw: Option<&String>,
    here: &[QuickAction],
    workflows: &HashSet<String>,
) -> Planned<QuickAction> {
    let mut plan = Planned::new("quick_actions");
    let mut read = Vec::new();
    for entry in entries(raw) {
        let name = entry_name(&entry);
        match serde_json::from_value::<WebViewAction>(entry) {
            Ok(action) if action.context == "selection" => read.push(action),
            _ => plan.hold(name, WebViewHold::Unreadable),
        }
    }
    read.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| left.name.cmp(&right.name))
    });
    let ids: HashSet<&str> = here.iter().map(|action| action.id.as_str()).collect();
    let mut names: HashSet<String> = here
        .iter()
        .map(|action| action.name.trim().to_lowercase())
        .collect();
    let mut room = crate::quick_actions::LIMIT.saturating_sub(here.len());
    for action in read {
        if action.builtin || action.id == BUILT_IN_ACTION {
            continue;
        }
        let name = action.name.trim().to_owned();
        let id = action.id.trim();
        let workflow = action.workflow_template_id.trim();
        let limit = crate::quick_actions::NAME_LIMIT;
        let why = if !action.enabled {
            Some(WebViewHold::Off)
        } else if BUILT_IN_WORKFLOWS.contains(&workflow) {
            Some(WebViewHold::BuiltInWorkflow)
        } else if !workflows.contains(workflow) {
            Some(WebViewHold::NoWorkflow)
        } else if name.is_empty() || id.is_empty() {
            Some(WebViewHold::Unreadable)
        } else if name.chars().count() > limit {
            Some(WebViewHold::Refused(format!(
                "A name is at most {limit} characters"
            )))
        } else if ids.contains(id) || !names.insert(name.to_lowercase()) {
            Some(WebViewHold::NameTaken)
        } else if room == 0 {
            Some(WebViewHold::Full)
        } else {
            None
        };
        match why {
            Some(why) => plan.hold(name, why),
            None => {
                room -= 1;
                plan.items.push((
                    name.clone(),
                    QuickAction {
                        id: id.to_owned(),
                        name,
                        prompt: String::new(),
                        workflow: workflow.to_owned(),
                    },
                ));
            }
        }
    }
    plan
}

/// The WebView's favorites, in the order they were saved, each keeping
/// its link to the conversation it was saved from only while that
/// conversation is here in the same project; otherwise the link is
/// dropped and its project's and conversation's names stay as words.
async fn plan_favorites(
    store: &wisp_store::Store,
    records: Vec<LibraryRecord>,
) -> Result<Planned<LibraryRecord>> {
    let mut plan = Planned::new("favorites");
    let mut found: HashMap<String, Option<String>> = HashMap::new();
    for mut record in records {
        if !record.fits() {
            plan.hold(record.title.clone(), WebViewHold::Unreadable);
            continue;
        }
        if !record.source_session_id.is_empty() {
            let session = record.source_session_id.clone();
            let project = match found.get(&session) {
                Some(project) => project.clone(),
                None => {
                    let project = store.live_frame_project_id(&session).await?;
                    found.insert(session, project.clone());
                    project
                }
            };
            if project.as_deref() != Some(record.source_project_id.as_str()) {
                plan.unlinked += 1;
                record.source_project_id.clear();
                record.source_session_id.clear();
            }
        }
        plan.items.push((record.title.clone(), record));
    }
    Ok(plan)
}

/// The ids of the experts the WebView keeps.
fn webview_experts(values: &HashMap<String, String>) -> impl Iterator<Item = String> {
    entries(values.get(WEBVIEW_EXPERTS))
        .into_iter()
        .filter_map(|entry| Some(entry.get("id")?.as_str()?.trim().to_owned()))
        .filter(|id| !id.is_empty())
}

impl Server {
    /// The ids of the experts here, the built-in one among them.
    async fn expert_ids(&self) -> HashSet<String> {
        crate::experts::list(&self.0.store)
            .await
            .into_iter()
            .map(|expert| expert.id)
            .collect()
    }

    /// The WebView's favorites in its library at `library`, as they would
    /// come; `None` when it has none, or while this desktop has some.
    async fn favorites_plan(&self, library: &Path) -> Result<Option<Planned<LibraryRecord>>> {
        if !library.is_file() || !self.library().await?.is_empty().await? {
            return Ok(None);
        }
        let Ok(records) = wisp_store::LibraryStore::read_records(library).await else {
            let mut plan = Planned::new("favorites");
            plan.hold(String::new(), WebViewHold::Unreadable);
            return Ok(Some(plan));
        };
        if records.is_empty() {
            return Ok(None);
        }
        plan_favorites(&self.0.store, records).await.map(Some)
    }

    /// The WebView's lists that can come now, with what of each would
    /// come: each only while this desktop has none of it. `settings` are
    /// the groups of settings that can come, as experts a workflow needs
    /// may come with theirs; the library is its favorites.
    async fn webview_lists(
        &self,
        values: &HashMap<String, String>,
        settings: &[String],
        library: &Path,
    ) -> Result<Vec<WebViewGroup>> {
        let store = &self.0.store;
        let secrets = self.0.secrets.as_ref();
        let mut groups = Vec::new();
        if crate::mcp::load(store).await.is_empty() {
            let plan = plan_servers(values.get(WEBVIEW_MCP), secrets);
            if !plan.is_empty() {
                groups.push(plan.shown());
            }
        }
        if store
            .get_setting(crate::decision_model::SETTINGS_KEY)
            .await?
            .is_none()
        {
            if let Some(plan) = plan_decision(values.get(WEBVIEW_DECISION), secrets) {
                groups.push(plan.shown());
            }
        }
        let here = crate::workflows::list(store).await;
        let mut workflows: HashSet<String> =
            here.iter().map(|workflow| workflow.id.clone()).collect();
        if store.get_setting(crate::workflows::KEY).await?.is_none() {
            let mut experts = self.expert_ids().await;
            if settings.iter().any(|group| group == "experts") {
                experts.extend(webview_experts(values));
            }
            let plan = plan_workflows(values.get(WEBVIEW_WORKFLOWS), &here, &experts);
            workflows.extend(plan.items.iter().map(|(_, workflow)| workflow.id.clone()));
            if !plan.is_empty() {
                groups.push(plan.shown());
            }
        }
        if store
            .get_setting(crate::quick_actions::KEY)
            .await?
            .is_none()
        {
            let here = crate::quick_actions::list(store).await;
            let plan = plan_actions(values.get(WEBVIEW_ACTIONS), &here, &workflows);
            if !plan.is_empty() {
                groups.push(plan.shown());
            }
        }
        if let Some(plan) = self.favorites_plan(library).await? {
            groups.push(plan.shown());
        }
        Ok(groups)
    }

    /// Brings the chosen lists over, each only while this desktop has
    /// none of it, after the settings so that the experts a workflow needs
    /// are here, and workflows before the quick actions that start them.
    /// Secrets go from the WebView's keychain entries to this desktop's;
    /// one the keychain does not give is entered again here.
    async fn bring_lists(
        &self,
        values: &HashMap<String, String>,
        chosen: &[String],
        library: &Path,
    ) -> Result<Vec<WebViewGroup>> {
        let store = &self.0.store;
        let secrets = self.0.secrets.as_ref();
        let picked = |group: &str| chosen.iter().any(|chosen| chosen == group);
        let mut done = Vec::new();
        if picked("mcp_servers") && crate::mcp::load(store).await.is_empty() {
            let plan = plan_servers(values.get(WEBVIEW_MCP), secrets);
            if !plan.is_empty() {
                let mut shown = plan.shown();
                shown.without_secret.clear();
                let mut servers = Vec::with_capacity(plan.items.len());
                if !plan.items.is_empty() {
                    let key_id = self.key_id().await?;
                    for (_, planned) in plan.items {
                        let mut missing = !planned.missing.is_empty();
                        if !planned.values.is_empty() {
                            let (env, headers) = match planned.server.transport {
                                McpTransport::Stdio { .. } => (planned.values, BTreeMap::new()),
                                McpTransport::Http { .. } => (BTreeMap::new(), planned.values),
                            };
                            let extra =
                                serde_json::json!({"env": env, "headers": headers}).to_string();
                            let secret = Server::mcp_extra_secret(&key_id, &planned.server.id);
                            // Not kept, so entered again here.
                            missing |= secrets.set(&secret, &extra).is_err();
                        }
                        if missing {
                            shown.without_secret.push(planned.server.name.clone());
                        }
                        servers.push(planned.server);
                    }
                    crate::mcp::save(store, &servers).await?;
                }
                done.push(shown);
            }
        }
        if picked("decision_model")
            && store
                .get_setting(crate::decision_model::SETTINGS_KEY)
                .await?
                .is_none()
        {
            if let Some(plan) = plan_decision(values.get(WEBVIEW_DECISION), secrets) {
                let mut shown = plan.shown();
                for (name, (settings, keys)) in plan.items {
                    let needs = settings.enabled
                        && DecisionPreset::find(&settings.preset)
                            .is_some_and(|preset| preset.key_required);
                    let preset = settings.preset.clone();
                    let key_id = self.key_id().await?;
                    let kept =
                        crate::decision_model::adopt(store, secrets, &key_id, settings, &keys)
                            .await?;
                    shown.without_secret.clear();
                    if needs && !kept.contains(&preset) {
                        shown.without_secret.push(name);
                    }
                }
                done.push(shown);
            }
        }
        if picked("workflows") && store.get_setting(crate::workflows::KEY).await?.is_none() {
            let mut here = crate::workflows::list(store).await;
            let experts = self.expert_ids().await;
            let plan = plan_workflows(values.get(WEBVIEW_WORKFLOWS), &here, &experts);
            if !plan.is_empty() {
                let shown = plan.shown();
                if !plan.items.is_empty() {
                    here.extend(plan.items.into_iter().map(|(_, workflow)| workflow));
                    crate::workflows::save(store, &here).await?;
                }
                done.push(shown);
            }
        }
        if picked("quick_actions")
            && store
                .get_setting(crate::quick_actions::KEY)
                .await?
                .is_none()
        {
            let workflows: HashSet<String> = crate::workflows::list(store)
                .await
                .into_iter()
                .map(|workflow| workflow.id)
                .collect();
            let mut here = crate::quick_actions::list(store).await;
            let plan = plan_actions(values.get(WEBVIEW_ACTIONS), &here, &workflows);
            if !plan.is_empty() {
                let shown = plan.shown();
                if !plan.items.is_empty() {
                    here.extend(plan.items.into_iter().map(|(_, action)| action));
                    crate::quick_actions::save(store, &here).await?;
                }
                done.push(shown);
            }
        }
        if picked("favorites") {
            if let Some(plan) = self.favorites_plan(library).await? {
                let mut shown = plan.shown();
                let records: Vec<LibraryRecord> =
                    plan.items.into_iter().map(|(_, record)| record).collect();
                let added: HashSet<String> = self
                    .library()
                    .await?
                    .adopt(&records)
                    .await?
                    .into_iter()
                    .collect();
                shown.items = records
                    .iter()
                    .filter(|record| added.contains(&record.id))
                    .map(|record| record.title.clone())
                    .collect();
                done.push(shown);
            }
        }
        Ok(done)
    }
}

impl Server {
    /// The WebView desktop's projects and settings in its database at
    /// `database`, read without changing it: each project marked when it
    /// is already here, the groups of settings this desktop has not made,
    /// and its lists this desktop has none of, from its database and the
    /// favorites library beside it.
    pub async fn webview_data(&self, database: &Path) -> Result<WebViewData> {
        let pool = read_only(database).await?;
        let read = async {
            anyhow::Ok((
                projects(&pool).await?,
                setting_values(&pool).await?,
                webview_profiles(&pool).await?,
            ))
        }
        .await;
        pool.close().await;
        let (mut projects, values, (profiles, active)) = read?;
        for project in &mut projects {
            project.here = self.0.store.get_project(&project.id).await?.is_some();
        }
        let here: HashSet<String> = self
            .model_settings()
            .await?
            .profiles
            .into_iter()
            .map(|profile| profile.id)
            .collect();
        let models = profiles
            .into_iter()
            .map(|profile| WebViewModel {
                has_key: match crate::subscriptions::Subscription::of_provider(&profile.provider) {
                    Some(subscription) => {
                        crate::subscriptions::load(self.0.secrets.as_ref(), subscription).is_some()
                    }
                    None => self.0.secrets.get(&webview_key(&profile.id)).is_some(),
                },
                active: profile.id == active,
                here: here.contains(&profile.id),
                name: match profile.label.trim() {
                    "" => profile.model.trim().to_owned(),
                    label => label.to_owned(),
                },
                model: profile.model.trim().to_owned(),
                provider: profile.provider.trim().to_owned(),
                id: profile.id,
            })
            .collect();
        let settings = offered(&self.0.store, &values).await?;
        let groups = self
            .webview_lists(&values, &settings, &database.with_file_name(LIBRARY))
            .await?;
        Ok(WebViewData {
            projects,
            models,
            settings,
            groups,
        })
    }

    /// Brings the WebView desktop's projects `ids`, groups `settings` (of
    /// [`SETTING_GROUPS`] and of [`LIST_GROUPS`]) and saved models
    /// `models` over from its database at `database`, which is not
    /// changed, nor is the favorites library beside it: a project kept in
    /// the WebView's database is copied, with its settings, and goes on
    /// using its folder where it is, and one that keeps its database in
    /// its folder is opened there; a group comes only while none of it is
    /// here; a model comes with its key unless one with its id is here.
    /// The reply says how each project went, one that fails not stopping
    /// the rest, which groups and models came, and of each list what came
    /// and what stayed.
    pub async fn import_webview(
        &self,
        database: &Path,
        ids: &[String],
        settings: &[String],
        models: &[String],
    ) -> Result<WebViewOutcome> {
        if !models.is_empty() {
            self.editable_models()?;
        }
        // Exclusive, as any import is: no turn starts meanwhile.
        let _admission = self.0.admission.write().await;
        ensure!(
            !self.0.shutting_down.load(Ordering::Relaxed),
            "Host is shutting down"
        );
        let data = &self.0.config.data;
        let scratch = data.join(SCRATCH);
        // What an earlier import could not remove goes first.
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch)?;
        let copy = scratch.join("wisp.sqlite");
        let pool = read_only(database).await?;
        let copied = sqlx::query("VACUUM INTO ?")
            .bind(copy.to_string_lossy().as_ref())
            .execute(&pool)
            .await;
        pool.close().await;
        copied.context("Cannot copy the WebView's database")?;

        // The copy forgets where projects keep their own databases, so
        // nothing read from it reaches into their folders.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&copy))
            .await?;
        let listed = async {
            let found = projects(&pool).await?;
            if has_table(&pool, "project_locations").await? {
                sqlx::query("DELETE FROM project_locations")
                    .execute(&pool)
                    .await?;
            }
            anyhow::Ok((
                found,
                setting_values(&pool).await?,
                webview_profiles(&pool).await?,
            ))
        }
        .await;
        let (found, values, (profiles, active)) = match listed {
            Ok(listed) => listed,
            Err(error) => {
                pool.close().await;
                return Err(error);
            }
        };
        let records = match wisp_store::Store::open(&copy).await {
            Ok(records) => records,
            Err(error) => {
                pool.close().await;
                return Err(error.context("Cannot read the copy of the WebView's database"));
            }
        };

        let mut done = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(project) = found.iter().find(|project| &project.id == id) else {
                done.push(WebViewImport {
                    id: id.clone(),
                    name: String::new(),
                    error: Some("The WebView has no such project".into()),
                });
                continue;
            };
            let result = bring(&self.0.store, &records, &pool, data, &scratch, project).await;
            done.push(WebViewImport {
                id: project.id.clone(),
                name: project.name.clone(),
                error: result.err().map(|error| format!("{error:#}")),
            });
        }
        pool.close().await;
        drop(records);
        let _ = std::fs::remove_dir_all(&scratch);

        let offered = offered(&self.0.store, &values).await?;
        let mut brought = Vec::new();
        for (group, keys) in SETTING_GROUPS {
            if !settings.iter().any(|chosen| chosen == group) || !offered.iter().any(|g| g == group)
            {
                continue;
            }
            for key in *keys {
                if let Some(value) = values.get(*key) {
                    self.0.store.set_setting(key, value).await?;
                }
            }
            brought.push((*group).to_owned());
        }
        let groups = self
            .bring_lists(&values, settings, &database.with_file_name(LIBRARY))
            .await?;

        let chosen: Vec<_> = profiles
            .iter()
            .filter(|profile| models.contains(&profile.id))
            .map(|profile| {
                // A subscription runs on its sign-in, which both share.
                let key = crate::subscriptions::Subscription::of_provider(&profile.provider)
                    .is_none()
                    .then(|| self.0.secrets.get(&webview_key(&profile.id)))
                    .flatten();
                (
                    profile.id.clone(),
                    profile.label.clone(),
                    profile.fields(),
                    key,
                )
            })
            .collect();
        let models = if chosen.is_empty() {
            Vec::new()
        } else {
            self.adopt_model_profiles(chosen, Some(active.as_str()))
                .await?
        };
        Ok(WebViewOutcome {
            projects: done,
            models,
            settings: brought,
            groups,
        })
    }
}
