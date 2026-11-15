//! How far a run that moves files between machines has got, as the run
//! manager records it in the run's progress: what is said about it, and
//! whether it still shows above the composer. No graphics.
use crate::i18n::{fill, tr};
use wisp_dto::{RunProgress, RunRecord};

/// How long a transfer that has ended stays in view.
pub const LINGER_SECONDS: i64 = 3;

/// The transfer progress a run records, if it records one.
pub fn progress(run: &RunRecord) -> Option<RunProgress> {
    serde_json::from_str(&run.progress_json).ok()
}

/// Whether a transfer shows: while it moves bytes, and a moment after.
pub fn visible(progress: &RunProgress, status: &str, now: i64) -> bool {
    (matches!(status, "submitted" | "running" | "cancelling")
        && matches!(progress.phase.as_str(), "uploading" | "downloading"))
        || (now - progress.updated_at).abs() <= LINGER_SECONDS
}

/// What one transfer shows: where it is, which file, how much of it, and
/// the share done when that is known.
#[derive(Debug, PartialEq)]
pub struct Meter {
    pub phase: &'static str,
    pub file: Option<String>,
    pub detail: String,
    /// From 0 to 1; `None` when the transport cannot say.
    pub done: Option<f32>,
}

pub fn meter(progress: &RunProgress) -> Meter {
    let moving = matches!(progress.phase.as_str(), "uploading" | "downloading");
    let unknown = moving && (progress.indeterminate || progress.total_bytes == 0);
    let percent = if progress.total_bytes == 0 {
        0
    } else {
        progress
            .completed_bytes
            .saturating_mul(100)
            .div_ceil(progress.total_bytes)
            .min(100)
    };
    let phase = match progress.phase.as_str() {
        "uploading" => tr("Uploading"),
        "uploaded" => tr("Upload complete"),
        "downloading" => tr("Downloading"),
        "downloaded" => tr("Download complete"),
        "cancelled" => tr("Cancelled"),
        "failed" => tr("Transfer failed"),
        _ => tr("Transferring"),
    };
    let size = crate::files::size_label;
    let mut parts = vec![if unknown {
        if progress.total_bytes > 0 {
            fill(
                tr("{} in all; how much is done cannot be told"),
                &[&size(progress.total_bytes)],
            )
        } else {
            tr("How much is done cannot be told").to_owned()
        }
    } else {
        format!(
            "{} / {} · {percent}%",
            size(progress.completed_bytes),
            size(progress.total_bytes)
        )
    }];
    if !unknown {
        if let Some(rate) = progress.bytes_per_second {
            parts.push(format!("{}/s", size(rate)));
        }
        if let Some(seconds) = progress.eta_seconds {
            parts.push(fill(tr("{} left"), &[&duration(seconds)]));
        }
        if progress.files_total > 1 {
            parts.push(fill(
                tr("{}/{} files"),
                &[&progress.files_completed, &progress.files_total],
            ));
        }
    }
    Meter {
        phase,
        file: progress.current_file.clone(),
        detail: parts.join(" · "),
        done: (!unknown).then_some(percent as f32 / 100.),
    }
}

/// A time to go, as a clock reads it.
pub fn duration(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// The transfers of the open conversation that show now, newest first as
/// the run list has them.
pub fn shown<'a>(runs: &'a [RunRecord], now: i64) -> Vec<(&'a RunRecord, RunProgress)> {
    runs.iter()
        .filter_map(|run| {
            let progress = progress(run)?;
            visible(&progress, &run.status, now).then_some((run, progress))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn recorded(phase: &str, done: u64, total: u64, updated_at: i64) -> serde_json::Value {
        json!({
            "indeterminate": false, "phase": phase, "direction": "download",
            "completed_bytes": done, "total_bytes": total,
            "files_completed": 1, "files_total": 3,
            "current_file": "reads_1.fq.gz", "bytes_per_second": 2048,
            "eta_seconds": 75, "updated_at": updated_at
        })
    }

    fn moving(phase: &str, done: u64, total: u64, updated_at: i64) -> RunProgress {
        serde_json::from_value(recorded(phase, done, total, updated_at)).unwrap()
    }

    fn run(status: &str, progress: Option<serde_json::Value>) -> RunRecord {
        serde_json::from_value(json!({
            "id": "r", "frame_id": "s", "context_id": "lab", "title": "Download reads",
            "kind": "file_transfer", "status": status, "command": "download",
            "created_at": 1, "started_at": null, "ended_at": null, "exit_code": null,
            "stdout_tail": null, "stderr_tail": null, "remote_workdir": null,
            "timeout_secs": null, "last_polled_at": null, "last_poll_error": null,
            "progress_json": progress.map_or("{}".to_owned(), |p| p.to_string()),
            "env_snapshot_json": "{}"
        }))
        .unwrap()
    }

    #[test]
    fn a_transfer_says_how_far_it_has_got() {
        let shown = meter(&moving("downloading", 1024, 4096, 0));
        assert_eq!(shown.phase, "Downloading");
        assert_eq!(shown.file.as_deref(), Some("reads_1.fq.gz"));
        assert_eq!(
            shown.detail,
            "1.0 KB / 4.0 KB · 25% · 2.0 KB/s · 1:15 left · 1/3 files"
        );
        assert_eq!(shown.done, Some(0.25));

        // A transport that cannot count bytes says so, and nothing more.
        let mut blind = moving("uploading", 0, 4096, 0);
        blind.indeterminate = true;
        let shown = meter(&blind);
        assert_eq!(shown.phase, "Uploading");
        assert_eq!(
            shown.detail,
            "4.0 KB in all; how much is done cannot be told"
        );
        assert_eq!(shown.done, None);
        blind.total_bytes = 0;
        assert_eq!(meter(&blind).detail, "How much is done cannot be told");

        assert_eq!(meter(&moving("failed", 1, 2, 0)).phase, "Transfer failed");
        assert_eq!(duration(75), "1:15");
        assert_eq!(duration(3_725), "1:02:05");
    }

    #[test]
    fn a_transfer_shows_while_it_moves_and_a_moment_after() {
        let going = moving("downloading", 1, 2, 100);
        assert!(visible(&going, "running", 1_000));
        assert!(!visible(&going, "succeeded", 1_000));
        let ended = moving("downloaded", 2, 2, 100);
        assert!(visible(&ended, "succeeded", 100 + LINGER_SECONDS));
        assert!(!visible(&ended, "succeeded", 100 + LINGER_SECONDS + 1));

        let runs = [
            run("running", Some(recorded("downloading", 1, 2, 100))),
            run("running", None),
            run("succeeded", Some(recorded("downloaded", 2, 2, 100))),
        ];
        let found = shown(&runs, 1_000);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].0.status.as_str(), found[0].1.phase.as_str()),
            ("running", "downloading")
        );
        // A run whose progress is not a transfer's shows nothing.
        assert!(progress(&runs[1]).is_none());
    }
}
