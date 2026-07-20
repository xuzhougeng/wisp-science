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
CREATE INDEX IF NOT EXISTS ix_frames_project_created ON frames(project_id, created_at DESC, id DESC);
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

CREATE TABLE IF NOT EXISTS session_reviews (
    id          TEXT PRIMARY KEY,
    frame_id    TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    message_seq INTEGER NOT NULL,
    report_json TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_session_reviews_frame
    ON session_reviews(frame_id, message_seq);

CREATE TABLE IF NOT EXISTS session_ui_events (
    frame_id  TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    seq       INTEGER NOT NULL,
    event_json TEXT NOT NULL,
    PRIMARY KEY(frame_id, seq)
);

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

-- Structured resource references discovered when a new assistant message is
-- persisted. The transcript keeps the agent's original Markdown; rendering
-- uses these bindings and the immutable artifact version instead of guessing a
-- filesystem path from an href at click time.
CREATE TABLE IF NOT EXISTS message_resource_links (
    id                  TEXT PRIMARY KEY,
    frame_id            TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    message_seq         INTEGER NOT NULL,
    ordinal             INTEGER NOT NULL,
    original_reference  TEXT NOT NULL,
    artifact_id         TEXT REFERENCES artifacts(id) ON DELETE SET NULL,
    artifact_version_id TEXT REFERENCES artifact_versions(id) ON DELETE SET NULL,
    display_name        TEXT NOT NULL,
    resource_kind       TEXT NOT NULL,
    mime_type           TEXT NOT NULL,
    status              TEXT NOT NULL,
    error               TEXT,
    created_at          INTEGER NOT NULL,
    UNIQUE(frame_id, message_seq, ordinal)
);
CREATE INDEX IF NOT EXISTS ix_message_resource_links_message
    ON message_resource_links(frame_id, message_seq, ordinal);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Codex native Plan proposals are persisted independently from transcript
-- messages and from Wisp's built-in update_plan progress tool.  A frame may
-- have several immutable revisions while status/progress are updated in place.
CREATE TABLE IF NOT EXISTS proposed_plans (
    id                  TEXT PRIMARY KEY,
    frame_id            TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    codex_thread_id     TEXT,
    codex_turn_id       TEXT,
    revision            INTEGER NOT NULL,
    markdown            TEXT NOT NULL,
    status              TEXT NOT NULL,
    mode                TEXT NOT NULL DEFAULT 'native',
    progress_json       TEXT NOT NULL DEFAULT '[]',
    runtime_config_json TEXT NOT NULL DEFAULT '{}',
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    UNIQUE(frame_id, revision)
);
CREATE INDEX IF NOT EXISTS ix_proposed_plans_frame
    ON proposed_plans(frame_id, revision DESC);

-- Immutable-at-start configuration audit for local Codex turns.  `actual_json`
-- may be updated when Codex reports a model reroute; requested/effective stay
-- frozen so the UI can explain exactly what changed.
CREATE TABLE IF NOT EXISTS codex_turn_configs (
    id                  TEXT PRIMARY KEY,
    frame_id            TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    codex_thread_id     TEXT,
    codex_turn_id       TEXT,
    mode                TEXT NOT NULL,
    config_version      INTEGER NOT NULL DEFAULT 0,
    config_version_text TEXT NOT NULL DEFAULT '',
    requested_json      TEXT NOT NULL,
    effective_json      TEXT NOT NULL,
    actual_json         TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    UNIQUE(frame_id, codex_turn_id)
);
CREATE INDEX IF NOT EXISTS ix_codex_turn_configs_frame
    ON codex_turn_configs(frame_id, created_at DESC);

-- Durable binding between a Wisp frame and the session owned by an external
-- ACP agent. Agent credentials and private configuration are never stored here.
CREATE TABLE IF NOT EXISTS acp_sessions (
    frame_id            TEXT PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE,
    agent_profile_id    TEXT NOT NULL,
    profile_fingerprint TEXT NOT NULL,
    agent_session_id    TEXT NOT NULL,
    cwd                 TEXT NOT NULL,
    protocol_version    INTEGER NOT NULL,
    agent_info_json     TEXT NOT NULL DEFAULT '{}',
    capabilities_json   TEXT NOT NULL DEFAULT '{}',
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    UNIQUE(agent_profile_id, agent_session_id)
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

-- Remote compute resources selected for one conversation. Local execution is
-- deliberately absent because it is always available.
CREATE TABLE IF NOT EXISTS session_execution_contexts (
    frame_id   TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    context_id TEXT NOT NULL REFERENCES execution_contexts(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(frame_id, context_id)
);
CREATE INDEX IF NOT EXISTS ix_session_execution_contexts_context
    ON session_execution_contexts(context_id);

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
    progress_json      TEXT NOT NULL DEFAULT '{}',
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

-- Device-local cursor for explicit snapshot synchronization. The project data
-- itself is exported separately; this row is never included in a snapshot.
CREATE TABLE IF NOT EXISTS project_sync_state (
    project_id         TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    transport_kind     TEXT NOT NULL,
    transport_location TEXT NOT NULL,
    relay_project_id   TEXT NOT NULL,
    base_revision      TEXT,
    base_state_hash    TEXT,
    base_manifest_json TEXT NOT NULL DEFAULT '{"version":1,"files":[],"skipped_paths":[]}',
    last_synced_at     INTEGER,
    last_direction     TEXT
);
