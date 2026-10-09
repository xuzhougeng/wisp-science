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

-- One row per round: what it did and what it left for the next one. The
-- ledger, not the transcript, is what a later round is briefed from.
CREATE TABLE IF NOT EXISTS mandate_rounds (
    id          TEXT PRIMARY KEY,
    mandate_id  TEXT NOT NULL REFERENCES mandates(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    done        TEXT NOT NULL,
    kpis        TEXT NOT NULL DEFAULT '[]',
    blockers    TEXT NOT NULL DEFAULT '',
    next_step   TEXT NOT NULL DEFAULT '',
    next_run_at INTEGER,
    source      TEXT NOT NULL DEFAULT 'agent' CHECK(source IN ('agent','host')),
    created_at  INTEGER NOT NULL,
    UNIQUE(mandate_id, seq)
);
CREATE INDEX IF NOT EXISTS ix_mandate_rounds_time ON mandate_rounds(mandate_id, created_at);
