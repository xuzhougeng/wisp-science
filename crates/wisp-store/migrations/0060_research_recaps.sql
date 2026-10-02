CREATE TABLE IF NOT EXISTS research_recaps (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    day_start INTEGER NOT NULL,
    status TEXT NOT NULL,
    recap_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(project_id, day_start)
);
