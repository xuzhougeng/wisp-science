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

#[cfg(test)]
mod tests {
    use super::*;

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
