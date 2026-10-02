-- Local commit receipts for filesystem journals. Intentionally excluded from
-- portable project exports: they belong to the device performing the operation.
CREATE TABLE IF NOT EXISTS session_file_operations (
    operation_id TEXT NOT NULL,
    role         TEXT NOT NULL CHECK(role IN ('source','target')),
    project_id   TEXT NOT NULL,
    frame_id     TEXT NOT NULL,
    committed_at INTEGER NOT NULL,
    PRIMARY KEY(operation_id, role)
);
