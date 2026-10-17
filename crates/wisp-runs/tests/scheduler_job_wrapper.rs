//! Behavior of the bundled `skills/remote-compute-ssh/scripts/scheduler_job.sh`
//! wrapper against fake scheduler clients on PATH. No real cluster is used.
#![cfg(unix)]

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const WRAPPER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../skills/remote-compute-ssh/scripts/scheduler_job.sh"
);

/// Everything the wrapper, its generated job, and the fakes run. PATH holds
/// only links to these, so a real `sbatch`, `qsub`, or `bsub` installed on the
/// test machine can never be reached.
const SYSTEM_TOOLS: &[&str] = &[
    "awk", "bash", "cat", "chmod", "cp", "date", "dirname", "grep", "head", "mkdir", "mv", "sed",
    "sh", "sleep", "tail", "tr",
];

fn link_system_tools(dir: &Path) {
    let search = std::env::var_os("PATH").unwrap_or_default();
    let find = |tool: &str| {
        std::env::split_paths(&search)
            .map(|candidate| candidate.join(tool))
            .find(|candidate| candidate.is_file())
    };
    for tool in SYSTEM_TOOLS {
        let path = find(tool).unwrap_or_else(|| panic!("{tool} not found on PATH"));
        std::os::unix::fs::symlink(path, dir.join(tool)).unwrap();
    }
    // Optional: without stdbuf the wrapper runs the client directly.
    if let Some(path) = find("stdbuf") {
        std::os::unix::fs::symlink(path, dir.join("stdbuf")).unwrap();
    }
}

/// A temporary login node: `bin/` holds fake scheduler clients, `sys/` links
/// the basic tools, `inputs/` is the Run's working directory, and fakes record
/// their arguments in the root.
struct Cluster {
    root: PathBuf,
    bin: PathBuf,
    sys: PathBuf,
    work: PathBuf,
}

struct Finished {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl Cluster {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("wisp_scheduler_{name}_{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        let sys = root.join("sys");
        let work = root.join("inputs");
        for dir in [&bin, &sys, &work] {
            fs::create_dir_all(dir).unwrap();
        }
        link_system_tools(&sys);
        Self {
            root,
            bin,
            sys,
            work,
        }
    }

    fn tool(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn job(&self, name: &str, body: &str) {
        fs::write(self.work.join(name), body).unwrap();
    }

    fn recorded(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap_or_default()
    }

    fn spawn(&self, args: &[&str]) -> Child {
        Command::new("bash")
            .arg(WRAPPER)
            .args(args)
            .current_dir(&self.work)
            // No inherited SGE_ROOT, SLURM_*, or other scheduler state.
            .env_clear()
            .env(
                "PATH",
                format!("{}:{}", self.bin.display(), self.sys.display()),
            )
            .env("FAKE_LOG", &self.root)
            .env("WISP_SCHEDULER_SETTLE_SECS", "0")
            .stdin(Stdio::null())
            .stdout(File::create(self.root.join("stdout")).unwrap())
            .stderr(File::create(self.root.join("stderr")).unwrap())
            // Its own process group, like the Run supervisor gives it.
            .process_group(0)
            .spawn()
            .unwrap()
    }

    fn finish(&self, child: &mut Child) -> Finished {
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("wrapper did not exit; stderr:\n{}", self.recorded("stderr"));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        Finished {
            status,
            stdout: self.recorded("stdout"),
            stderr: self.recorded("stderr"),
        }
    }

    fn run(&self, args: &[&str]) -> Finished {
        let mut child = self.spawn(args);
        self.finish(&mut child)
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Fake clients record their arguments and the submitted file, then (when
/// blocking) run it in place, as the scheduler would on a compute node.
const RECORD: &str = r#"printf '%s\n' "$@" > "$FAKE_LOG/client.args"
eval "last=\${$#}"
cp "$last" "$FAKE_LOG/submitted.sh""#;

#[test]
fn slurm_job_keeps_directives_runs_in_the_workdir_and_returns_its_exit_code() {
    let cluster = Cluster::new("slurm");
    cluster.tool(
        "sbatch",
        &format!("{RECORD}\necho 4242\nSLURM_JOB_ID=4242 bash \"$last\""),
    );
    cluster.job(
        "align.sbatch",
        "#!/bin/bash\n# align reads\n#SBATCH -t 00:10:00\n#SBATCH -o job.out\n\
         echo aligned > result.txt\necho progress-line > job.out\nexit 3\n",
    );

    let done = cluster.run(&["--follow", "job.out", "align.sbatch"]);

    assert_eq!(done.status.code(), Some(3), "stderr:\n{}", done.stderr);
    let args = cluster.recorded("client.args");
    assert!(args.lines().any(|arg| arg == "--wait"), "{args}");
    assert!(args.lines().any(|arg| arg == "--parsable"), "{args}");
    let submitted = cluster.recorded("submitted.sh");
    let cd = submitted
        .find("\ncd ")
        .expect("submitted file enters the workdir");
    for directive in ["#SBATCH -t 00:10:00", "#SBATCH -o job.out"] {
        let at = submitted.find(directive).expect(directive);
        assert!(at < cd, "{directive} must precede the first command");
    }
    assert!(
        !submitted.contains("echo aligned"),
        "the job body stays in its own file"
    );
    assert_eq!(
        fs::read_to_string(cluster.work.join("result.txt")).unwrap(),
        "aligned\n"
    );
    assert!(
        done.stdout.contains("progress-line"),
        "--follow streams the log"
    );
    assert!(done.stderr.contains("submitted slurm job 4242"));
}

#[test]
fn termination_cancels_the_queued_slurm_job() {
    let cluster = Cluster::new("cancel");
    cluster.tool("sbatch", &format!("{RECORD}\necho 4242\nexec sleep 30"));
    cluster.tool(
        "scancel",
        r#"printf '%s\n' "$@" >> "$FAKE_LOG/scancel.args""#,
    );
    cluster.job(
        "long.sbatch",
        "#!/bin/bash\n#SBATCH -t 1:00:00\nsleep 600\n",
    );

    let mut child = cluster.spawn(&["long.sbatch"]);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !cluster
        .recorded("stderr")
        .contains("submitted slurm job 4242")
    {
        assert!(Instant::now() < deadline, "job id never reported");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The Run supervisor's cancel and timeout both TERM the whole group.
    // `-s TERM --` is the POSIX form; dash rejects `kill -TERM -- -pgid`.
    let group = child.id().to_string();
    let sent = Command::new("sh")
        .args(["-c", r#"kill -s TERM -- "-$1""#, "sh", &group])
        .status()
        .unwrap();
    assert!(sent.success());
    let done = cluster.finish(&mut child);

    assert_eq!(done.status.code(), Some(143), "stderr:\n{}", done.stderr);
    assert_eq!(cluster.recorded("scancel.args").trim(), "4242");
    assert!(done.stderr.contains("cancelling slurm job 4242"));
}

#[test]
fn array_tasks_report_the_highest_task_exit_code() {
    let cluster = Cluster::new("array");
    cluster.tool(
        "sbatch",
        &format!(
            "{RECORD}\necho 51\n\
             SLURM_ARRAY_TASK_ID=1 bash \"$last\"\nSLURM_ARRAY_TASK_ID=2 bash \"$last\"\nexit 0"
        ),
    );
    cluster.job(
        "array.sbatch",
        "#!/bin/bash\n#SBATCH --array=1-2\n[ \"$SLURM_ARRAY_TASK_ID\" = 2 ] && exit 4\nexit 0\n",
    );

    let done = cluster.run(&["array.sbatch"]);

    assert_eq!(done.status.code(), Some(4), "stderr:\n{}", done.stderr);
}

fn pbs_qsub(body: &str) -> String {
    format!(
        "if [ \"$1\" = --version ]; then echo 'pbs_version = 2022.1.0'; exit 0; fi\n{RECORD}\n{body}"
    )
}

#[test]
fn pbs_exit_code_comes_from_the_job_marker_not_qsub() {
    let cluster = Cluster::new("pbs");
    // `qsub -W block=true` may return 0 whatever the job did.
    cluster.tool(
        "qsub",
        &pbs_qsub("echo 77.pbs01\nPBS_JOBID=77.pbs01 bash \"$last\"\nexit 0"),
    );
    cluster.job(
        "fit.pbs",
        "#!/bin/bash\n#PBS -l walltime=01:00:00\nexit 5\n",
    );

    let done = cluster.run(&["fit.pbs"]);

    assert_eq!(done.status.code(), Some(5), "stderr:\n{}", done.stderr);
    let args = cluster.recorded("client.args");
    assert!(args.contains("-W\nblock=true"), "{args}");
    assert!(cluster
        .recorded("submitted.sh")
        .contains("#PBS -l walltime=01:00:00"));
    assert!(done.stderr.contains("submitted pbs job 77.pbs01"));
}

#[test]
fn pbs_job_killed_before_its_script_finished_fails() {
    let cluster = Cluster::new("pbs_killed");
    cluster.tool("qsub", &pbs_qsub("echo 78.pbs01\nexit 0"));
    cluster.job("fit.pbs", "#!/bin/bash\nexit 0\n");

    let done = cluster.run(&["fit.pbs"]);

    assert_eq!(done.status.code(), Some(1), "stderr:\n{}", done.stderr);
    assert!(done.stderr.contains("ended without finishing its script"));
}

#[test]
fn lsf_job_is_piped_to_bsub_and_gets_an_output_file() {
    let cluster = Cluster::new("lsf");
    cluster.tool(
        "bsub",
        "printf '%s\\n' \"$@\" > \"$FAKE_LOG/client.args\"\n\
         cat > \"$FAKE_LOG/submitted.sh\"\n\
         echo 'Job <981> is submitted to default queue <normal>.'\n\
         echo '<<Waiting for dispatch ...>>' >&2\n\
         LSB_JOBINDEX=0 bash \"$FAKE_LOG/submitted.sh\"",
    );
    cluster.job("count.lsf", "#!/bin/bash\n#BSUB -n 4\nexit 0\n");

    let done = cluster.run(&["count.lsf"]);

    assert!(done.status.success(), "stderr:\n{}", done.stderr);
    assert_eq!(cluster.recorded("client.args").trim(), "-K");
    let submitted = cluster.recorded("submitted.sh");
    assert!(submitted.contains("#BSUB -n 4"));
    // Without -o, LSF would mail the output instead of writing a file.
    assert!(submitted.contains("#BSUB -o lsf-%J.out"));
    assert!(done.stderr.contains("submitted lsf job 981"));
}

#[test]
fn grid_engine_qsub_is_told_to_wait_and_use_bash() {
    let cluster = Cluster::new("sge");
    cluster.tool(
        "qsub",
        &format!(
            "if [ \"$1\" = --version ]; then echo 'invalid option argument \"--version\"' >&2; exit 2; fi\n\
             {RECORD}\necho 5150\nSGE_TASK_ID=undefined bash \"$last\""
        ),
    );
    cluster.tool("qconf", "exit 0");
    cluster.job("qc.sge", "#!/bin/bash\n#$ -pe smp 8\nexit 0\n");

    let done = cluster.run(&["qc.sge"]);

    assert!(done.status.success(), "stderr:\n{}", done.stderr);
    let args = cluster.recorded("client.args");
    assert!(args.contains("-terse\n-sync\ny\n"), "{args}");
    let submitted = cluster.recorded("submitted.sh");
    for directive in ["#$ -S /bin/bash", "#$ -cwd", "#$ -pe smp 8"] {
        assert!(submitted.contains(directive), "missing {directive}");
    }
    assert!(done.stderr.contains("submitted sge job 5150"));
}

#[test]
fn torque_qsub_is_refused_without_submitting() {
    let cluster = Cluster::new("torque");
    cluster.tool(
        "qsub",
        &format!("if [ \"$1\" = --version ]; then echo 'Version: 6.1.3'; exit 0; fi\n{RECORD}"),
    );
    cluster.job("fit.pbs", "#!/bin/bash\nexit 0\n");

    let done = cluster.run(&["fit.pbs"]);

    assert_eq!(done.status.code(), Some(69), "stderr:\n{}", done.stderr);
    assert!(
        cluster.recorded("client.args").is_empty(),
        "nothing submitted"
    );
    assert!(done.stderr.contains("Torque"));
}

#[test]
fn a_rejected_submission_reports_the_client_error() {
    let cluster = Cluster::new("rejected");
    cluster.tool(
        "sbatch",
        "echo 'sbatch: error: invalid partition specified: gpu' >&2\nexit 1",
    );
    cluster.job("bad.sbatch", "#!/bin/bash\n#SBATCH -p gpu\nexit 0\n");

    let done = cluster.run(&["bad.sbatch"]);

    assert_eq!(done.status.code(), Some(1));
    assert!(done.stderr.contains("invalid partition specified"));
    assert!(done.stderr.contains("failed before reporting a job id"));
}
