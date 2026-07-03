-- Wisp initial schema (MVP subset of the upstream drizzle model).
-- projects: a workspace; frames: a conversation root/branch; messages:
-- serialized agent turns; artifacts: saved files; settings: kv config;
-- api_keys are kept in the OS keyring, not here.

CREATE TABLE IF NOT EXISTS projects (
    id            TEXT PRIMARY KEY,
    name          TEXT,
    description   TEXT,
    workspace_dir TEXT NOT NULL DEFAULT '',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS frames (
    id              TEXT PRIMARY KEY,
    parent_frame_id TEXT REFERENCES frames(id) ON DELETE SET NULL,
    root_frame_id   TEXT REFERENCES frames(id) ON DELETE SET NULL,
    agent_name      TEXT NOT NULL,
    status          TEXT NOT NULL,
    project_id      TEXT REFERENCES projects(id) ON DELETE CASCADE,
    model           TEXT,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    completed_at    INTEGER
);
CREATE INDEX IF NOT EXISTS ix_frames_project_id ON frames(project_id);
CREATE INDEX IF NOT EXISTS ix_frames_root ON frames(root_frame_id);

CREATE TABLE IF NOT EXISTS messages (
    id          TEXT PRIMARY KEY,
    frame_id    TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    role        TEXT NOT NULL,
    content     TEXT,
    tool_calls  TEXT,
    tool_call_id TEXT,
    tool_name   TEXT,
    reasoning   TEXT,
    ts          INTEGER NOT NULL,
    UNIQUE(frame_id, seq)
);
CREATE INDEX IF NOT EXISTS ix_messages_frame ON messages(frame_id);

CREATE TABLE IF NOT EXISTS artifacts (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    root_frame_id   TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    filename        TEXT NOT NULL,
    content_type    TEXT NOT NULL,
    storage_path    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_artifacts_project ON artifacts(project_id);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
