//! Runs the user moved to the background from their card, shared by the
//! WebView desktop and the GPUI host. Asking records the request on the Run;
//! that alone ends any `monitor_run` / `wait_for_completion` wait on it (the
//! wait reads the flag), so the agent gets `wait_backgrounded` and finishes
//! its reply while the Run goes on. Once the Run ends, the host claims one
//! completion notice and posts it to the conversation that owns the Run.
//!
//! The request is persisted, so a Run that ends while Wisp is closed is still
//! reported after the next launch (`Store::list_pending_background_runs`).

use std::time::Duration;
use wisp_store::{RunRecord, RunStatus, Store};

/// Records that the user moved `run` to the background. Only a Run started
/// from a conversation qualifies, and only while it has not ended; asking
/// again for a Run already moved is not an error.
pub async fn request(store: &Store, run: &RunRecord) -> Result<(), String> {
    if run.frame_id.is_none() {
        return Err("Only a Run started from a conversation can move to the background".into());
    }
    let requested = store
        .request_run_background(&run.id)
        .await
        .map_err(|error| error.to_string())?;
    if !requested
        && !store
            .run_background_requested(&run.id)
            .await
            .map_err(|error| error.to_string())?
    {
        return Err("The Run has already ended".into());
    }
    Ok(())
}

/// Waits, reading the Run's status every `poll`, until a backgrounded Run
/// ends, then claims its completion notice. `Ok(Some(run))` means this call
/// claimed it and the caller posts it; a notice is claimed once per Run, so
/// two watchers never both post. `Ok(None)`: the Run is gone (deleted with
/// its project or conversation) or its notice was already claimed. A failed
/// status read says nothing about the Run and is waited through; a failed
/// claim is returned, leaving the notice for the next startup scan.
pub async fn claim_when_ended(
    store: &Store,
    run_id: &str,
    poll: Duration,
) -> Result<Option<RunRecord>, String> {
    let run = loop {
        match store.get_run(run_id).await {
            Ok(Some(run)) if run.status.is_terminal() => break run,
            Ok(Some(_)) => {}
            Ok(None) => return Ok(None),
            Err(error) => {
                tracing::debug!(target: "wisp", %run_id, %error, "background Run status read failed")
            }
        }
        tokio::time::sleep(poll).await;
    };
    match store.claim_run_background_notice(run_id).await {
        Ok(true) => Ok(Some(run)),
        Ok(false) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// The message posted to the owning conversation when a backgrounded Run
/// ends: its title, outcome, exit code, time taken, id and context.
pub fn completion_notice(chinese: bool, run: &RunRecord) -> String {
    let outcome = match (run.status, chinese) {
        (RunStatus::Succeeded, true) => "已成功完成",
        (RunStatus::Succeeded, false) => "succeeded",
        (RunStatus::Failed, true) => "失败",
        (RunStatus::Failed, false) => "failed",
        (RunStatus::Cancelled, true) => "已取消",
        (RunStatus::Cancelled, false) => "was cancelled",
        (RunStatus::TimedOut, true) => "超时",
        (RunStatus::TimedOut, false) => "timed out",
        (_, true) => "失去连接",
        (_, false) => "lost its connection",
    };
    let mut facts = Vec::new();
    if let Some(code) = run.exit_code {
        facts.push(if chinese {
            format!("退出码 {code}")
        } else {
            format!("exit code {code}")
        });
    }
    if let (Some(started), Some(ended)) = (run.started_at, run.ended_at) {
        let elapsed = elapsed_label(ended.saturating_sub(started), chinese);
        facts.push(if chinese {
            format!("用时 {elapsed}")
        } else {
            format!("took {elapsed}")
        });
    }
    let facts = if facts.is_empty() {
        String::new()
    } else if chinese {
        format!("（{}）", facts.join("，"))
    } else {
        format!(" ({})", facts.join(", "))
    };
    if chinese {
        format!(
            "后台任务《{}》{outcome}{facts}。\n\nRun `{}` · {}",
            run.title, run.id, run.context_id
        )
    } else {
        format!(
            "Background Run “{}” {outcome}{facts}.\n\nRun `{}` · {}",
            run.title, run.id, run.context_id
        )
    }
}

fn elapsed_label(seconds: i64, chinese: bool) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    match (hours, minutes, chinese) {
        (0, 0, true) => format!("{seconds} 秒"),
        (0, 0, false) => format!("{seconds}s"),
        (0, _, true) => format!("{minutes} 分 {seconds} 秒"),
        (0, _, false) => format!("{minutes}m {seconds}s"),
        (_, _, true) => format!("{hours} 小时 {minutes} 分"),
        (_, _, false) => format!("{hours}h {minutes}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: RunStatus) -> RunRecord {
        let mut run = RunRecord::new("run-1", "p", "ssh:gpu", "Align reads", "ssh_direct");
        run.status = status;
        run.started_at = Some(100);
        run.ended_at = Some(100 + 3 * 3600 + 25 * 60 + 7);
        run.exit_code = Some(0);
        run
    }

    #[test]
    fn notice_names_the_run_its_outcome_and_how_to_find_it() {
        let succeeded = run(RunStatus::Succeeded);
        assert_eq!(
            completion_notice(true, &succeeded),
            "后台任务《Align reads》已成功完成（退出码 0，用时 3 小时 25 分）。\n\nRun `run-1` · ssh:gpu"
        );
        assert_eq!(
            completion_notice(false, &succeeded),
            "Background Run “Align reads” succeeded (exit code 0, took 3h 25m).\n\nRun `run-1` · ssh:gpu"
        );
        let mut failed = run(RunStatus::Failed);
        failed.exit_code = Some(137);
        failed.ended_at = Some(100 + 65);
        assert!(completion_notice(true, &failed).contains("失败（退出码 137，用时 1 分 5 秒）"));
        let mut lost = run(RunStatus::Lost);
        lost.exit_code = None;
        lost.started_at = None;
        assert_eq!(
            completion_notice(false, &lost),
            "Background Run “Align reads” lost its connection.\n\nRun `run-1` · ssh:gpu"
        );
    }

    #[test]
    fn elapsed_label_keeps_the_two_largest_units() {
        assert_eq!(elapsed_label(42, false), "42s");
        assert_eq!(elapsed_label(61, true), "1 分 1 秒");
        assert_eq!(elapsed_label(7200 + 59, false), "2h 0m");
    }

    #[tokio::test]
    async fn a_backgrounded_run_is_reported_once_when_it_ends() {
        let path = std::env::temp_dir().join(format!(
            "wisp_background_notice_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store.create_project("p", "proj", "").await.unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        let mut owned = RunRecord::new("owned", "p", "local", "Align", "local_detached");
        owned.frame_id = Some("f".into());
        owned.status = RunStatus::Running;
        store.create_run(&owned).await.unwrap();
        let mut loose = RunRecord::new("loose", "p", "local", "Align", "local_detached");
        loose.status = RunStatus::Running;
        store.create_run(&loose).await.unwrap();
        let mut done = RunRecord::new("done", "p", "local", "Align", "local_detached");
        done.frame_id = Some("f".into());
        done.status = RunStatus::Failed;
        store.create_run(&done).await.unwrap();

        // Only a conversation's Run can go to the background.
        let refused = request(&store, &loose).await.unwrap_err();
        assert!(refused.contains("started from a conversation"), "{refused}");
        request(&store, &owned).await.unwrap();
        // Asking twice is fine; the Run is untouched.
        request(&store, &owned).await.unwrap();
        let still = store.get_run("owned").await.unwrap().unwrap();
        assert_eq!(still.status, RunStatus::Running);
        assert!(store.run_background_requested("owned").await.unwrap());

        // It ends; one watcher claims the notice, a second gets nothing.
        assert!(store
            .claim_run_lifecycle("owned", "test", 60)
            .await
            .unwrap());
        assert!(store
            .finish_active_run_owned("owned", "test", RunStatus::Succeeded, Some(0))
            .await
            .unwrap());
        let poll = Duration::from_millis(5);
        let ended = claim_when_ended(&store, "owned", poll).await.unwrap();
        assert_eq!(ended.map(|run| run.status), Some(RunStatus::Succeeded));
        assert!(claim_when_ended(&store, "owned", poll)
            .await
            .unwrap()
            .is_none());
        assert!(store
            .list_pending_background_runs()
            .await
            .unwrap()
            .is_empty());
        // An ended Run cannot be moved any more.
        let refused = request(&store, &done).await.unwrap_err();
        assert!(refused.contains("already ended"), "{refused}");
        // A Run that is gone has nobody to tell.
        assert!(claim_when_ended(&store, "ghost", poll)
            .await
            .unwrap()
            .is_none());
        drop(store);
        let _ = std::fs::remove_file(&path);
    }
}
