-- One rolling progress memory per project (#1589): goal, done, conclusions,
-- open questions, work in progress and next steps. Derived from the project's
-- records by the daily recap run and rebuilt from them, so it is neither
-- exported nor part of the project's state generation.
CREATE TABLE IF NOT EXISTS project_status (
    project_id  TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    status_json TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);
