use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

const HELP: &str = "Repository commands:
  cargo run build                 Build the WebView desktop once (debug, no installer)
  cargo run build --release       Build release installers using the existing Tauri pipeline
  cargo run build-gpui            Build the cross-platform GPUI desktop (debug)
  cargo run build-gpui --release  Build the GPUI desktop in release mode
  cargo run gpui                  Launch the GPUI desktop
  cargo run dev [TAURI OPTIONS]   Run the desktop with hot reload

Builds do not launch the app or watch source changes. See docs/development.md
for output paths, prerequisites, and how to launch the built application.

Other commands (including no arguments) are forwarded to the headless CLI:
  cargo run -- run <prompt>
  cargo run -- eval
  cargo run -p wisp-cli -- --help
";

#[derive(Debug, PartialEq, Eq)]
struct Step {
    program: &'static str,
    args: Vec<OsString>,
    repository_cwd: bool,
    fast_build: bool,
}

impl Step {
    fn new(program: &'static str, args: &[&str]) -> Self {
        Self {
            program,
            args: args.iter().map(OsString::from).collect(),
            repository_cwd: true,
            fast_build: false,
        }
    }

    fn command(&self, root: &Path) -> Command {
        let program = if self.program == "cargo" {
            std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
        } else {
            self.program.into()
        };
        let mut command = Command::new(program);
        command.args(&self.args);
        if self.repository_cwd {
            command.current_dir(root);
        }
        if self.fast_build {
            command.env("WISP_CATALOG_OFFLINE", "1");
        }
        command
    }
}

fn plan(args: &[OsString], os: &str) -> Result<Step, String> {
    let name = args.first().and_then(|arg| arg.to_str()).unwrap_or("");
    let options = args.get(1..).unwrap_or_default();
    let mut step = match name {
        "build" => {
            let release = release_option(options)?;
            if release {
                Step::new("cargo", &["tauri", "build"])
            } else {
                Step::new(
                    "cargo",
                    &[
                        "tauri",
                        "build",
                        "--debug",
                        "--no-bundle",
                        "--config",
                        "src-tauri/tauri.fast.conf.json",
                    ],
                )
            }
        }
        "build-gpui" | "gpui" => {
            let release = release_option(options)?;
            if !matches!(os, "macos" | "windows" | "linux") {
                return Err("GPUI desktop requires macOS, Windows, or Linux.".into());
            }
            let mut step = Step::new(
                "cargo",
                &[
                    if name == "gpui" { "run" } else { "build" },
                    "--locked",
                    "--manifest-path",
                    "apps/gpui/Cargo.toml",
                ],
            );
            if release {
                step.args.push("--release".into());
            }
            step
        }
        "build-win" | "build-mac" => {
            return Err("The SwiftUI and WinUI desktops have been retired. Use cargo run build-gpui on macOS, Windows, or Linux; see docs/gpui-desktop.md.".into());
        }
        "dev" => {
            let mut step = Step::new("cargo", &["tauri", "dev"]);
            step.args.extend_from_slice(options);
            step
        }
        _ => {
            let mut step = Step::new(
                "cargo",
                &[
                    "run",
                    "--quiet",
                    "--manifest-path",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"),
                    "-p",
                    "wisp-cli",
                    "--bin",
                    "wisp-science",
                    "--",
                ],
            );
            step.args.extend_from_slice(args);
            // CLI prompts and evaluation paths belong to the caller's project.
            step.repository_cwd = false;
            step
        }
    };
    step.fast_build = matches!(name, "build") && !options.iter().any(|arg| arg == "--release");
    Ok(step)
}

fn release_option(args: &[OsString]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [arg] if arg == "--release" => Ok(true),
        _ => Err("expected no options or --release; use cargo run -- --help for usage".into()),
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "help" || arg == "--help" || arg == "-h")
    {
        print!("{HELP}");
        return;
    }
    let step = match plan(&args, std::env::consts::OS) {
        Ok(step) => step,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("launcher is in crates/wisp-dev");
    let mut command = step.command(root);
    if step.repository_cwd {
        eprintln!("Running: {command:?}");
    }
    match command.status() {
        Ok(status) => {
            if status.success() && args.first().is_some_and(|arg| arg == "build") {
                if step.fast_build {
                    eprintln!("Built desktop executable in Cargo's debug output directory (normally target/debug/wisp-tauri{}). Run it directly; no watcher is started.", std::env::consts::EXE_SUFFIX);
                } else {
                    eprintln!("Built release bundles in Cargo's release output directory (normally target/release/bundle).");
                }
            }
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(error) => {
            eprintln!(
                "Could not start {}: {error}. Check the prerequisites in docs/development.md.",
                step.program
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(args: &[&str], os: &str) -> Result<Step, String> {
        plan(&args.iter().map(OsString::from).collect::<Vec<_>>(), os)
    }

    #[test]
    fn fast_build_is_one_shot_and_uses_the_checked_in_catalog_on_all_hosts() {
        for os in ["windows", "macos", "linux"] {
            let step = task(&["build"], os).unwrap();
            assert_eq!(step.program, "cargo");
            assert_eq!(
                step.args,
                [
                    "tauri",
                    "build",
                    "--debug",
                    "--no-bundle",
                    "--config",
                    "src-tauri/tauri.fast.conf.json"
                ]
            );
            let command = step.command(Path::new("repo with spaces"));
            assert_eq!(
                command.get_current_dir(),
                Some(Path::new("repo with spaces"))
            );
            assert!(command
                .get_envs()
                .any(|(key, value)| key == "WISP_CATALOG_OFFLINE"
                    && value == Some(std::ffi::OsStr::new("1"))));
        }
    }

    #[test]
    fn release_keeps_the_existing_tauri_pipeline() {
        let step = task(&["build", "--release"], "linux").unwrap();
        assert_eq!(step.args, ["tauri", "build"]);
        assert!(!step.fast_build);
        assert!(step.command(Path::new(".")).get_envs().next().is_none());
    }

    #[test]
    fn gpui_build_and_launch_use_the_same_workspace_on_every_desktop() {
        for os in ["macos", "windows", "linux"] {
            for (name, command) in [("build-gpui", "build"), ("gpui", "run")] {
                let debug = task(&[name], os).unwrap();
                assert_eq!(debug.program, "cargo");
                assert_eq!(
                    debug.args,
                    [
                        command,
                        "--locked",
                        "--manifest-path",
                        "apps/gpui/Cargo.toml"
                    ]
                );
                assert!(!debug.fast_build);
                let release = task(&[name, "--release"], os).unwrap();
                assert_eq!(release.args.last().unwrap(), "--release");
            }
        }
        assert!(task(&["build-gpui"], "android").is_err());
    }

    #[test]
    fn retired_commands_explain_the_replacement_on_every_host() {
        for os in ["macos", "windows", "linux"] {
            for name in ["build-mac", "build-win"] {
                assert!(task(&[name], os).unwrap_err().contains("build-gpui"));
            }
        }
    }

    #[test]
    fn rejects_misspelled_or_conflicting_build_options() {
        for name in ["build", "build-gpui", "gpui"] {
            for args in [
                vec![name, "--relase"],
                vec![name, "--release", "--debug"],
                vec![name, "--release", "--release"],
            ] {
                assert!(task(&args, "windows").is_err());
            }
        }
    }

    #[test]
    fn dev_preserves_tauri_options_without_building_the_cli() {
        let step = task(&["dev", "--no-watch"], "linux").unwrap();
        assert_eq!(step.args, ["tauri", "dev", "--no-watch"]);
        assert!(!step.fast_build);
    }

    #[test]
    fn headless_commands_keep_arguments_and_callers_working_directory() {
        for args in [
            vec![],
            vec!["run", "--output", "jsonl", "带有空格的 prompt"],
            vec!["eval"],
            vec!["rpc"],
            vec!["login", "chatgpt"],
        ] {
            let step = task(&args, "linux").unwrap();
            assert_eq!(step.program, "cargo");
            assert!(step.args.contains(&"wisp-cli".into()));
            assert_eq!(&step.args[9..], args);
            assert!(!step.repository_cwd);
            assert!(!step.fast_build);
            assert!(step.command(Path::new("repo")).get_current_dir().is_none());
        }
    }
}
