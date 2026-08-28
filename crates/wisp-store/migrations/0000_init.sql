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
    updated_at    INTEGER NOT NULL,
    run_retention_days        INTEGER,
    failed_run_retention_days INTEGER,
    orphan_file_retention_days INTEGER
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
    exploration_id  TEXT,
    folder_id       TEXT REFERENCES folders(id) ON DELETE SET NULL,
    branched_from   TEXT,
    branch_point_user_index INTEGER,
    branch_point_kind TEXT,
    pinned          INTEGER NOT NULL DEFAULT 0,
    model           TEXT,
    reasoning_effort TEXT,
    service_tier    TEXT,
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

CREATE TABLE IF NOT EXISTS session_branch_merges (
    id                      TEXT PRIMARY KEY,
    source_frame_id         TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    branch_frame_id         TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    checkpoint_user_index   INTEGER NOT NULL,
    checkpoint_kind         TEXT NOT NULL,
    summary_message_seq     INTEGER NOT NULL,
    guard_hash              TEXT NOT NULL,
    created_at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_session_branch_merges_source
    ON session_branch_merges(source_frame_id, created_at DESC);

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
    created_at      INTEGER NOT NULL,
    latest_version_id TEXT,
    logical_key     TEXT,
    exploration_id  TEXT
);
CREATE INDEX IF NOT EXISTS ix_artifacts_project ON artifacts(project_id);
CREATE INDEX IF NOT EXISTS ix_artifacts_project_logical_key
    ON artifacts(project_id, logical_key) WHERE logical_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS artifact_versions (
    id                  TEXT PRIMARY KEY,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    version_number      INTEGER NOT NULL,
    content_type        TEXT NOT NULL,
    storage_path        TEXT NOT NULL,
    size_bytes          INTEGER,
    checksum            TEXT,
    parent_version_id   TEXT REFERENCES artifact_versions(id) ON DELETE SET NULL,
    producing_run_id    TEXT REFERENCES runs(id) ON DELETE SET NULL,
    env_snapshot_hash   TEXT REFERENCES env_snapshots(hash) ON DELETE SET NULL,
    materialization     TEXT NOT NULL DEFAULT 'reference',
    capture_timing      TEXT NOT NULL DEFAULT 'unknown',
    created_at          INTEGER NOT NULL,
    source_discarded_at INTEGER,
    UNIQUE(artifact_id, version_number)
);
CREATE INDEX IF NOT EXISTS ix_artifact_versions_artifact
    ON artifact_versions(artifact_id, version_number DESC);
CREATE INDEX IF NOT EXISTS ix_artifact_versions_run
    ON artifact_versions(producing_run_id);

CREATE TABLE IF NOT EXISTS artifact_dependencies (
    id                    TEXT PRIMARY KEY,
    artifact_version_id   TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE,
    depends_on_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE,
    reference_name        TEXT,
    basis                 TEXT NOT NULL DEFAULT 'inferred',
    confidence            TEXT NOT NULL DEFAULT 'uncertain',
    created_at            INTEGER NOT NULL,
    UNIQUE(artifact_version_id, depends_on_version_id)
);
CREATE INDEX IF NOT EXISTS ix_artifact_dependencies_version
    ON artifact_dependencies(artifact_version_id);

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
    created_artifact    INTEGER NOT NULL DEFAULT 0,
    created_version     INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    UNIQUE(frame_id, message_seq, ordinal)
);
CREATE INDEX IF NOT EXISTS ix_message_resource_links_message
    ON message_resource_links(frame_id, message_seq, ordinal);

-- Bounded text-file preimages for undoing the latest completed agent turn.
-- Snapshot bytes live under the project's .wisp/undo directory; SQLite keeps
-- only lineage and checksums.
CREATE TABLE IF NOT EXISTS turn_file_undo (
    frame_id             TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    user_message_seq     INTEGER NOT NULL,
    path                 TEXT NOT NULL,
    before_exists        INTEGER NOT NULL,
    before_snapshot_path TEXT,
    before_checksum      TEXT,
    after_checksum       TEXT,
    reversible           INTEGER NOT NULL,
    reason               TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    PRIMARY KEY(frame_id, user_message_seq, path)
);
CREATE INDEX IF NOT EXISTS ix_turn_file_undo_turn
    ON turn_file_undo(frame_id, user_message_seq);

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

-- External coding-agent transcripts imported as Wisp sessions (#464). The
-- legacy table/key names remain for compatibility; non-Codex ids are
-- provider-prefixed. Deleting the Wisp session frees the id for import again.
CREATE TABLE IF NOT EXISTS codex_imports (
    codex_session_id TEXT PRIMARY KEY,
    frame_id         TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    source_path      TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_codex_imports_frame ON codex_imports(frame_id);

-- Metadata-only cache for external CLI session discovery. File stamps let
-- local/WSL/SSH scans avoid rereading unchanged JSONL transcripts.
CREATE TABLE IF NOT EXISTS external_session_cache (
    source_id            TEXT NOT NULL,
    provider             TEXT NOT NULL,
    source_path          TEXT NOT NULL,
    file_size            INTEGER NOT NULL,
    modified_at_ms       INTEGER NOT NULL,
    session_id           TEXT NOT NULL,
    title                TEXT NOT NULL,
    cwd                  TEXT NOT NULL,
    message_count        INTEGER NOT NULL,
    created_at_ms        INTEGER NOT NULL,
    last_active_at_ms    INTEGER NOT NULL,
    changed_since_import INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(source_id, provider, source_path)
);
CREATE INDEX IF NOT EXISTS ix_external_session_cache_source
    ON external_session_cache(source_id, provider, last_active_at_ms DESC);

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

CREATE TABLE IF NOT EXISTS context_storage_prefs (
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    context_id          TEXT NOT NULL,
    remote_data_root    TEXT NOT NULL,
    remote_workdir_root TEXT NOT NULL,
    local_results_dir   TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY (project_id, context_id)
);

CREATE TABLE IF NOT EXISTS remote_staging (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    context_id  TEXT NOT NULL,
    run_id      TEXT,
    remote_path TEXT NOT NULL,
    source      TEXT NOT NULL,
    checksum    TEXT,
    size_bytes  INTEGER,
    created_at  INTEGER NOT NULL,
    removed_at  INTEGER
);
CREATE INDEX IF NOT EXISTS ix_remote_staging_ctx ON remote_staging(context_id, removed_at);

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
    env_snapshot_json  TEXT NOT NULL DEFAULT '{}',
    exploration_id     TEXT,
    harvested_at       INTEGER,
    cleaned_at         INTEGER,
    cleanup_error      TEXT,
    logs_path          TEXT
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

CREATE TABLE IF NOT EXISTS external_resources (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL,
    uri                 TEXT NOT NULL,
    version             TEXT,
    checksum            TEXT,
    size_bytes          INTEGER,
    license             TEXT,
    visibility          TEXT NOT NULL DEFAULT 'restricted',
    access_instructions TEXT,
    accessed_at         INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    exploration_id      TEXT,
    UNIQUE(project_id, uri, version)
);

CREATE TABLE IF NOT EXISTS run_inputs (
    id                   TEXT PRIMARY KEY,
    run_id               TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    artifact_version_id  TEXT REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    external_resource_id TEXT REFERENCES external_resources(id) ON DELETE RESTRICT,
    source_ref           TEXT NOT NULL,
    role                 TEXT NOT NULL,
    required             INTEGER NOT NULL DEFAULT 1,
    basis                TEXT NOT NULL,
    confidence           TEXT NOT NULL,
    created_at           INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_run_inputs_run ON run_inputs(run_id);
CREATE INDEX IF NOT EXISTS ix_run_inputs_artifact_version
    ON run_inputs(artifact_version_id);

CREATE TABLE IF NOT EXISTS run_outputs (
    id                  TEXT PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    role                TEXT NOT NULL,
    logical_output_key  TEXT NOT NULL,
    source_path         TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    UNIQUE(run_id, artifact_version_id, role)
);
CREATE INDEX IF NOT EXISTS ix_run_outputs_run ON run_outputs(run_id);
CREATE INDEX IF NOT EXISTS ix_run_outputs_artifact_version
    ON run_outputs(artifact_version_id);

CREATE TABLE IF NOT EXISTS run_code_snapshots (
    id           TEXT PRIMARY KEY,
    run_id       TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    source_kind  TEXT NOT NULL,
    source_path  TEXT,
    source_text  TEXT NOT NULL,
    checksum     TEXT NOT NULL,
    storage_path TEXT,
    git_commit   TEXT,
    dirty_patch  TEXT,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_run_code_snapshots_run
    ON run_code_snapshots(run_id);

CREATE TABLE IF NOT EXISTS run_environment_snapshots (
    run_id            TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    env_snapshot_hash TEXT NOT NULL REFERENCES env_snapshots(hash) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS publications (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_publications_project
    ON publications(project_id, updated_at DESC, id);

CREATE TABLE IF NOT EXISTS publication_revisions (
    id                  TEXT PRIMARY KEY,
    publication_id      TEXT NOT NULL REFERENCES publications(id) ON DELETE CASCADE,
    parent_revision_id  TEXT REFERENCES publication_revisions(id) ON DELETE SET NULL,
    revision_number     INTEGER NOT NULL CHECK(revision_number > 0),
    label               TEXT NOT NULL,
    state               TEXT NOT NULL DEFAULT 'draft'
                            CHECK(state IN ('draft','freezing','frozen','published','deleting')),
    capability_level    TEXT NOT NULL DEFAULT 'archived'
                            CHECK(capability_level IN
                                  ('archived','traceable','re_executable','reproduced')),
    manifest_json       TEXT,
    manifest_sha256     TEXT,
    frozen_at           INTEGER,
    published_at        INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    CHECK(state NOT IN ('frozen','published')
          OR (manifest_json IS NOT NULL AND length(manifest_sha256)=64 AND frozen_at IS NOT NULL)),
    CHECK(state <> 'published' OR published_at IS NOT NULL),
    UNIQUE(publication_id, revision_number)
);
CREATE INDEX IF NOT EXISTS ix_publication_revisions_publication
    ON publication_revisions(publication_id, revision_number DESC);

CREATE TABLE IF NOT EXISTS publication_items (
    id             TEXT PRIMARY KEY,
    revision_id    TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    parent_item_id TEXT REFERENCES publication_items(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL
                       CHECK(kind IN ('section','claim','figure','table','methods','supplement')),
    title          TEXT NOT NULL,
    content        TEXT NOT NULL DEFAULT '',
    ordinal        INTEGER NOT NULL,
    metadata_json  TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_publication_items_revision_order
    ON publication_items(revision_id, parent_item_id, ordinal, id);
CREATE UNIQUE INDEX IF NOT EXISTS ux_publication_items_sibling_order
    ON publication_items(revision_id, COALESCE(parent_item_id,''), ordinal);

CREATE TABLE IF NOT EXISTS publication_item_links (
    id             TEXT PRIMARY KEY,
    revision_id    TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    source_item_id TEXT NOT NULL REFERENCES publication_items(id) ON DELETE CASCADE,
    target_item_id TEXT NOT NULL REFERENCES publication_items(id) ON DELETE CASCADE,
    relation       TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    CHECK(source_item_id <> target_item_id),
    UNIQUE(revision_id, source_item_id, target_item_id, relation)
);
CREATE INDEX IF NOT EXISTS ix_publication_item_links_revision
    ON publication_item_links(revision_id, source_item_id, target_item_id);

CREATE TABLE IF NOT EXISTS evidence_bindings (
    id                      TEXT PRIMARY KEY,
    revision_id             TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    item_id                 TEXT REFERENCES publication_items(id) ON DELETE CASCADE,
    source_kind             TEXT NOT NULL
                                CHECK(source_kind IN
                                      ('artifact_version','run','execution_log','message_span',
                                       'tool_call','code_cell','external_resource')),
    source_id               TEXT NOT NULL,
    artifact_version_id     TEXT REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    run_id                  TEXT REFERENCES runs(id) ON DELETE RESTRICT,
    external_resource_id    TEXT REFERENCES external_resources(id) ON DELETE RESTRICT,
    purpose                 TEXT NOT NULL DEFAULT '',
    supported_claim_item_id TEXT REFERENCES publication_items(id) ON DELETE SET NULL,
    selection_state         TEXT NOT NULL DEFAULT 'candidate'
                                CHECK(selection_state IN ('candidate','selected','rejected')),
    review_state            TEXT NOT NULL DEFAULT 'unreviewed'
                                CHECK(review_state IN ('unreviewed','reviewed')),
    reproduction_state      TEXT NOT NULL DEFAULT 'not_run'
                                CHECK(reproduction_state IN
                                      ('not_run','passed','failed','not_applicable')),
    visibility              TEXT NOT NULL DEFAULT 'private'
                                CHECK(visibility IN ('public','restricted','private')),
    source_snapshot_json    TEXT NOT NULL,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    CHECK(
        (source_kind='artifact_version'
         AND artifact_version_id IS NOT NULL AND source_id=artifact_version_id
         AND run_id IS NULL AND external_resource_id IS NULL)
        OR
        (source_kind='run'
         AND run_id IS NOT NULL AND source_id=run_id
         AND artifact_version_id IS NULL AND external_resource_id IS NULL)
        OR
        (source_kind='external_resource'
         AND external_resource_id IS NOT NULL AND source_id=external_resource_id
         AND artifact_version_id IS NULL AND run_id IS NULL)
        OR
        (source_kind IN ('execution_log','message_span','tool_call','code_cell')
         AND artifact_version_id IS NULL AND run_id IS NULL AND external_resource_id IS NULL)
    )
);
CREATE INDEX IF NOT EXISTS ix_evidence_bindings_revision
    ON evidence_bindings(revision_id, item_id, selection_state, id);
CREATE INDEX IF NOT EXISTS ix_evidence_bindings_artifact_version
    ON evidence_bindings(artifact_version_id);
CREATE INDEX IF NOT EXISTS ix_evidence_bindings_run
    ON evidence_bindings(run_id);

CREATE TABLE IF NOT EXISTS evidence_reviews (
    id               TEXT PRIMARY KEY,
    binding_id       TEXT NOT NULL REFERENCES evidence_bindings(id) ON DELETE CASCADE,
    reviewer         TEXT NOT NULL,
    method           TEXT NOT NULL,
    verified_at      INTEGER NOT NULL,
    environment_json TEXT NOT NULL DEFAULT '{}',
    comparator_json  TEXT NOT NULL DEFAULT '{}',
    tolerance_json   TEXT NOT NULL DEFAULT '{}',
    result           TEXT NOT NULL,
    report_json      TEXT NOT NULL DEFAULT '{}',
    created_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_evidence_reviews_binding
    ON evidence_reviews(binding_id, verified_at, id);

CREATE TABLE IF NOT EXISTS evidence_supersessions (
    id             TEXT PRIMARY KEY,
    revision_id    TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    old_binding_id TEXT NOT NULL REFERENCES evidence_bindings(id) ON DELETE CASCADE,
    new_binding_id TEXT NOT NULL REFERENCES evidence_bindings(id) ON DELETE CASCADE,
    reason         TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL,
    CHECK(old_binding_id <> new_binding_id),
    UNIQUE(revision_id, old_binding_id)
);
CREATE INDEX IF NOT EXISTS ix_evidence_supersessions_revision
    ON evidence_supersessions(revision_id, old_binding_id);

CREATE TABLE IF NOT EXISTS publication_readiness_reports (
    id                TEXT PRIMARY KEY,
    revision_id       TEXT NOT NULL UNIQUE
                          REFERENCES publication_revisions(id) ON DELETE CASCADE,
    capability_level  TEXT NOT NULL
                          CHECK(capability_level IN
                                ('archived','traceable','re_executable','reproduced')),
    target_visibility TEXT NOT NULL DEFAULT 'public'
                          CHECK(target_visibility IN ('public','restricted','private')),
    policy_json       TEXT NOT NULL DEFAULT '{}',
    blockers_json     TEXT NOT NULL DEFAULT '[]',
    warnings_json     TEXT NOT NULL DEFAULT '[]',
    omissions_json    TEXT NOT NULL DEFAULT '[]',
    manifest_json     TEXT NOT NULL,
    manifest_sha256   TEXT NOT NULL,
    created_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS publication_waivers (
    id           TEXT PRIMARY KEY,
    revision_id  TEXT NOT NULL REFERENCES publication_revisions(id) ON DELETE CASCADE,
    finding_code TEXT NOT NULL,
    author       TEXT NOT NULL,
    reason       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    UNIQUE(revision_id, finding_code)
);
CREATE INDEX IF NOT EXISTS ix_publication_waivers_revision
    ON publication_waivers(revision_id, finding_code);

CREATE TABLE IF NOT EXISTS publication_freeze_attempts (
    id                TEXT PRIMARY KEY,
    revision_id       TEXT NOT NULL UNIQUE
                          REFERENCES publication_revisions(id) ON DELETE CASCADE,
    target_visibility TEXT NOT NULL
                          CHECK(target_visibility IN ('public','restricted','private')),
    policy_json       TEXT NOT NULL,
    started_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_publication_freeze_attempts_started
    ON publication_freeze_attempts(started_at, revision_id);

CREATE TABLE IF NOT EXISTS capsule_builds (
    id                       TEXT PRIMARY KEY,
    revision_id              TEXT NOT NULL
                                 REFERENCES publication_revisions(id) ON DELETE CASCADE,
    format                   TEXT NOT NULL,
    visibility               TEXT NOT NULL
                                 CHECK(visibility IN ('public','restricted','private')),
    status                   TEXT NOT NULL,
    output_path              TEXT,
    revision_manifest_sha256 TEXT NOT NULL,
    archive_sha256           TEXT,
    error                    TEXT,
    created_at               INTEGER NOT NULL,
    completed_at             INTEGER
);
CREATE INDEX IF NOT EXISTS ix_capsule_builds_revision
    ON capsule_builds(revision_id, created_at DESC, id);

CREATE TABLE IF NOT EXISTS reproduction_runs (
    id                        TEXT PRIMARY KEY,
    revision_id               TEXT NOT NULL
                                      REFERENCES publication_revisions(id) ON DELETE CASCADE,
    source_run_id             TEXT NOT NULL REFERENCES runs(id) ON DELETE RESTRICT,
    status                    TEXT NOT NULL
                                      CHECK(status IN ('running','completed','failed')),
    capability_level          TEXT NOT NULL
                                      CHECK(capability_level IN ('re_executable','reproduced')),
    command_sha256            TEXT NOT NULL,
    expected_environment_hash TEXT,
    actual_environment_json   TEXT NOT NULL,
    actual_environment_hash   TEXT NOT NULL,
    environment_matched       INTEGER NOT NULL DEFAULT 0,
    workspace_manifest_json   TEXT NOT NULL,
    stdout_tail               TEXT,
    stderr_tail               TEXT,
    exit_code                 INTEGER,
    error                     TEXT,
    created_at                INTEGER NOT NULL,
    started_at                INTEGER NOT NULL,
    completed_at              INTEGER
);
CREATE INDEX IF NOT EXISTS ix_reproduction_runs_revision
    ON reproduction_runs(revision_id, created_at DESC, id);
CREATE INDEX IF NOT EXISTS ix_reproduction_runs_source
    ON reproduction_runs(source_run_id, created_at DESC, id);

CREATE TABLE IF NOT EXISTS reproduction_results (
    id                           TEXT PRIMARY KEY,
    reproduction_run_id          TEXT NOT NULL
                                         REFERENCES reproduction_runs(id) ON DELETE CASCADE,
    output_id                    TEXT NOT NULL,
    output_path                  TEXT NOT NULL,
    expected_artifact_version_id TEXT NOT NULL
                                         REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    comparator_kind              TEXT NOT NULL
                                         CHECK(comparator_kind IN
                                               ('sha256','text','json','numeric')),
    required                     INTEGER NOT NULL DEFAULT 1,
    expected_json                TEXT NOT NULL,
    actual_json                  TEXT NOT NULL,
    tolerance_json               TEXT NOT NULL DEFAULT '{}',
    passed                       INTEGER NOT NULL,
    report_json                  TEXT NOT NULL DEFAULT '{}',
    created_at                   INTEGER NOT NULL,
    UNIQUE(reproduction_run_id, output_id)
);
CREATE INDEX IF NOT EXISTS ix_reproduction_results_run
    ON reproduction_results(reproduction_run_id, output_id);

CREATE TABLE IF NOT EXISTS research_nodes (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL,
    title         TEXT NOT NULL,
    ref_id        TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    exploration_id TEXT,
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
    exploration_id TEXT,
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_research_edges_project ON research_edges(project_id, source_id, target_id);

-- Immutable project-state checkpoints and private exploration overlays.  A
-- normal conversation has frames.exploration_id=NULL; exploration-owned rows
-- stay hidden until a later promotion transaction adopts them.
CREATE TABLE IF NOT EXISTS project_state_counters (
    project_id          TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    mainline_generation INTEGER NOT NULL DEFAULT 0,
    updated_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_snapshots (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    manifest_json   TEXT NOT NULL,
    manifest_sha256 TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_workspace_snapshots_project
    ON workspace_snapshots(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS context_archives (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    frame_id     TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    storage_path TEXT NOT NULL,
    checksum     TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_context_archives_frame
    ON context_archives(frame_id, created_at DESC);

CREATE TABLE IF NOT EXISTS project_state_revisions (
    id                        TEXT PRIMARY KEY,
    project_id                TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    frame_id                  TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    turn_index                INTEGER NOT NULL CHECK(turn_index >= 0),
    message_seq               INTEGER NOT NULL CHECK(message_seq > 0),
    ui_event_seq              INTEGER NOT NULL CHECK(ui_event_seq >= 0),
    parent_revision_id        TEXT REFERENCES project_state_revisions(id) ON DELETE CASCADE,
    workspace_snapshot_id     TEXT NOT NULL REFERENCES workspace_snapshots(id) ON DELETE CASCADE,
    workspace_manifest_sha256 TEXT NOT NULL,
    workspace_delta_json      TEXT NOT NULL DEFAULT '[]',
    artifact_heads_json       TEXT NOT NULL DEFAULT '[]',
    entities_json             TEXT NOT NULL DEFAULT '[]',
    run_ids_json              TEXT NOT NULL DEFAULT '[]',
    decision_ids_json         TEXT NOT NULL DEFAULT '[]',
    external_effects_json     TEXT NOT NULL DEFAULT '[]',
    context_archive_id        TEXT NOT NULL REFERENCES context_archives(id) ON DELETE CASCADE,
    state_generation          INTEGER NOT NULL,
    is_full                   INTEGER NOT NULL DEFAULT 0 CHECK(is_full IN (0,1)),
    created_at                INTEGER NOT NULL,
    UNIQUE(frame_id, turn_index)
);
CREATE INDEX IF NOT EXISTS ix_project_state_revisions_project_created
    ON project_state_revisions(project_id, created_at, id);
CREATE INDEX IF NOT EXISTS ix_project_state_revisions_frame_turn
    ON project_state_revisions(frame_id, turn_index);

CREATE TABLE IF NOT EXISTS global_memories (
    id                TEXT PRIMARY KEY,
    content           TEXT NOT NULL CHECK(length(trim(content)) > 0),
    source_frame_id   TEXT REFERENCES frames(id) ON DELETE SET NULL,
    source_turn_index INTEGER CHECK(source_turn_index IS NULL OR source_turn_index >= 0),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_global_memories_updated
    ON global_memories(updated_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS exploration_families (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    root_frame_id     TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    mainline_frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    generation        INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE(project_id, root_frame_id)
);
CREATE INDEX IF NOT EXISTS ix_exploration_families_mainline
    ON exploration_families(project_id, mainline_frame_id);

CREATE TABLE IF NOT EXISTS exploration_checkpoints (
    id                      TEXT PRIMARY KEY,
    family_id               TEXT NOT NULL REFERENCES exploration_families(id) ON DELETE CASCADE,
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_frame_id         TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    source_message_seq      INTEGER NOT NULL,
    source_frame_head_seq   INTEGER NOT NULL,
    source_ui_event_seq      INTEGER NOT NULL,
    source_family_generation INTEGER NOT NULL,
    source_state_generation  INTEGER NOT NULL,
    workspace_snapshot_id   TEXT NOT NULL REFERENCES workspace_snapshots(id) ON DELETE RESTRICT,
    context_archive_id      TEXT NOT NULL REFERENCES context_archives(id) ON DELETE RESTRICT,
    guard_hash              TEXT NOT NULL,
    entity_hash             TEXT NOT NULL,
    isolation_summary_json  TEXT NOT NULL DEFAULT '{}',
    created_at              INTEGER NOT NULL,
    UNIQUE(family_id, source_frame_id, source_message_seq, guard_hash)
);
CREATE INDEX IF NOT EXISTS ix_exploration_checkpoints_source
    ON exploration_checkpoints(source_frame_id, source_message_seq);

CREATE TABLE IF NOT EXISTS explorations (
    id               TEXT PRIMARY KEY,
    checkpoint_id    TEXT NOT NULL REFERENCES exploration_checkpoints(id) ON DELETE CASCADE,
    frame_id          TEXT NOT NULL UNIQUE REFERENCES frames(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    status            TEXT NOT NULL
                          CHECK(status IN ('creating','active','promoting','failed')),
    workspace_dir     TEXT NOT NULL,
    workspace_backend TEXT NOT NULL,
    scope_generation  INTEGER NOT NULL DEFAULT 0,
    warnings_json     TEXT NOT NULL DEFAULT '[]',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_explorations_checkpoint_status
    ON explorations(checkpoint_id, status, created_at);

CREATE TABLE IF NOT EXISTS exploration_baseline_entities (
    checkpoint_id TEXT NOT NULL REFERENCES exploration_checkpoints(id) ON DELETE CASCADE,
    entity_kind   TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    version_id    TEXT,
    fingerprint   TEXT NOT NULL,
    PRIMARY KEY(checkpoint_id, entity_kind, entity_id)
);

CREATE TABLE IF NOT EXISTS exploration_baseline_artifact_heads (
    checkpoint_id       TEXT NOT NULL REFERENCES exploration_checkpoints(id) ON DELETE CASCADE,
    logical_key         TEXT NOT NULL,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE RESTRICT,
    fingerprint         TEXT NOT NULL,
    PRIMARY KEY(checkpoint_id, logical_key)
);

CREATE TABLE IF NOT EXISTS artifact_heads (
    project_id          TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scope_key           TEXT NOT NULL,
    logical_key         TEXT NOT NULL,
    artifact_id         TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    artifact_version_id TEXT NOT NULL REFERENCES artifact_versions(id) ON DELETE CASCADE,
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY(project_id, scope_key, logical_key)
);
CREATE INDEX IF NOT EXISTS ix_artifact_heads_artifact
    ON artifact_heads(artifact_id, artifact_version_id);

CREATE TABLE IF NOT EXISTS exploration_effects (
    id             TEXT PRIMARY KEY,
    exploration_id TEXT NOT NULL REFERENCES explorations(id) ON DELETE CASCADE,
    effect_kind    TEXT NOT NULL,
    recoverability TEXT NOT NULL,
    target_summary TEXT NOT NULL,
    metadata_json  TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_exploration_effects_exploration
    ON exploration_effects(exploration_id, created_at);

CREATE TABLE IF NOT EXISTS exploration_promotions (
    id                  TEXT PRIMARY KEY,
    exploration_id      TEXT NOT NULL,
    expected_guard_hash TEXT NOT NULL,
    status              TEXT NOT NULL,
    diff_json           TEXT NOT NULL,
    journal_path        TEXT,
    error               TEXT,
    started_at          INTEGER NOT NULL,
    committed_at        INTEGER
);
CREATE INDEX IF NOT EXISTS ix_exploration_promotions_exploration
    ON exploration_promotions(exploration_id, started_at DESC);

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
