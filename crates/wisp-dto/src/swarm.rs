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
            log: vec![],
            result: String::new(),
        }
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
}
