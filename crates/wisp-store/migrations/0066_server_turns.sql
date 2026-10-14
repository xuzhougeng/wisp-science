CREATE TABLE IF NOT EXISTS server_turns (
    frame_id TEXT NOT NULL REFERENCES frames(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    request_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('accepted','completed','failed','interrupted')),
    error TEXT,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(frame_id, request_id)
);
CREATE INDEX IF NOT EXISTS ix_server_turns_frame ON server_turns(frame_id, created_at);
