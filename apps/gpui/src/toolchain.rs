//! Python, R and Node installed into the host's data folder: what can be
//! chosen, and how an install's progress and outcome read.
use wisp_dto::ToolchainStatus;

use crate::{
    i18n::{fill, tr},
    present::Tone,
};

/// The tools offered, by the host's id. uv comes with Python.
pub const CHOICES: [&str; 3] = ["python", "r", "node"];

pub fn tool_name(id: &str) -> &'static str {
    match id {
        "python" => tr("Python (with uv)"),
        "r" => "R",
        "node" => "Node.js",
        "uv" => "uv",
        _ => tr("Other tool"),
    }
}

pub fn running(status: &ToolchainStatus) -> bool {
    status.state == "running"
}

/// What is installed, in a line.
pub fn installed_line(status: &ToolchainStatus) -> String {
    if status.installed.is_empty() {
        return tr("Nothing installed here yet.").to_owned();
    }
    let names: Vec<&str> = status
        .installed
        .iter()
        .map(|id| match id.as_str() {
            "python" => "Python",
            "r" => "R",
            "node" => "Node.js",
            other => other,
        })
        .collect();
    fill(tr("Installed here: {}"), &[&names.join(", ")])
}

/// The step under way, in words.
fn step_name(label: &str) -> String {
    match label {
        "pixi" => tr("Getting pixi").to_owned(),
        "venv" => tr("Making Wisp's Python environment").to_owned(),
        "packages" => tr("Installing the usual analysis packages").to_owned(),
        "save" => tr("Saving where the tools are").to_owned(),
        other => match other.strip_prefix("install:") {
            Some(tool) => fill(tr("Installing {}"), &[&tool_name(tool)]),
            None => other.to_owned(),
        },
    }
}

/// Where a running install is: the step of how many.
pub fn progress_line(status: &ToolchainStatus) -> String {
    if status.step == 0 {
        return tr("Starting…").to_owned();
    }
    fill(
        tr("Step {} of {}: {}"),
        &[&status.step, &status.total, &step_name(&status.label)],
    )
}

/// How the last install ended, when one did.
pub fn outcome(status: &ToolchainStatus) -> Option<(String, Tone)> {
    match status.state.as_str() {
        "done" => Some((
            tr("Installed. Conversations and commands use these tools from now on.").to_owned(),
            Tone::Success,
        )),
        "cancelled" => Some((
            tr("Stopped. What finished stays installed; installing again goes on from there.")
                .to_owned(),
            Tone::Attention,
        )),
        "failed" => Some((
            status
                .error
                .as_deref()
                .map(crate::i18n::host)
                .unwrap_or_else(|| tr("The install stopped.").to_owned()),
            Tone::Danger,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str) -> ToolchainStatus {
        ToolchainStatus {
            state: state.into(),
            ..Default::default()
        }
    }

    #[test]
    fn progress_names_the_step_in_words() {
        let mut running = status("running");
        assert_eq!(progress_line(&running), "Starting…");
        running.step = 2;
        running.total = 5;
        running.label = "install:python".into();
        assert_eq!(
            progress_line(&running),
            "Step 2 of 5: Installing Python (with uv)"
        );
        running.label = "venv".into();
        assert_eq!(
            progress_line(&running),
            "Step 2 of 5: Making Wisp's Python environment"
        );
    }

    #[test]
    fn what_is_installed_and_how_it_ended_read_plainly() {
        let mut idle = status("idle");
        assert_eq!(installed_line(&idle), "Nothing installed here yet.");
        assert!(outcome(&idle).is_none());
        idle.installed = vec!["uv".into(), "python".into(), "r".into()];
        assert_eq!(installed_line(&idle), "Installed here: uv, Python, R");
        let mut failed = status("failed");
        failed.error = Some("Choose at least one tool to install.".into());
        let (text, tone) = outcome(&failed).unwrap();
        assert_eq!(text, "Choose at least one tool to install.");
        assert_eq!(tone, Tone::Danger);
        assert_eq!(outcome(&status("done")).unwrap().1, Tone::Success);
        assert_eq!(outcome(&status("cancelled")).unwrap().1, Tone::Attention);
    }
}
