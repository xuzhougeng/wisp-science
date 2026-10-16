-- Private per-project memos. Researcher-only notes that never enter the
-- agent's context: no prompt assembly, tool, journey projection or export of
-- a conversation reads this table. A memo reaches the model only when the
-- researcher pastes it into the composer and sends it as an ordinary message.
CREATE TABLE IF NOT EXISTS project_memos (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL CHECK(length(trim(body)) > 0),
    pinned      INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_project_memos_project_updated
    ON project_memos(project_id, pinned DESC, updated_at DESC, id DESC);
