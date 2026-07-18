CREATE TABLE IF NOT EXISTS agent_workflows (
    id           TEXT PRIMARY KEY,
    project_id   TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    version      INTEGER NOT NULL DEFAULT 1,
    enabled      INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ix_agent_workflows_project
    ON agent_workflows(project_id, name, id);

CREATE TABLE IF NOT EXISTS agent_workflow_steps (
    id                   TEXT PRIMARY KEY,
    workflow_id          TEXT NOT NULL REFERENCES agent_workflows(id) ON DELETE CASCADE,
    position             INTEGER NOT NULL CHECK(position >= 0),
    agent_id             TEXT NOT NULL,
    role                 TEXT NOT NULL,
    backend              TEXT NOT NULL,
    model                TEXT,
    prompt_template      TEXT NOT NULL,
    input_schema_json    TEXT NOT NULL DEFAULT '{}',
    output_schema_json   TEXT NOT NULL DEFAULT '{}',
    permissions_json     TEXT NOT NULL DEFAULT '{}',
    context_policy_json  TEXT NOT NULL DEFAULT '{}',
    timeout_secs         INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    UNIQUE(workflow_id, position)
);
CREATE INDEX IF NOT EXISTS ix_agent_workflow_steps_workflow
    ON agent_workflow_steps(workflow_id, position, id);
