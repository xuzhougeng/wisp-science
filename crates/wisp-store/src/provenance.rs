use super::{ExecLog, Store};
use anyhow::Result;
use sqlx::Row;

impl Store {
    /// Next `cell_index` for a frame = count of existing rows.
    pub async fn next_cell_index(&self, frame_id: &str) -> Result<i64> {
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM execution_log WHERE frame_id=?")
            .bind(frame_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(n.0)
    }

    pub async fn insert_execution_log(&self, e: &ExecLog) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let fw = serde_json::to_string(&e.files_written).unwrap_or_else(|_| "[]".into());
        let fr = serde_json::to_string(&e.files_read).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO execution_log(id,frame_id,cell_index,tool,language,source,stdout,stderr,\
             exit_status,wall_s,files_written,files_read,env_hash,created_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&e.id)
        .bind(&e.frame_id)
        .bind(e.cell_index)
        .bind(&e.tool)
        .bind(&e.language)
        .bind(&e.source)
        .bind(&e.stdout)
        .bind(&e.stderr)
        .bind(&e.exit_status)
        .bind(e.wall_s)
        .bind(&fw)
        .bind(&fr)
        .bind(&e.env_hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_env_snapshot(
        &self,
        hash: &str,
        env_name: Option<&str>,
        packages_json: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT OR IGNORE INTO env_snapshots(hash,env_name,packages_json,created_at) VALUES(?,?,?,?)",
        )
        .bind(hash).bind(env_name).bind(packages_json).bind(now)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_env_snapshot(&self, hash: &str) -> Result<Option<(Option<String>, String)>> {
        let row: Option<(Option<String>, String)> =
            sqlx::query_as("SELECT env_name, packages_json FROM env_snapshots WHERE hash=?")
                .bind(hash)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    /// Most-recent execution_log row in `frame_id` whose files_written contains `path`.
    pub async fn find_provenance_by_path(
        &self,
        frame_id: &str,
        path: &str,
    ) -> Result<Option<ExecLog>> {
        // Substring prefilter pushed into SQL so we don't fetch and JSON-parse
        // every row in the frame. The needle is the path's JSON encoding
        // (quotes included) because that's the byte form stored in the column —
        // matching the raw path would miss e.g. backslashes stored as `\\`.
        // The exact match below drops false positives (`a.csv` in `data.csv`,
        // LIKE's ASCII case folding), so the prefilter only ever over-selects.
        let needle = serde_json::to_string(path).unwrap_or_default();
        let escaped = needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let rows = sqlx::query(
            "SELECT id,frame_id,cell_index,tool,language,source,stdout,stderr,exit_status,\
             wall_s,files_written,files_read,env_hash FROM execution_log \
             WHERE frame_id=? AND files_written LIKE '%' || ? || '%' ESCAPE '\\' \
             ORDER BY created_at DESC, cell_index DESC",
        )
        .bind(frame_id)
        .bind(&escaped)
        .fetch_all(&self.pool)
        .await?;
        for r in rows {
            let fw: String = r.try_get("files_written")?;
            let written: Vec<String> = serde_json::from_str(&fw).unwrap_or_default();
            if written.iter().any(|p| p == path) {
                let fr: String = r.try_get("files_read")?;
                return Ok(Some(ExecLog {
                    id: r.try_get("id")?,
                    frame_id: r.try_get("frame_id")?,
                    cell_index: r.try_get("cell_index")?,
                    tool: r.try_get("tool")?,
                    language: r.try_get("language")?,
                    source: r.try_get("source")?,
                    stdout: r.try_get("stdout").unwrap_or_default(),
                    stderr: r.try_get("stderr").unwrap_or_default(),
                    exit_status: r.try_get("exit_status")?,
                    wall_s: r.try_get("wall_s").ok(),
                    files_written: written,
                    files_read: serde_json::from_str(&fr).unwrap_or_default(),
                    env_hash: r.try_get("env_hash").ok(),
                }));
            }
        }
        Ok(None)
    }

    /// Union of every path written by any cell in the frame (marks linkable inputs).
    pub async fn frame_written_paths(
        &self,
        frame_id: &str,
    ) -> Result<std::collections::HashSet<String>> {
        let rows = sqlx::query("SELECT files_written FROM execution_log WHERE frame_id=?")
            .bind(frame_id)
            .fetch_all(&self.pool)
            .await?;
        let mut set = std::collections::HashSet::new();
        for r in rows {
            let fw: String = r.try_get("files_written")?;
            if let Ok(v) = serde_json::from_str::<Vec<String>>(&fw) {
                set.extend(v);
            }
        }
        Ok(set)
    }
}
