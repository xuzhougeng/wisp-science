-- Research mandates: a long-running responsibility an agent carries round by
-- round. `frame_id` stays a plain column so a mandate survives its
-- conversation being deleted; the next round opens a fresh one.
CREATE TABLE IF NOT EXISTS mandates (
    id                   TEXT PRIMARY KEY,
    project_id           TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    frame_id             TEXT,
    name                 TEXT NOT NULL CHECK(length(trim(name)) > 0),
    goal                 TEXT NOT NULL CHECK(length(trim(goal)) > 0),
    kpis                 TEXT NOT NULL DEFAULT '[]',
    constraints          TEXT NOT NULL DEFAULT '{}',
    ends_at              INTEGER,
    interval_secs        INTEGER NOT NULL,
    report_interval_secs INTEGER NOT NULL,
    next_report_at       INTEGER,
    status               TEXT NOT NULL CHECK(status IN ('active','paused','waiting','done')),
    next_run_at          INTEGER NOT NULL,
    last_run_at          INTEGER,
    wait_run_id          TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_mandates_due ON mandates(status, next_run_at);
CREATE INDEX IF NOT EXISTS ix_mandates_frame ON mandates(frame_id);
