//! ACP agents: how an agent's arguments are typed on one line, and how what
//! it said about itself reads.
use wisp_server::desktop::{AcpAgentReport, AcpAgentRow};

use crate::i18n::{fill, tr};

/// The arguments typed on one line: split at spaces, with double quotes
/// keeping a space inside one argument.
pub fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ch if ch.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            ch => {
                current.push(ch);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// The arguments back on one line, quoting those with a space.
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(char::is_whitespace) {
                format!("\"{arg}\"")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command as it is started, in a line.
pub fn command_line(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_owned()
    } else {
        format!("{command} {}", join_args(args))
    }
}

/// The name of the agent `id` names, as the composer shows it.
pub fn agent_label(agents: &[AcpAgentRow], id: &str) -> String {
    agents
        .iter()
        .find(|row| row.agent.id == id)
        .map(|row| row.agent.label.clone())
        .unwrap_or_else(|| tr("ACP agent").to_owned())
}

/// What a test found: its name and version, the protocol, and how it signs
/// in.
pub fn report_line(report: &AcpAgentReport) -> String {
    let mut parts = Vec::new();
    match (report.name.trim(), report.version.trim()) {
        ("", _) => parts.push(tr("Started").to_owned()),
        (name, "") => parts.push(name.to_owned()),
        (name, version) => parts.push(format!("{name} {version}")),
    }
    parts.push(format!("ACP v{}", report.protocol));
    if !report.sign_in.is_empty() {
        parts.push(fill(tr("sign in: {}"), &[&report.sign_in.join(", ")]));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_split_at_spaces_and_keep_quoted_ones_whole() {
        assert_eq!(
            split_args("  --acp   --model x "),
            ["--acp", "--model", "x"]
        );
        assert_eq!(
            split_args(r#"--name "two words" "" end"#),
            ["--name", "two words", "", "end"]
        );
        assert!(split_args("   ").is_empty());
        let args = split_args(r#"--name "two words" end"#);
        assert_eq!(join_args(&args), r#"--name "two words" end"#);
        assert_eq!(split_args(&join_args(&args)), args);
        assert_eq!(command_line("agent", &[]), "agent");
        assert_eq!(
            command_line("agent", &args),
            r#"agent --name "two words" end"#
        );
    }

    #[test]
    fn the_composer_names_the_agent_or_says_it_is_one() {
        let rows = vec![AcpAgentRow {
            agent: wisp_server::desktop::AcpAgent {
                id: "a".into(),
                label: "Fake agent".into(),
                command: "agent".into(),
                args: vec![],
            },
            found: true,
        }];
        assert_eq!(agent_label(&rows, "a"), "Fake agent");
        assert_eq!(agent_label(&rows, "gone"), "ACP agent");
    }

    #[test]
    fn a_report_names_the_agent_and_how_it_signs_in() {
        let mut report = AcpAgentReport {
            protocol: 1,
            name: "Fake agent".into(),
            version: "0.1.0".into(),
            sign_in: vec!["Sign in".into()],
        };
        assert_eq!(
            report_line(&report),
            "Fake agent 0.1.0 · ACP v1 · sign in: Sign in"
        );
        report.sign_in.clear();
        report.version.clear();
        assert_eq!(report_line(&report), "Fake agent · ACP v1");
        report.name.clear();
        assert_eq!(report_line(&report), "Started · ACP v1");
    }
}
