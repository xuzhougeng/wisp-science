CREATE TABLE IF NOT EXISTS agent_workflow_deliveries (
    id             TEXT PRIMARY KEY,
    workflow_id    TEXT NOT NULL REFERENCES agent_workflows(id) ON DELETE CASCADE,
    frame_id       TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    generation     INTEGER NOT NULL CHECK(generation > 0),
    auto_resume    INTEGER NOT NULL DEFAULT 0 CHECK(auto_resume IN (0,1)),
    result_json    TEXT,
    message_seq    INTEGER,
    delivered_at  INTEGER,
    resume_status TEXT NOT NULL DEFAULT 'disabled',
    resume_error  TEXT,
    presented_at  INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE(workflow_id, generation),
    UNIQUE(frame_id, message_seq)
);

CREATE INDEX IF NOT EXISTS ix_agent_workflow_deliveries_ready
    ON agent_workflow_deliveries(frame_id, delivered_at, resume_status, created_at);
