CREATE TABLE IF NOT EXISTS assistant_tasks (
    id         TEXT PRIMARY KEY,
    day        TEXT NOT NULL,
    title      TEXT NOT NULL CHECK(length(trim(title)) > 0),
    project_id TEXT,
    session_id TEXT,
    status     TEXT NOT NULL CHECK(status IN ('open','done','dropped')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_assistant_tasks_day ON assistant_tasks(day, created_at);
