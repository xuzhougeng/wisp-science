use super::{ProjectSyncState, Store};
use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteConnection, SqlitePoolOptions},
    Column, Connection, Row, Sqlite, Transaction, TypeInfo, ValueRef,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectTransferStats {
    pub frames: i64,
    pub messages: i64,
    pub artifacts: i64,
    pub runs: i64,
    pub path_warnings: i64,
}

/// Copy every project-owned row from the attached `transfer` database.
/// Machine-global settings, execution contexts, credentials, and ACP runtime
/// bindings are deliberately not project data and are not copied.
async fn copy_project_children(tx: &mut Transaction<'_, Sqlite>, project_id: &str) -> Result<()> {
    const QUERIES: &[&str] = &[
        "INSERT INTO folders(id,project_id,name,created_at,updated_at) \
         SELECT id,project_id,name,created_at,updated_at FROM transfer.folders WHERE project_id=?",
        "INSERT INTO agent_workflows(id,project_id,workspace_id,name,description,version,enabled,created_at,updated_at) \
         SELECT id,project_id,workspace_id,name,description,version,enabled,created_at,updated_at \
         FROM transfer.agent_workflows WHERE project_id=?",
        "INSERT INTO agent_workflow_steps(id,workflow_id,position,agent_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,input_contract_json,output_contract_json,permissions_json,context_policy_json,budget_json,timeout_secs,created_at,updated_at) \
         SELECT s.id,s.workflow_id,s.position,s.agent_id,s.role,s.backend,s.model,s.prompt_template,s.input_schema_json,s.output_schema_json,s.input_contract_json,s.output_contract_json,s.permissions_json,s.context_policy_json,s.budget_json,s.timeout_secs,s.created_at,s.updated_at \
         FROM transfer.agent_workflow_steps s JOIN transfer.agent_workflows w ON w.id=s.workflow_id WHERE w.project_id=?",
        "INSERT INTO frames(id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,input_tokens,output_tokens,created_at,updated_at,completed_at,title) \
         SELECT id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,input_tokens,output_tokens,created_at,updated_at,completed_at,title \
         FROM transfer.frames WHERE project_id=?",
        "INSERT INTO messages(id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name) \
         SELECT id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name FROM transfer.messages \
         WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO session_reviews(id,frame_id,message_seq,report_json,created_at,updated_at) \
         SELECT id,frame_id,message_seq,report_json,created_at,updated_at FROM transfer.session_reviews \
         WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO session_ui_events(frame_id,seq,event_json) \
         SELECT frame_id,seq,event_json FROM transfer.session_ui_events \
         WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO proposed_plans(id,frame_id,codex_thread_id,codex_turn_id,revision,markdown,status,mode,progress_json,runtime_config_json,created_at,updated_at) \
         SELECT id,frame_id,codex_thread_id,codex_turn_id,revision,markdown,status,mode,progress_json,runtime_config_json,created_at,updated_at \
         FROM transfer.proposed_plans WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO codex_turn_configs(id,frame_id,codex_thread_id,codex_turn_id,mode,config_version,config_version_text,requested_json,effective_json,actual_json,created_at,updated_at) \
         SELECT id,frame_id,codex_thread_id,codex_turn_id,mode,config_version,config_version_text,requested_json,effective_json,actual_json,created_at,updated_at \
         FROM transfer.codex_turn_configs WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO execution_log(id,frame_id,cell_index,tool,language,source,stdout,stderr,exit_status,wall_s,files_written,files_read,env_hash,created_at) \
         SELECT id,frame_id,cell_index,tool,language,source,stdout,stderr,exit_status,wall_s,files_written,files_read,env_hash,created_at \
         FROM transfer.execution_log WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT OR IGNORE INTO env_snapshots(hash,env_name,packages_json,created_at) \
         SELECT hash,env_name,packages_json,created_at FROM transfer.env_snapshots WHERE hash IN (\
           SELECT env_hash FROM transfer.execution_log WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?))",
        "INSERT INTO runs(id,project_id,frame_id,context_id,title,kind,status,command,script_path,input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,last_polled_at,last_poll_error,lifecycle_owner,lifecycle_lease_until,env_snapshot_json) \
         SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,last_polled_at,last_poll_error,lifecycle_owner,lifecycle_lease_until,env_snapshot_json \
         FROM transfer.runs WHERE project_id=?",
        "INSERT INTO artifacts(id,project_id,root_frame_id,filename,content_type,storage_path,created_at,latest_version_id) \
         SELECT id,project_id,root_frame_id,filename,content_type,storage_path,created_at,latest_version_id \
         FROM transfer.artifacts WHERE project_id=?",
        "INSERT OR IGNORE INTO env_snapshots(hash,env_name,packages_json,created_at) \
         SELECT hash,env_name,packages_json,created_at FROM transfer.env_snapshots WHERE hash IN (\
           SELECT av.env_snapshot_hash FROM transfer.artifact_versions av JOIN transfer.artifacts a ON a.id=av.artifact_id WHERE a.project_id=?)",
        "INSERT INTO artifact_versions(id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,parent_version_id,producing_run_id,env_snapshot_hash,created_at) \
         SELECT av.id,av.artifact_id,av.version_number,av.content_type,av.storage_path,av.size_bytes,av.checksum,av.parent_version_id,av.producing_run_id,av.env_snapshot_hash,av.created_at \
         FROM transfer.artifact_versions av JOIN transfer.artifacts a ON a.id=av.artifact_id WHERE a.project_id=?",
        "INSERT INTO message_resource_links(id,frame_id,message_seq,ordinal,original_reference,artifact_id,artifact_version_id,display_name,resource_kind,mime_type,status,error,created_at) \
         SELECT id,frame_id,message_seq,ordinal,original_reference,artifact_id,artifact_version_id,display_name,resource_kind,mime_type,status,error,created_at \
         FROM transfer.message_resource_links WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO artifact_dependencies(id,artifact_version_id,depends_on_version_id,reference_name,created_at) \
         SELECT d.id,d.artifact_version_id,d.depends_on_version_id,d.reference_name,d.created_at FROM transfer.artifact_dependencies d \
         WHERE d.artifact_version_id IN (SELECT av.id FROM transfer.artifact_versions av JOIN transfer.artifacts a ON a.id=av.artifact_id WHERE a.project_id=?)",
        "INSERT INTO run_artifacts(id,run_id,artifact_id,role,created_at) \
         SELECT id,run_id,artifact_id,role,created_at FROM transfer.run_artifacts \
         WHERE run_id IN (SELECT id FROM transfer.runs WHERE project_id=?)",
        "INSERT INTO research_nodes(id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at) \
         SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at FROM transfer.research_nodes WHERE project_id=?",
        "INSERT INTO research_edges(id,project_id,source_id,target_id,relation,metadata_json,created_at) \
         SELECT id,project_id,source_id,target_id,relation,metadata_json,created_at FROM transfer.research_edges WHERE project_id=?",
    ];

    for query in QUERIES {
        sqlx::query(query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Remove every row owned by a project while retaining the project itself.
/// SQLite foreign keys are not enabled on legacy stores, so replacement must
/// spell out the cascade in dependency order.
pub(crate) async fn delete_project_children(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<()> {
    const QUERIES: &[&str] = &[
        "DELETE FROM artifact_dependencies WHERE artifact_version_id IN (SELECT av.id FROM artifact_versions av JOIN artifacts a ON a.id=av.artifact_id WHERE a.project_id=?)",
        "DELETE FROM agent_workflow_steps WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE project_id=?)",
        "DELETE FROM agent_workflows WHERE project_id=?",
        "DELETE FROM message_resource_links WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM session_execution_contexts WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM artifact_versions WHERE artifact_id IN (SELECT id FROM artifacts WHERE project_id=?)",
        "DELETE FROM run_artifacts WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM session_reviews WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM session_ui_events WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM proposed_plans WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM codex_turn_configs WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM acp_sessions WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM execution_log WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM research_edges WHERE project_id=?",
        "DELETE FROM research_nodes WHERE project_id=?",
        "DELETE FROM artifacts WHERE project_id=?",
        "DELETE FROM runs WHERE project_id=?",
        "DELETE FROM frames WHERE project_id=?",
        "DELETE FROM folders WHERE project_id=?",
    ];
    for query in QUERIES {
        sqlx::query(query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn sanitize_export_machine_state(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<()> {
    // Handles, leases, remote work directories and the launch environment can
    // contain hostnames, process ids and private-key paths. They are runtime
    // state, not portable research history.
    sqlx::query(
        "UPDATE runs SET remote_workdir=NULL,remote_handle_json=NULL,\
         lifecycle_owner=NULL,lifecycle_lease_until=NULL,env_snapshot_json='{}' \
         WHERE project_id=?",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/')
        || path.starts_with("//")
}

fn normalize_path(raw: &str) -> Option<(String, bool)> {
    let path = raw.replace('\\', "/");
    let windows = is_windows_absolute(&path);
    let (prefix, rest, absolute) = if windows && path.as_bytes().get(1) == Some(&b':') {
        (&path[..2], &path[3..], true)
    } else if path.starts_with("//") {
        ("//", path.trim_start_matches('/'), true)
    } else if let Some(rest) = path.strip_prefix('/') {
        ("/", rest, true)
    } else {
        ("", path.as_str(), false)
    };
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.pop().is_none() => return None,
            ".." => {}
            value => parts.push(value),
        }
    }
    let joined = parts.join("/");
    let normalized = match prefix {
        "/" => format!("/{joined}"),
        "//" => format!("//{joined}"),
        "" => joined,
        drive => format!("{drive}/{joined}"),
    };
    Some((normalized, absolute))
}

fn unavailable_path(raw: &str) -> String {
    let name = raw
        .replace('\\', "/")
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("file")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("wisp-unavailable://source-local/{name}")
}

/// Convert a path written on the source OS to the archive's portable form.
/// This intentionally does not use host `Path` parsing so Windows drive paths
/// are testable and recognizable when the importer runs on macOS/Linux.
fn portable_project_path(source_root: &str, raw: &str) -> (String, bool) {
    let value = raw.trim();
    if let Some(file_path) = value.strip_prefix("file://") {
        let file_path = if file_path.starts_with('/')
            && file_path.as_bytes().get(2) == Some(&b':')
            && file_path
                .as_bytes()
                .get(1)
                .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            &file_path[1..]
        } else {
            file_path
        };
        return portable_project_path(source_root, file_path);
    }
    if value.contains("://") {
        return (value.to_string(), false);
    }
    let Some((path, absolute)) = normalize_path(value) else {
        return (unavailable_path(value), true);
    };
    if !absolute {
        return (path, false);
    }
    let Some((root, root_absolute)) = normalize_path(source_root) else {
        return (unavailable_path(value), true);
    };
    if !root_absolute {
        return (unavailable_path(value), true);
    }
    let windows = is_windows_absolute(&root);
    let (candidate, base) = if windows {
        (path.to_ascii_lowercase(), root.to_ascii_lowercase())
    } else {
        (path.clone(), root.clone())
    };
    if candidate == base {
        return (String::new(), false);
    }
    let prefix = format!("{}/", base.trim_end_matches('/'));
    if candidate.starts_with(&prefix) {
        return (path[prefix.len()..].to_string(), false);
    }
    (unavailable_path(value), true)
}

fn restored_project_path(workspace: &Path, archived: &str) -> Result<String> {
    if archived.contains("://") {
        return Ok(archived.to_string());
    }
    let (relative, absolute) = normalize_path(archived)
        .ok_or_else(|| anyhow::anyhow!("archive contains an unsafe project path"))?;
    if absolute {
        anyhow::bail!("archive contains a non-portable absolute project path");
    }
    let workspace = workspace.to_string_lossy();
    let separator = if workspace.contains('\\') && !workspace.contains('/') {
        '\\'
    } else {
        '/'
    };
    let workspace = workspace.trim_end_matches(['/', '\\']);
    if relative.is_empty() {
        return Ok(workspace.to_string());
    }
    let relative = if separator == '\\' {
        relative.replace('/', "\\")
    } else {
        relative
    };
    Ok(format!("{workspace}{separator}{relative}"))
}

async fn rewrite_export_paths(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    source_root: &str,
) -> Result<i64> {
    let mut warnings = 0;
    let artifacts = sqlx::query("SELECT id,storage_path FROM artifacts WHERE project_id=?")
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
    for row in artifacts {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        let (portable, warned) = portable_project_path(source_root, &value);
        warnings += i64::from(warned);
        sqlx::query("UPDATE artifacts SET storage_path=? WHERE id=?")
            .bind(portable)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let versions = sqlx::query(
        "SELECT av.id,av.storage_path FROM artifact_versions av \
         JOIN artifacts a ON a.id=av.artifact_id WHERE a.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in versions {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        let (portable, warned) = portable_project_path(source_root, &value);
        warnings += i64::from(warned);
        sqlx::query("UPDATE artifact_versions SET storage_path=? WHERE id=?")
            .bind(portable)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let runs = sqlx::query(
        "SELECT id,script_path,input_refs_json,output_specs_json FROM runs WHERE project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in runs {
        let id: String = row.try_get("id")?;
        let script_path: Option<String> = row.try_get("script_path")?;
        let script_path = script_path.map(|value| {
            let (portable, warned) = portable_project_path(source_root, &value);
            warnings += i64::from(warned);
            portable
        });
        let input_refs: String = row.try_get("input_refs_json")?;
        let input_refs = serde_json::from_str::<Vec<String>>(&input_refs)
            .map(|paths| {
                paths
                    .into_iter()
                    .map(|value| {
                        let (portable, warned) = portable_project_path(source_root, &value);
                        warnings += i64::from(warned);
                        portable
                    })
                    .collect::<Vec<_>>()
            })
            .and_then(|paths| serde_json::to_string(&paths))
            .unwrap_or(input_refs);
        let output_specs: String = row.try_get("output_specs_json")?;
        let output_specs = serde_json::from_str::<serde_json::Value>(&output_specs)
            .map(|mut value| {
                if let Some(specs) = value.as_array_mut() {
                    for spec in specs {
                        let Some(glob) = spec.get_mut("glob") else {
                            continue;
                        };
                        let Some(path) = glob.as_str() else { continue };
                        let (portable, warned) = portable_project_path(source_root, path);
                        warnings += i64::from(warned);
                        *glob = serde_json::Value::String(portable);
                    }
                }
                value
            })
            .and_then(|value| serde_json::to_string(&value))
            .unwrap_or(output_specs);
        sqlx::query(
            "UPDATE runs SET script_path=?,input_refs_json=?,output_specs_json=? WHERE id=?",
        )
        .bind(script_path)
        .bind(input_refs)
        .bind(output_specs)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(warnings)
}

async fn restore_import_paths(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    workspace: &Path,
) -> Result<()> {
    let artifacts = sqlx::query("SELECT id,storage_path FROM artifacts WHERE project_id=?")
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
    for row in artifacts {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        sqlx::query("UPDATE artifacts SET storage_path=? WHERE id=?")
            .bind(restored_project_path(workspace, &value)?)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let versions = sqlx::query(
        "SELECT av.id,av.storage_path FROM artifact_versions av \
         JOIN artifacts a ON a.id=av.artifact_id WHERE a.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in versions {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        sqlx::query("UPDATE artifact_versions SET storage_path=? WHERE id=?")
            .bind(restored_project_path(workspace, &value)?)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let runs = sqlx::query(
        "SELECT id,script_path FROM runs WHERE project_id=? AND script_path IS NOT NULL",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in runs {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("script_path")?;
        sqlx::query("UPDATE runs SET script_path=? WHERE id=?")
            .bind(restored_project_path(workspace, &value)?)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

impl Store {
    async fn database_path(&self) -> Result<PathBuf> {
        let rows = sqlx::query("PRAGMA database_list")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .find(|row| matches!(row.try_get::<String, _>("name"), Ok(name) if name == "main"))
            .and_then(|row| row.try_get::<String, _>("file").ok())
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("project transfer requires a file-backed database"))
    }

    /// Stable logical fingerprint of a filtered project database. SQLite file
    /// headers and page layouts are intentionally ignored because they can
    /// change after VACUUM or across operating systems without a data change.
    pub async fn portable_project_database_hash(database: &Path) -> Result<String> {
        const TABLES: &[(&str, &str, &str)] = &[
            // `updated_at` is touched when a project is merely opened on one
            // device. Name/description still participate in the fingerprint.
            (
                "projects",
                "id,name,description,workspace_dir,created_at",
                "id",
            ),
            ("folders", "*", "id"),
            ("agent_workflows", "*", "id"),
            ("agent_workflow_steps", "*", "id"),
            ("frames", "*", "id"),
            ("messages", "*", "id"),
            ("session_reviews", "*", "id"),
            ("session_ui_events", "*", "frame_id,seq"),
            ("proposed_plans", "*", "id"),
            ("codex_turn_configs", "*", "id"),
            ("execution_log", "*", "id"),
            ("env_snapshots", "*", "hash"),
            ("runs", "*", "id"),
            ("artifacts", "*", "id"),
            ("artifact_versions", "*", "id"),
            ("message_resource_links", "*", "id"),
            ("artifact_dependencies", "*", "id"),
            ("run_artifacts", "*", "id"),
            ("research_nodes", "*", "id"),
            ("research_edges", "*", "id"),
        ];
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", database.display()))?
            .read_only(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let mut digest = Sha256::new();
        for (table, columns, order) in TABLES {
            digest.update((table.len() as u64).to_le_bytes());
            digest.update(table.as_bytes());
            let query = format!("SELECT {columns} FROM {table} ORDER BY {order}");
            let mut rows = sqlx::query(&query).fetch(&pool);
            while let Some(row) = rows.try_next().await? {
                for (index, column) in row.columns().iter().enumerate() {
                    let raw = row.try_get_raw(index)?;
                    digest.update((column.name().len() as u64).to_le_bytes());
                    digest.update(column.name().as_bytes());
                    if raw.is_null() {
                        digest.update([0]);
                        continue;
                    }
                    digest.update([1]);
                    let bytes = match column.type_info().name() {
                        "INTEGER" | "INT8" => row.try_get::<i64, _>(index)?.to_le_bytes().to_vec(),
                        "REAL" | "FLOAT8" => row
                            .try_get::<f64, _>(index)?
                            .to_bits()
                            .to_le_bytes()
                            .to_vec(),
                        "BLOB" => row.try_get::<Vec<u8>, _>(index)?,
                        _ => row.try_get::<String, _>(index)?.into_bytes(),
                    };
                    digest.update((bytes.len() as u64).to_le_bytes());
                    digest.update(bytes);
                }
            }
        }
        pool.close().await;
        Ok(hex::encode(digest.finalize()))
    }

    /// Build a standalone, filtered SQLite snapshot for one project. Paths in
    /// operational columns are workspace-relative and slash-normalized.
    pub async fn export_project_database(
        &self,
        project_id: &str,
        destination: &Path,
    ) -> Result<ProjectTransferStats> {
        let (_, source_root) = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        if destination.exists() {
            std::fs::remove_file(destination)?;
        }
        let source_db = self.database_path().await?;
        let transfer = Store::open(destination).await?;
        let mut connection = transfer.pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ? AS transfer")
            .bind(source_db.to_string_lossy().as_ref())
            .execute(&mut *connection)
            .await?;
        let result: Result<i64> = async {
            let mut tx = connection.begin().await?;
            sqlx::query(
                "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) \
                 SELECT id,name,description,'',created_at,updated_at FROM transfer.projects WHERE id=?",
            )
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            copy_project_children(&mut tx, project_id).await?;
            sanitize_export_machine_state(&mut tx, project_id).await?;
            let warnings = rewrite_export_paths(&mut tx, project_id, &source_root).await?;
            tx.commit().await?;
            Ok(warnings)
        }
        .await;
        let _ = sqlx::query("DETACH DATABASE transfer")
            .execute(&mut *connection)
            .await;
        drop(connection);
        let warnings = result?;
        // Store::open creates machine-global defaults and records wall-clock
        // migration times. Remove/normalize them so an unchanged project makes
        // the same portable snapshot on every device.
        for query in [
            "DELETE FROM settings",
            "DELETE FROM execution_contexts",
            "DELETE FROM project_sync_state",
            "UPDATE wisp_schema_migrations SET applied_at=0",
        ] {
            sqlx::query(query).execute(&transfer.pool).await?;
        }
        let counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM frames WHERE project_id=?), \
               (SELECT COUNT(*) FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)), \
               (SELECT COUNT(*) FROM artifacts WHERE project_id=?), \
               (SELECT COUNT(*) FROM runs WHERE project_id=?)",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(project_id)
        .bind(project_id)
        .fetch_one(&transfer.pool)
        .await?;
        transfer.pool.close().await;
        // Reopen with exactly one connection before leaving WAL mode. A pool
        // can keep idle readers alive and make journal_mode changes fail with
        // SQLITE_BUSY; the resulting snapshot must be one standalone file.
        let options =
            SqliteConnectOptions::from_str(&format!("sqlite://{}", destination.display()))?;
        let mut standalone = SqliteConnection::connect_with(&options).await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut standalone)
            .await?;
        sqlx::query("PRAGMA journal_mode=DELETE")
            .execute(&mut standalone)
            .await?;
        sqlx::query("VACUUM").execute(&mut standalone).await?;
        standalone.close().await?;
        Ok(ProjectTransferStats {
            frames: counts.0,
            messages: counts.1,
            artifacts: counts.2,
            runs: counts.3,
            path_warnings: warnings,
        })
    }

    /// Import a v1 project snapshot into the live store. Project ids remain
    /// stable; a duplicate id is rejected instead of merging two histories.
    pub async fn import_project_database(
        &self,
        archive_database: &Path,
        project_id: &str,
        workspace: &Path,
    ) -> Result<()> {
        if self.get_project(project_id).await?.is_some() {
            anyhow::bail!("this project is already present on this device");
        }
        let mut connection = self.pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ? AS transfer")
            .bind(archive_database.to_string_lossy().as_ref())
            .execute(&mut *connection)
            .await
            .context("invalid project metadata database")?;
        let result: Result<()> = async {
            let archived_projects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects WHERE id=?")
                    .bind(project_id)
                    .fetch_one(&mut *connection)
                    .await?;
            let all_projects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects")
                .fetch_one(&mut *connection)
                .await?;
            if archived_projects != 1 || all_projects != 1 {
                anyhow::bail!("project archive metadata does not match its manifest");
            }
            let mut tx = connection.begin().await?;
            sqlx::query(
                "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) \
                 SELECT id,name,description,?,created_at,updated_at FROM transfer.projects WHERE id=?",
            )
            .bind(workspace.to_string_lossy().as_ref())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            copy_project_children(&mut tx, project_id).await?;
            restore_import_paths(&mut tx, project_id, workspace).await?;
            sqlx::query(
                "UPDATE runs SET status='lost', ended_at=COALESCE(ended_at,?), \
                 last_poll_error=COALESCE(last_poll_error,'Imported from another device; the run was not resumed.'), \
                 lifecycle_owner=NULL,lifecycle_lease_until=NULL \
                 WHERE project_id=? AND status IN ('submitted','running','cancelling')",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        let _ = sqlx::query("DETACH DATABASE transfer")
            .execute(&mut *connection)
            .await;
        result.context("could not import project metadata")
    }

    /// Replace an existing project's portable rows from a trusted, decrypted
    /// sync snapshot. The local workspace root and sync cursor remain
    /// device-specific; the cursor update commits in the same SQLite
    /// transaction as the project replacement.
    pub async fn replace_project_database(
        &self,
        archive_database: &Path,
        project_id: &str,
        workspace: &Path,
        sync_state: &ProjectSyncState,
    ) -> Result<()> {
        if sync_state.project_id != project_id {
            anyhow::bail!("sync cursor does not belong to the replaced project");
        }
        if self.get_project(project_id).await?.is_none() {
            anyhow::bail!("project to replace was not found");
        }
        let mut connection = self.pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ? AS transfer")
            .bind(archive_database.to_string_lossy().as_ref())
            .execute(&mut *connection)
            .await
            .context("invalid project sync metadata database")?;
        let result: Result<()> = async {
            let archived_projects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects WHERE id=?")
                    .bind(project_id)
                    .fetch_one(&mut *connection)
                    .await?;
            let all_projects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects")
                    .fetch_one(&mut *connection)
                    .await?;
            if archived_projects != 1 || all_projects != 1 {
                anyhow::bail!("sync metadata does not match the project");
            }
            let mut tx = connection.begin().await?;
            delete_project_children(&mut tx, project_id).await?;
            sqlx::query(
                "UPDATE projects SET \
                 name=(SELECT name FROM transfer.projects WHERE id=?), \
                 description=(SELECT description FROM transfer.projects WHERE id=?), \
                 created_at=(SELECT created_at FROM transfer.projects WHERE id=?), \
                 updated_at=(SELECT updated_at FROM transfer.projects WHERE id=?) \
                 WHERE id=?",
            )
            .bind(project_id)
            .bind(project_id)
            .bind(project_id)
            .bind(project_id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            copy_project_children(&mut tx, project_id).await?;
            restore_import_paths(&mut tx, project_id, workspace).await?;
            sqlx::query(
                "UPDATE runs SET status='lost',ended_at=COALESCE(ended_at,?),\
                 last_poll_error=COALESCE(last_poll_error,'Synced from another device; the run was not resumed.'),\
                 remote_workdir=NULL,remote_handle_json=NULL,lifecycle_owner=NULL,\
                 lifecycle_lease_until=NULL,env_snapshot_json='{}' \
                 WHERE project_id=? AND status IN ('submitted','running','cancelling')",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO project_sync_state(\
                 project_id,transport_kind,transport_location,relay_project_id,base_revision,base_state_hash,\
                 base_manifest_json,last_synced_at,last_direction) VALUES(?,?,?,?,?,?,?,?,?) \
                 ON CONFLICT(project_id) DO UPDATE SET transport_kind=excluded.transport_kind,\
                 transport_location=excluded.transport_location,relay_project_id=excluded.relay_project_id,base_revision=excluded.base_revision,\
                 base_state_hash=excluded.base_state_hash,base_manifest_json=excluded.base_manifest_json,\
                 last_synced_at=excluded.last_synced_at,last_direction=excluded.last_direction",
            )
            .bind(&sync_state.project_id)
            .bind(&sync_state.transport_kind)
            .bind(&sync_state.transport_location)
            .bind(&sync_state.relay_project_id)
            .bind(&sync_state.base_revision)
            .bind(&sync_state.base_state_hash)
            .bind(&sync_state.base_manifest_json)
            .bind(sync_state.last_synced_at)
            .bind(&sync_state.last_direction)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        let _ = sqlx::query("DETACH DATABASE transfer")
            .execute(&mut *connection)
            .await;
        result.context("could not replace project metadata from sync")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RunRecord, RunStatus};
    use wisp_llm::Message;

    #[test]
    fn windows_paths_become_portable_and_restore_on_macos() {
        let root = r"C:\Users\Alice\Wisp\Study";
        assert_eq!(
            portable_project_path(root, r"c:\users\alice\wisp\study\figures\plot.png"),
            ("figures/plot.png".into(), false)
        );
        assert_eq!(
            portable_project_path(root, r"file://C:\Users\Alice\Wisp\Study\data\x.csv"),
            ("data/x.csv".into(), false)
        );
        assert_eq!(
            portable_project_path(root, "file:///C:/Users/Alice/Wisp/Study/data/y.csv"),
            ("data/y.csv".into(), false)
        );
        let (outside, warned) = portable_project_path(root, r"D:\shared\large.fastq");
        assert!(warned);
        assert!(outside.starts_with("wisp-unavailable://"));
        assert_eq!(
            restored_project_path(Path::new("/Users/alice/Study"), "figures/plot.png").unwrap(),
            "/Users/alice/Study/figures/plot.png"
        );
        assert_eq!(
            restored_project_path(Path::new(r"C:\Users\Alice\Study"), "figures/plot.png").unwrap(),
            r"C:\Users\Alice\Study\figures\plot.png"
        );
        assert!(restored_project_path(Path::new("/tmp/study"), "../escape").is_err());
    }

    #[tokio::test]
    async fn project_database_roundtrip_rebinds_paths_and_stops_live_runs() {
        let token = uuid::Uuid::new_v4();
        let source_path = std::env::temp_dir().join(format!("wisp_transfer_source_{token}.sqlite"));
        let archive_path =
            std::env::temp_dir().join(format!("wisp_transfer_archive_{token}.sqlite"));
        let target_path = std::env::temp_dir().join(format!("wisp_transfer_target_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project-1", "Study", r"C:\Users\Alice\Study")
            .await
            .unwrap();
        let workflow =
            crate::AgentWorkflow::new("workflow-1", "project-1", "workspace-1", "Review QC")
                .unwrap();
        source.create_agent_workflow(&workflow).await.unwrap();
        let step = crate::AgentWorkflowStep::new(
            "workflow-step-1",
            "workflow-1",
            0,
            "reviewer",
            "reviewer",
            "acp",
            "Review {{input}}",
        )
        .unwrap();
        source.create_agent_workflow_step(&step).await.unwrap();
        source
            .create_frame("frame-1", "project-1", "OPERON", "model")
            .await
            .unwrap();
        source
            .append_message("frame-1", 1, &Message::user("hello"))
            .await
            .unwrap();
        let artifact_version_id = source
            .save_artifact(
                "artifact-1",
                "project-1",
                "frame-1",
                "plot.png",
                "image/png",
                r"C:\Users\Alice\Study\.wisp\artifacts\sha256\ab\abcdef.png",
            )
            .await
            .unwrap();
        source
            .replace_message_resource_links(
                "frame-1",
                1,
                &[crate::MessageResourceLink {
                    id: "resource-link-1".into(),
                    frame_id: "frame-1".into(),
                    message_seq: 1,
                    ordinal: 0,
                    original_reference: r"D:/original/location/plot.png".into(),
                    artifact_id: Some("artifact-1".into()),
                    artifact_version_id: Some(artifact_version_id.clone()),
                    display_name: "plot.png".into(),
                    resource_kind: "image".into(),
                    mime_type: "image/png".into(),
                    status: "ready".into(),
                    error: None,
                    created_at: 1,
                }],
            )
            .await
            .unwrap();
        let mut run = RunRecord::new("run-1", "project-1", "local", "QC", "command");
        run.frame_id = Some("frame-1".into());
        run.script_path = Some(r"C:\Users\Alice\Study\analysis\qc.py".into());
        run.input_refs_json = r#"["C:\\Users\\Alice\\Study\\data\\counts.csv"]"#.into();
        run.output_specs_json =
            r#"[{"glob":"C:\\Users\\Alice\\Study\\results\\*.csv","kind":"table"}]"#.into();
        run.remote_workdir = Some("/home/alice/private-run".into());
        run.remote_handle_json =
            Some(r#"{"identity_file":"C:\\Users\\Alice\\.ssh\\id_ed25519","pid":42}"#.into());
        run.env_snapshot_json = r#"{"SSH_AUTH_SOCK":"/tmp/private-agent"}"#.into();
        source.create_run(&run).await.unwrap();
        source
            .update_run_status("run-1", RunStatus::Submitted)
            .await
            .unwrap();

        let stats = source
            .export_project_database("project-1", &archive_path)
            .await
            .unwrap();
        assert_eq!(stats.frames, 1);
        assert_eq!(stats.messages, 1);
        assert_eq!(stats.artifacts, 1);
        assert_eq!(stats.runs, 1);
        assert_eq!(stats.path_warnings, 0);

        let target = Store::open(&target_path).await.unwrap();
        let workspace = Path::new("/Users/alice/Study");
        target
            .import_project_database(&archive_path, "project-1", workspace)
            .await
            .unwrap();
        assert_eq!(
            target.get_project("project-1").await.unwrap().unwrap().1,
            "/Users/alice/Study"
        );
        assert_eq!(target.load_messages("frame-1").await.unwrap().len(), 1);
        assert_eq!(
            target.list_agent_workflows("project-1").await.unwrap(),
            vec![workflow]
        );
        assert_eq!(
            target
                .list_agent_workflow_steps("workflow-1")
                .await
                .unwrap(),
            vec![step]
        );
        let imported_resources = target
            .list_message_resource_links("frame-1", 1, None)
            .await
            .unwrap();
        assert_eq!(imported_resources.len(), 1);
        assert_eq!(
            imported_resources[0].artifact_version_id.as_deref(),
            Some(artifact_version_id.as_str())
        );
        assert_eq!(
            imported_resources[0].original_reference,
            "D:/original/location/plot.png"
        );
        assert_eq!(
            target.get_artifact("artifact-1").await.unwrap().unwrap().2,
            "/Users/alice/Study/.wisp/artifacts/sha256/ab/abcdef.png"
        );
        assert_eq!(
            target
                .get_artifact_version(&artifact_version_id)
                .await
                .unwrap()
                .unwrap()
                .storage_path,
            "/Users/alice/Study/.wisp/artifacts/sha256/ab/abcdef.png"
        );
        let imported_run = target.get_run("run-1").await.unwrap().unwrap();
        assert_eq!(imported_run.status, RunStatus::Lost);
        assert_eq!(
            imported_run.script_path.as_deref(),
            Some("/Users/alice/Study/analysis/qc.py")
        );
        assert_eq!(imported_run.input_refs_json, r#"["data/counts.csv"]"#);
        assert!(imported_run
            .output_specs_json
            .contains(r#""glob":"results/*.csv""#));
        assert!(imported_run.remote_workdir.is_none());
        assert!(imported_run.remote_handle_json.is_none());
        assert_eq!(imported_run.env_snapshot_json, "{}");
        assert!(target
            .import_project_database(&archive_path, "project-1", workspace)
            .await
            .is_err());

        source.pool.close().await;
        target.pool.close().await;
        for path in [source_path, archive_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn portable_hash_ignores_open_recency_but_detects_project_edits() {
        let token = uuid::Uuid::new_v4();
        let source_path = std::env::temp_dir().join(format!("wisp_hash_source_{token}.sqlite"));
        let first_path = std::env::temp_dir().join(format!("wisp_hash_first_{token}.sqlite"));
        let second_path = std::env::temp_dir().join(format!("wisp_hash_second_{token}.sqlite"));
        let edited_path = std::env::temp_dir().join(format!("wisp_hash_edited_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project-1", "Study", "/tmp/study")
            .await
            .unwrap();
        source
            .export_project_database("project-1", &first_path)
            .await
            .unwrap();
        sqlx::query("UPDATE projects SET updated_at=updated_at+100 WHERE id='project-1'")
            .execute(&source.pool)
            .await
            .unwrap();
        source
            .export_project_database("project-1", &second_path)
            .await
            .unwrap();
        assert_eq!(
            Store::portable_project_database_hash(&first_path)
                .await
                .unwrap(),
            Store::portable_project_database_hash(&second_path)
                .await
                .unwrap()
        );
        source
            .update_project("project-1", "Study", "changed")
            .await
            .unwrap();
        source
            .export_project_database("project-1", &edited_path)
            .await
            .unwrap();
        assert_ne!(
            Store::portable_project_database_hash(&first_path)
                .await
                .unwrap(),
            Store::portable_project_database_hash(&edited_path)
                .await
                .unwrap()
        );
        source.pool.close().await;
        for path in [source_path, first_path, second_path, edited_path] {
            let _ = std::fs::remove_file(path);
        }
    }
}
