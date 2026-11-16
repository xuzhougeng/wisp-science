//! Opening the system's terminal in a project's folder. The app has no
//! terminal of its own; it starts the one the system has, there.
use std::path::Path;

use crate::notify::System;

/// What is run to open a terminal: a program, what it is given, and
/// whether it is started inside the folder instead of being told it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: &'static str,
    pub arguments: Vec<String>,
    pub inside: bool,
}

/// How `system` is asked for a terminal in `folder`. The folder is handed
/// over as an argument of its own or as the working directory, never as
/// part of a command line to be parsed.
pub fn launch(system: System, folder: &Path) -> Launch {
    let folder = folder.to_string_lossy().into_owned();
    match system {
        System::Mac => Launch {
            program: "/usr/bin/open",
            arguments: vec!["-a".into(), "Terminal".into(), folder],
            inside: false,
        },
        // `start` takes the first quoted word as a window title.
        System::Windows => Launch {
            program: "cmd.exe",
            arguments: vec![
                "/C".into(),
                "start".into(),
                String::new(),
                "/D".into(),
                folder,
                "cmd.exe".into(),
            ],
            inside: false,
        },
        // Terminals disagree about the option for a folder; all of them
        // start where they are started.
        System::Linux => Launch {
            program: "x-terminal-emulator",
            arguments: vec![],
            inside: true,
        },
    }
}

/// Opens the terminal. A folder that is gone, or a system without the
/// program, is said rather than passed over.
pub fn open(folder: &Path) -> Result<(), String> {
    if !folder.is_dir() {
        return Err(crate::i18n::fill(
            crate::i18n::tr("The folder {} is not there."),
            &[&folder.display()],
        ));
    }
    let launch = launch(System::current(), folder);
    let mut command = std::process::Command::new(launch.program);
    command
        .args(&launch.arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if launch.inside {
        command.current_dir(folder);
    }
    command.spawn().map(drop).map_err(|problem| {
        crate::i18n::fill(
            crate::i18n::tr("Could not open a terminal: {}"),
            &[&problem],
        )
    })
}

/// One word for `/bin/sh`, whatever it holds.
fn quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// What a terminal on macOS runs to start `program` in `folder` with
/// `env` set: every value is quoted, and a name that is not one is left
/// out.
pub fn script(folder: &Path, program: &str, args: &[String], env: &[(String, String)]) -> String {
    let mut text = format!(
        "#!/bin/sh\ncd {} || exit 1\n",
        quoted(&folder.to_string_lossy())
    );
    for (name, value) in env {
        let valid = name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        if valid {
            text.push_str(&format!("export {name}={}\n", quoted(value)));
        }
    }
    text.push_str("exec ");
    text.push_str(
        &std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .map(quoted)
            .collect::<Vec<_>>()
            .join(" "),
    );
    text.push('\n');
    text
}

/// Opens the system's terminal running `program` in `folder`, for a
/// program that asks the user something there (an agent's sign-in).
pub fn run(
    folder: &Path,
    program: &str,
    args: &[String],
    env: &[(String, String)],
) -> Result<(), String> {
    if !folder.is_dir() {
        return Err(crate::i18n::fill(
            crate::i18n::tr("The folder {} is not there."),
            &[&folder.display()],
        ));
    }
    let failed = |problem: std::io::Error| {
        crate::i18n::fill(
            crate::i18n::tr("Could not open a terminal: {}"),
            &[&problem],
        )
    };
    let mut command = match System::current() {
        System::Mac => {
            // Terminal runs a `.command` file it is given; the file holds
            // the command, quoted, so nothing is parsed from a string.
            let path = std::env::temp_dir().join(format!(
                "wisp-{}-{}.command",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_nanos())
                    .unwrap_or_default()
            ));
            std::fs::write(&path, script(folder, program, args, env)).map_err(failed)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                    .map_err(failed)?;
            }
            let mut command = std::process::Command::new("/usr/bin/open");
            command.args(["-a", "Terminal"]).arg(path);
            command
        }
        System::Windows => {
            let mut command = std::process::Command::new("cmd.exe");
            command
                .args(["/C", "start", "", "/D"])
                .arg(folder)
                .arg(program)
                .args(args)
                .envs(env.iter().cloned());
            command
        }
        System::Linux => {
            let mut command = std::process::Command::new("x-terminal-emulator");
            command
                .arg("-e")
                .arg(program)
                .args(args)
                .current_dir(folder)
                .envs(env.iter().cloned());
            command
        }
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_for_the_terminal_is_quoted_word_by_word() {
        let text = script(
            Path::new("/work/it's here"),
            "/opt/agent bin/agent",
            &["auth".into(), "log in; rm -rf ~".into()],
            &[
                ("AGENT_TOKEN_FILE".into(), "a'b".into()),
                ("not a name".into(), "x".into()),
            ],
        );
        assert_eq!(
            text,
            "#!/bin/sh\ncd '/work/it'\\''s here' || exit 1\n\
             export AGENT_TOKEN_FILE='a'\\''b'\n\
             exec '/opt/agent bin/agent' 'auth' 'log in; rm -rf ~'\n"
        );
    }

    #[test]
    fn each_system_is_asked_its_own_way_with_the_folder_as_data() {
        let folder = Path::new("/work/my project; rm -rf ~");
        let mac = launch(System::Mac, folder);
        assert_eq!(mac.program, "/usr/bin/open");
        // One argument, whatever is in the name.
        assert_eq!(
            mac.arguments,
            ["-a", "Terminal", "/work/my project; rm -rf ~"]
        );
        assert!(!mac.inside);

        let windows = launch(System::Windows, Path::new(r"C:\lab\rna seq"));
        assert_eq!(
            windows.arguments,
            ["/C", "start", "", "/D", r"C:\lab\rna seq", "cmd.exe"]
        );

        let linux = launch(System::Linux, folder);
        assert_eq!(linux.program, "x-terminal-emulator");
        assert!(linux.arguments.is_empty() && linux.inside);
    }

    #[test]
    fn a_folder_that_is_gone_is_said_and_nothing_is_started() {
        let problem = open(Path::new("/definitely/not/here")).unwrap_err();
        assert!(problem.contains("/definitely/not/here"), "{problem}");
    }
}
