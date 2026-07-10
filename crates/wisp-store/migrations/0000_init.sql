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

CREATE TABLE IF NOT EXISTS folders (
    id         TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_folders_project ON folders(project_id);

CREATE TABLE IF NOT EXISTS frames (
    id              TEXT PRIMARY KEY,
    parent_frame_id TEXT REFERENCES frames(id) ON DELETE SET NULL,
    root_frame_id   TEXT REFERENCES frames(id) ON DELETE SET NULL,
    agent_name      TEXT NOT NULL,
    status          TEXT NOT NULL,
    project_id      TEXT REFERENCES projects(id) ON DELETE CASCADE,
    folder_id       TEXT REFERENCES folders(id) ON DELETE SET NULL,
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

CREATE TABLE IF NOT EXISTS execution_contexts (
    id                 TEXT PRIMARY KEY,
    kind               TEXT NOT NULL,
    label              TEXT NOT NULL,
    config_json        TEXT NOT NULL DEFAULT '{}',
    capabilities_json  TEXT NOT NULL DEFAULT '{}',
    last_probe_at      INTEGER,
    last_probe_status  TEXT,
    last_probe_error   TEXT,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_execution_contexts_kind ON execution_contexts(kind);

CREATE TABLE IF NOT EXISTS runs (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    frame_id           TEXT,
    context_id         TEXT NOT NULL,
    title              TEXT NOT NULL,
    kind               TEXT NOT NULL,
    status             TEXT NOT NULL,
    command            TEXT,
    script_path        TEXT,
    input_refs_json    TEXT NOT NULL DEFAULT '[]',
    output_specs_json  TEXT NOT NULL DEFAULT '[]',
    created_at         INTEGER NOT NULL,
    started_at         INTEGER,
    ended_at           INTEGER,
    exit_code          INTEGER,
    stdout_tail        TEXT,
    stderr_tail        TEXT,
    remote_workdir     TEXT,
    remote_handle_json TEXT,
    timeout_secs       INTEGER,
    last_polled_at     INTEGER,
    last_poll_error    TEXT,
    lifecycle_owner    TEXT,
    lifecycle_lease_until INTEGER,
    env_snapshot_json  TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS ix_runs_project ON runs(project_id, created_at);
CREATE INDEX IF NOT EXISTS ix_runs_context ON runs(context_id);

CREATE TABLE IF NOT EXISTS run_artifacts (
    id          TEXT PRIMARY KEY,
    run_id      TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    role        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_run_artifacts_run ON run_artifacts(run_id);

CREATE TABLE IF NOT EXISTS research_nodes (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,
    title         TEXT NOT NULL,
    ref_id        TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_research_nodes_project ON research_nodes(project_id, kind);

CREATE TABLE IF NOT EXISTS research_edges (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_id     TEXT NOT NULL REFERENCES research_nodes(id) ON DELETE CASCADE,
    target_id     TEXT NOT NULL REFERENCES research_nodes(id) ON DELETE CASCADE,
    relation      TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_research_edges_project ON research_edges(project_id, source_id, target_id);
