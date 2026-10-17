//! Shared progress contract for map_items, Reader and nested exploration.
use serde::{Deserialize, Serialize};

pub const SWARM_PRESENTATION: &str = "agent_swarm";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwarmPhase {
    Starting,
    Running,
    Summarizing,
    Completed,
    Partial,
    Failed,
    Paused,
    Cancelled,
}

impl SwarmPhase {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::Starting | Self::Running | Self::Summarizing)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwarmTaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Paused,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmTask {
    pub id: String,
    pub title: String,
    pub instruction: String,
    pub status: SwarmTaskStatus,
    pub revision: u64,
    pub completed_units: usize,
    pub total_units: usize,
    pub failed_units: usize,
    /// Zero-based units that failed, so a retry can re-run only those parts.
    /// Older snapshots omit it; a retry then re-runs the whole task.
    #[serde(default)]
    pub failed_unit_indexes: Vec<usize>,
    /// Bounded tool/progress observations, never hidden reasoning.
    pub log: Vec<String>,
    pub result: String,
}

impl SwarmTask {
    pub fn queued(id: String, title: String, instruction: String, total_units: usize) -> Self {
        Self {
            id,
            title,
            instruction,
            total_units,
            status: SwarmTaskStatus::Queued,
            revision: 0,
            completed_units: 0,
            failed_units: 0,
            failed_unit_indexes: vec![],
            log: vec![],
            result: String::new(),
        }
    }

    /// Put a failed task back to work on its failed units only. Completed
    /// units and their evidence are kept; the failure bookkeeping is cleared so
    /// the retry can report fresh outcomes.
    pub fn reset_for_retry(&mut self) {
        self.completed_units = self.completed_units.saturating_sub(self.failed_units);
        self.failed_units = 0;
        self.failed_unit_indexes.clear();
        self.status = SwarmTaskStatus::Queued;
    }
}

impl SwarmProgress {
    /// Reader batches can re-run their failed tasks after the batch ended.
    pub fn retryable_task_ids(&self) -> Vec<String> {
        if !self.phase.terminal() || self.source != "reader" {
            return vec![];
        }
        self.tasks
            .iter()
            .filter(|task| task.status == SwarmTaskStatus::Failed)
            .map(|task| task.id.clone())
            .collect()
    }
}

/// Start and terminal events contain all tasks; live updates contain changed
/// tasks only. The same shape is returned as a full snapshot on session load.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmProgress {
    pub id: String,
    pub source: String,
    pub title: String,
    pub model: String,
    pub started_at_ms: i64,
    pub revision: u64,
    pub phase: SwarmPhase,
    pub total: usize,
    pub tasks: Vec<SwarmTask>,
    pub summary: String,
}

impl SwarmProgress {
    /// Merge a live patch or a snapshot fetched before a newer live update.
    /// Per-task revisions let the older snapshot fill missing queued tasks
    /// without rolling a completed task back to running.
    pub fn merge(&mut self, update: Self) {
        if self.id != update.id {
            if update.started_at_ms >= self.started_at_ms {
                *self = update;
            }
            return;
        }
        if update.revision >= self.revision {
            self.revision = update.revision;
            self.phase = update.phase;
            self.total = update.total;
            self.title = update.title;
            self.model = update.model;
            self.summary = update.summary;
        }
        for task in update.tasks {
            if let Some(current) = self.tasks.iter_mut().find(|item| item.id == task.id) {
                if task.revision >= current.revision {
                    *current = task;
                }
            } else {
                self.tasks.push(task);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SwarmProgress {
        SwarmProgress {
            id: "batch".into(),
            source: "map_items".into(),
            title: "Extract".into(),
            model: "fake".into(),
            started_at_ms: 1,
            revision: 2,
            phase: SwarmPhase::Running,
            total: 2,
            tasks: vec![
                SwarmTask::queued("a".into(), "A".into(), "Extract".into(), 1),
                SwarmTask::queued("b".into(), "B".into(), "Extract".into(), 1),
            ],
            summary: String::new(),
        }
    }

    #[test]
    fn late_snapshot_fills_missing_tasks_without_rolling_back_live_work() {
        let old = snapshot();
        let mut live = old.clone();
        live.revision = 5;
        live.tasks.truncate(1);
        live.tasks[0].revision = 5;
        live.tasks[0].status = SwarmTaskStatus::Completed;
        live.merge(old);
        assert_eq!(live.tasks.len(), 2);
        assert_eq!(live.tasks[0].status, SwarmTaskStatus::Completed);
        assert_eq!(live.revision, 5);
        let mut previous_run = snapshot();
        previous_run.id = "previous".into();
        previous_run.started_at_ms = 0;
        live.merge(previous_run);
        assert_eq!(live.id, "batch");
    }

    #[test]
    fn older_snapshots_without_failed_unit_indexes_still_deserialize() {
        let raw = r#"{"id":"a","title":"A","instruction":"Read","status":"failed","revision":3,
            "completed_units":3,"total_units":3,"failed_units":1,"log":[],"result":""}"#;
        let task: SwarmTask = serde_json::from_str(raw).unwrap();
        assert!(task.failed_unit_indexes.is_empty());
    }

    #[test]
    fn retry_reset_keeps_completed_units_and_only_reader_batches_offer_retries() {
        let mut progress = snapshot();
        progress.source = "reader".into();
        progress.phase = SwarmPhase::Partial;
        progress.tasks[0].status = SwarmTaskStatus::Completed;
        progress.tasks[0].completed_units = 1;
        let failed = &mut progress.tasks[1];
        failed.status = SwarmTaskStatus::Failed;
        failed.total_units = 3;
        failed.completed_units = 3;
        failed.failed_units = 1;
        failed.failed_unit_indexes = vec![1];
        assert_eq!(progress.retryable_task_ids(), vec!["b".to_string()]);
        progress.tasks[1].reset_for_retry();
        let task = &progress.tasks[1];
        assert_eq!(task.status, SwarmTaskStatus::Queued);
        assert_eq!(task.completed_units, 2);
        assert_eq!(task.failed_units, 0);
        assert!(task.failed_unit_indexes.is_empty());
        progress.phase = SwarmPhase::Running;
        assert!(progress.retryable_task_ids().is_empty());
        progress.phase = SwarmPhase::Partial;
        progress.source = "map_items".into();
        assert!(progress.retryable_task_ids().is_empty());
    }
}
