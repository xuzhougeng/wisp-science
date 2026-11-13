use std::process::Command;

#[test]
fn help_works_without_any_build_sdks() {
    let output = Command::new(env!("CARGO_BIN_EXE_wisp-dev"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for command in ["cargo run build", "cargo run build-gpui", "cargo run gpui"] {
        assert!(help.contains(command));
    }
}

#[test]
fn retired_native_builds_explain_the_replacement_without_starting_a_build() {
    for command in ["build-mac", "build-win"] {
        let output = Command::new(env!("CARGO_BIN_EXE_wisp-dev"))
            .env("CARGO", "this-build-tool-must-not-be-started")
            .arg(command)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("build-gpui"));
        assert!(error.contains("retired"));
    }
}

#[cfg(unix)]
#[test]
fn child_exit_status_arguments_stdout_and_caller_directory_are_preserved() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("wisp launcher test {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let fake = root.join("fake cargo");
    std::fs::write(&fake, "#!/bin/sh\npwd > cwd.txt\nprintf '%s\\n' \"$@\" > args.txt\nprintf 'jsonl output\\n'\nexit 37\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_wisp-dev"))
        .env("CARGO", &fake)
        .current_dir(&root)
        .args(["run", "--output", "jsonl", "prompt with spaces; $HOME"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(output.stdout, b"jsonl output\n");
    assert!(output.stderr.is_empty());
    assert!(std::fs::read_to_string(root.join("args.txt"))
        .unwrap()
        .ends_with("--\nrun\n--output\njsonl\nprompt with spaces; $HOME\n"));
    assert_eq!(
        std::fs::read_to_string(root.join("cwd.txt"))
            .unwrap()
            .trim(),
        root.canonicalize().unwrap().to_str().unwrap()
    );
}
