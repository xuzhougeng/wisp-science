-- Global research-plan change sequence; offline clients never compare wall clocks.
CREATE TABLE IF NOT EXISTS research_checklist_clock (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    epoch TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO research_checklist_clock(singleton, epoch, revision)
VALUES(1, lower(hex(randomblob(16))), 0);

CREATE INDEX IF NOT EXISTS ix_assistant_tasks_revision ON assistant_tasks(sync_revision);
CREATE TRIGGER IF NOT EXISTS research_checklist_insert AFTER INSERT ON assistant_tasks
BEGIN
    UPDATE research_checklist_clock SET revision = revision + 1 WHERE singleton = 1;
    UPDATE assistant_tasks SET sync_revision = (SELECT revision FROM research_checklist_clock WHERE singleton = 1) WHERE id = NEW.id;
END;
CREATE TRIGGER IF NOT EXISTS research_checklist_update
AFTER UPDATE OF day,title,project_id,session_id,status,created_at,updated_at ON assistant_tasks
BEGIN
    UPDATE research_checklist_clock SET revision = revision + 1 WHERE singleton = 1;
    UPDATE assistant_tasks SET sync_revision = (SELECT revision FROM research_checklist_clock WHERE singleton = 1) WHERE id = NEW.id;
END;
-- Seed legacy rows once. Reopening the database does not advance their revisions.
UPDATE assistant_tasks SET updated_at = updated_at WHERE sync_revision = 0;
