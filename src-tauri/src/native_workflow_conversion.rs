//! Bounded, ephemeral progress for native conversion views. Drafts still return
//! through the original command; this never estimates progress or starts work.
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use wisp_dto::{WorkflowConversionProgress, WorkflowConversionStage};

const CAPACITY: usize = 128;
const RETENTION: Duration = Duration::from_secs(3600);

struct Entry {
    project: String,
    progress: WorkflowConversionProgress,
    updated: Instant,
}

#[derive(Default)]
pub(crate) struct Progress(Mutex<VecDeque<Entry>>);

impl Progress {
    pub(crate) fn record(&self, project: &str, id: &str, stage: WorkflowConversionStage) {
        self.record_at(project, id, stage, Instant::now());
    }

    fn record_at(&self, project: &str, id: &str, stage: WorkflowConversionStage, now: Instant) {
        if project.is_empty() || !valid_id(id) {
            return;
        }
        let mut entries = self.0.lock().unwrap_or_else(|error| error.into_inner());
        entries.retain(|entry| {
            now.duration_since(entry.updated) < RETENTION
                && !(entry.project == project && entry.progress.conversion_id == id)
        });
        while entries.len() >= CAPACITY {
            entries.pop_front();
        }
        entries.push_back(Entry {
            project: project.into(),
            progress: WorkflowConversionProgress {
                conversion_id: id.into(),
                stage,
            },
            updated: now,
        });
    }

    pub(crate) fn read(&self, project: &str, id: &str) -> Option<WorkflowConversionProgress> {
        self.read_at(project, id, Instant::now())
    }

    fn read_at(&self, project: &str, id: &str, now: Instant) -> Option<WorkflowConversionProgress> {
        let mut entries = self.0.lock().unwrap_or_else(|error| error.into_inner());
        entries.retain(|entry| now.duration_since(entry.updated) < RETENTION);
        entries
            .iter()
            .find(|entry| entry.project == project && entry.progress.conversion_id == id)
            .map(|entry| entry.progress.clone())
    }
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 200
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_keeps_request_and_project_scope_and_latest_host_stage() {
        let progress = Progress::default();
        let now = Instant::now();
        progress.record_at(
            "project-a",
            "request",
            WorkflowConversionStage::Generating,
            now,
        );
        progress.record_at(
            "project-b",
            "request",
            WorkflowConversionStage::ReadingSources,
            now,
        );
        progress.record_at(
            "project-a",
            "request",
            WorkflowConversionStage::Repairing,
            now,
        );
        assert_eq!(
            progress.read_at("project-a", "request", now).unwrap().stage,
            WorkflowConversionStage::Repairing
        );
        assert_eq!(
            progress.read_at("project-b", "request", now).unwrap().stage,
            WorkflowConversionStage::ReadingSources
        );
        assert!(progress.read_at("other-project", "request", now).is_none());
        assert!(progress
            .read_at("project-a", "other-request", now)
            .is_none());
        assert!(progress
            .read_at("project-a", "request", now + RETENTION)
            .is_none());
    }

    #[test]
    fn retention_is_bounded_and_invalid_identifiers_are_not_recorded() {
        let progress = Progress::default();
        let now = Instant::now();
        progress.record_at("project", " ", WorkflowConversionStage::Preparing, now);
        progress.record_at(
            "project",
            &"x".repeat(201),
            WorkflowConversionStage::Preparing,
            now,
        );
        assert!(progress.0.lock().unwrap().is_empty());
        for index in 0..=CAPACITY {
            progress.record_at(
                "project",
                &index.to_string(),
                WorkflowConversionStage::Validating,
                now,
            );
        }
        assert_eq!(progress.0.lock().unwrap().len(), CAPACITY);
        assert!(progress.read_at("project", "0", now).is_none());
        assert!(progress
            .read_at("project", &CAPACITY.to_string(), now)
            .is_some());
    }
}
