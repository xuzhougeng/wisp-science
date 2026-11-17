-- Outcomes of automatic project-database recovery (#1635). A damaged
-- `.wisp/project.sqlite` is moved aside and replaced by its newest backup
-- copy, or by an empty database when no copy opens. The outcome is written to
-- the application database, never to the project database that was just
-- replaced, so the notice survives restarts until the user acknowledges it,
-- whichever entry point (explicit open, project list, sync) found the damage.
-- No foreign key: a project database's registry row is enough, and the
-- record must outlive a failed or partial project-row lookup.
CREATE TABLE IF NOT EXISTS project_database_recoveries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id      TEXT NOT NULL,
    occurred_at     INTEGER NOT NULL,
    outcome         TEXT NOT NULL CHECK (outcome IN ('restored', 'rebuilt')),
    copy_name       TEXT,
    damaged_path    TEXT NOT NULL,
    backup_dir      TEXT,
    acknowledged_at INTEGER
);
CREATE INDEX IF NOT EXISTS ix_project_database_recoveries_pending
    ON project_database_recoveries(project_id, acknowledged_at, occurred_at);
