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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

use crate::Server;

/// The folder in this host's data folder the copy is made in; emptied
/// before and after every import.
const SCRATCH: &str = "webview-import";

/// The WebView's saved models, kept as one JSON list.
const MODELS: &str = "model_profiles";
/// The id of the WebView's everyday model.
const ACTIVE_MODEL: &str = "active_model_id";

/// The WebView's settings that mean the same here, in the groups they
/// are offered by: a group's name and its keys. The decision model, whose
/// key is kept under another name, does not come.
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
}

/// How an import went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebViewOutcome {
    pub projects: Vec<WebViewImport>,
    /// The ids of the saved models brought over.
    pub models: Vec<String>,
    /// The groups of settings brought over.
    pub settings: Vec<String>,
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

/// The WebView's values for the keys of [`SETTING_GROUPS`].
async fn setting_values(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    if !has_table(pool, "settings").await? {
        return Ok(values);
    }
    for key in SETTING_GROUPS.iter().flat_map(|(_, keys)| keys.iter()) {
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

impl Server {
    /// The WebView desktop's projects and settings in its database at
    /// `database`, read without changing it: each project marked when it
    /// is already here, and the groups of settings this desktop has not
    /// made.
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
        Ok(WebViewData {
            projects,
            models,
            settings: offered(&self.0.store, &values).await?,
        })
    }

    /// Brings the WebView desktop's projects `ids`, groups of settings
    /// `settings` and saved models `models` over from its database at
    /// `database`, which is not changed: a project kept in the WebView's
    /// database is copied, with its settings, and goes on using its folder
    /// where it is, and one that keeps its database in its folder is
    /// opened there; a group of settings comes only while none of it is
    /// set here; a model comes with its key unless one with its id is
    /// here. The reply says how each project went, one that fails not
    /// stopping the rest, and which groups and models came.
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
        })
    }
}
