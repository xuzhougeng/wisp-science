use async_trait::async_trait;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{io::AsyncWriteExt, sync::Mutex};

const GIT_AUTHOR_NAME: &str = "Wisp Science Agent";
const GIT_AUTHOR_EMAIL: &str = "wisp-agent@localhost";

#[derive(Debug, Clone)]
pub(crate) struct GitCommandOutput {
    pub(crate) success: bool,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

#[async_trait]
pub(crate) trait GitCommandRunner: Send + Sync {
    async fn run(
        &self,
        cwd: &Path,
        args: Vec<OsString>,
        stdin: Option<Vec<u8>>,
    ) -> anyhow::Result<GitCommandOutput>;
}

#[derive(Debug, Default)]
pub(crate) struct ProcessGitCommandRunner;

#[async_trait]
impl GitCommandRunner for ProcessGitCommandRunner {
    async fn run(
        &self,
        cwd: &Path,
        args: Vec<OsString>,
        stdin: Option<Vec<u8>>,
    ) -> anyhow::Result<GitCommandOutput> {
        let mut command = tokio::process::Command::new("git");
        command
            .args(args)
            .current_dir(cwd)
            .kill_on_drop(true)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        if let Some(stdin) = stdin {
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("Git stdin was unavailable"))?;
            pipe.write_all(&stdin).await?;
        }
        let output = child.wait_with_output().await?;
        Ok(GitCommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[derive(Debug, Clone)]
struct GitProject {
    repo_root: PathBuf,
    project_relative: PathBuf,
    head: String,
}

#[derive(Debug, Clone)]
pub(crate) struct IsolatedWorkspace {
    pub(crate) project_root: PathBuf,
    repo_root: PathBuf,
    worktree_root: PathBuf,
    project_relative: PathBuf,
    branch: String,
    base_commit: String,
}

impl IsolatedWorkspace {
    #[cfg(test)]
    fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IsolationDisposition {
    NoChanges,
    Applied { commit: String },
    Preserved { reason: String },
    Rejected { reason: String },
}

#[derive(Debug, Clone)]
pub(crate) struct IsolationResult {
    pub(crate) changed_files: Vec<String>,
    pub(crate) patch: Vec<u8>,
    pub(crate) disposition: IsolationDisposition,
    pub(crate) cleanup_warning: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum IsolationFinish {
    Merge,
    Preserve { reason: String },
}

#[derive(Clone)]
pub(crate) struct GitWorktreeIsolation {
    runner: Arc<dyn GitCommandRunner>,
    temp_root: PathBuf,
    repo_lock: Arc<Mutex<()>>,
}

impl GitWorktreeIsolation {
    pub(crate) fn new(temp_root: PathBuf) -> Self {
        Self::with_runner(Arc::new(ProcessGitCommandRunner), temp_root)
    }

    pub(crate) fn with_runner(runner: Arc<dyn GitCommandRunner>, temp_root: PathBuf) -> Self {
        Self {
            runner,
            temp_root,
            repo_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) async fn available(&self, project_root: &Path) -> bool {
        self.probe(project_root).await.is_ok()
    }

    pub(crate) async fn create(&self, project_root: &Path) -> anyhow::Result<IsolatedWorkspace> {
        let _repo = self.repo_lock.lock().await;
        let project = self.probe(project_root).await?;
        tokio::fs::create_dir_all(&self.temp_root).await?;
        let token = uuid::Uuid::new_v4().to_string();
        let branch = format!("wisp-agent/{token}");
        let worktree_root = self.temp_root.join(&token);
        let args = worktree_add_args(&branch, &worktree_root, &project.head);
        let added = self.runner.run(&project.repo_root, args, None).await?;
        if !added.success {
            anyhow::bail!(
                "Git could not create the isolated worktree: {}",
                stderr_text(&added)
            );
        }
        let isolated_project_root = worktree_root.join(&project.project_relative);
        if !isolated_project_root.is_dir() {
            let partial = IsolatedWorkspace {
                project_root: isolated_project_root,
                repo_root: project.repo_root,
                worktree_root,
                project_relative: project.project_relative,
                branch,
                base_commit: project.head,
            };
            drop(_repo);
            let _ = self.cleanup(&partial).await;
            anyhow::bail!("Git created a worktree without the project directory");
        }
        Ok(IsolatedWorkspace {
            project_root: isolated_project_root,
            repo_root: project.repo_root,
            worktree_root,
            project_relative: project.project_relative,
            branch,
            base_commit: project.head,
        })
    }

    pub(crate) async fn finish(
        &self,
        workspace: &IsolatedWorkspace,
        finish: IsolationFinish,
    ) -> anyhow::Result<IsolationResult> {
        let result = self.finish_inner(workspace, finish).await;
        let cleanup_warning = self
            .cleanup(workspace)
            .await
            .err()
            .map(|error| error.to_string());
        result.map(|mut result| {
            result.cleanup_warning = cleanup_warning;
            result
        })
    }

    pub(crate) async fn abort(&self, workspace: &IsolatedWorkspace) -> anyhow::Result<()> {
        self.cleanup(workspace).await
    }

    async fn probe(&self, project_root: &Path) -> anyhow::Result<GitProject> {
        if !project_root.is_dir() {
            anyhow::bail!("the project directory does not exist");
        }
        let version = self
            .runner
            .run(project_root, os_args(["--version"]), None)
            .await?;
        if !version.success {
            anyhow::bail!("Git is unavailable: {}", stderr_text(&version));
        }
        let top = self
            .run_checked(project_root, os_args(["rev-parse", "--show-toplevel"]))
            .await?;
        let repo_root = PathBuf::from(stdout_line(&top)?);
        let repo_root = std::fs::canonicalize(repo_root)?;
        let project_root = std::fs::canonicalize(project_root)?;
        let project_relative = project_root
            .strip_prefix(&repo_root)
            .map_err(|_| anyhow::anyhow!("the project is outside the detected Git repository"))?
            .to_path_buf();
        let pathspec = project_pathspec(&project_relative);
        let status = self
            .run_checked(
                &repo_root,
                vec![
                    "status".into(),
                    "--porcelain=v1".into(),
                    "-z".into(),
                    "--untracked-files=all".into(),
                    "--".into(),
                    pathspec,
                ],
            )
            .await?;
        if !status.stdout.is_empty() {
            anyhow::bail!(
                "isolated execution requires a clean project checkout; commit or stash current project changes first"
            );
        }
        let head = self
            .run_checked(&repo_root, os_args(["rev-parse", "HEAD"]))
            .await?;
        Ok(GitProject {
            repo_root,
            project_relative,
            head: stdout_line(&head)?,
        })
    }

    async fn finish_inner(
        &self,
        workspace: &IsolatedWorkspace,
        finish: IsolationFinish,
    ) -> anyhow::Result<IsolationResult> {
        let pathspec = project_pathspec(&workspace.project_relative);
        // A child may have created its own commits. Rebuild an isolated index
        // from the approved base so the host captures one bounded, project-only
        // patch without trusting or retaining the child's branch history.
        self.run_checked(
            &workspace.worktree_root,
            vec!["read-tree".into(), workspace.base_commit.clone().into()],
        )
        .await?;
        self.run_checked(
            &workspace.worktree_root,
            vec!["add".into(), "--all".into(), "--".into(), pathspec.clone()],
        )
        .await?;
        let names = self
            .run_checked(
                &workspace.worktree_root,
                vec![
                    "diff".into(),
                    "--cached".into(),
                    "--name-only".into(),
                    "-z".into(),
                    workspace.base_commit.clone().into(),
                    "--".into(),
                    pathspec.clone(),
                ],
            )
            .await?;
        let changed_files = nul_paths(&names.stdout);
        if changed_files.is_empty() {
            return Ok(IsolationResult {
                changed_files,
                patch: vec![],
                disposition: IsolationDisposition::NoChanges,
                cleanup_warning: None,
            });
        }
        let patch = self
            .run_checked(
                &workspace.worktree_root,
                vec![
                    "diff".into(),
                    "--cached".into(),
                    "--binary".into(),
                    "--full-index".into(),
                    "--no-ext-diff".into(),
                    workspace.base_commit.clone().into(),
                    "--".into(),
                    pathspec,
                ],
            )
            .await?
            .stdout;
        if let IsolationFinish::Preserve { reason } = finish {
            return Ok(IsolationResult {
                changed_files,
                patch,
                disposition: IsolationDisposition::Preserved { reason },
                cleanup_warning: None,
            });
        }

        let _repo = self.repo_lock.lock().await;
        let disposition = match self.merge_patch(workspace, &patch).await {
            Ok(commit) => IsolationDisposition::Applied { commit },
            Err(error) => IsolationDisposition::Rejected {
                reason: error.to_string(),
            },
        };
        Ok(IsolationResult {
            changed_files,
            patch,
            disposition,
            cleanup_warning: None,
        })
    }

    async fn merge_patch(
        &self,
        workspace: &IsolatedWorkspace,
        patch: &[u8],
    ) -> anyhow::Result<String> {
        let tree = stdout_line(
            &self
                .run_checked(&workspace.worktree_root, os_args(["write-tree"]))
                .await?,
        )?;
        let hooks_path = self.temp_root.join("no-git-hooks");
        let commit = self
            .runner
            .run(
                &workspace.worktree_root,
                git_identity_args(
                    &hooks_path,
                    [
                        OsString::from("commit-tree"),
                        OsString::from(&tree),
                        OsString::from("-p"),
                        OsString::from(&workspace.base_commit),
                    ],
                ),
                Some(b"wisp(agent): apply isolated task\n".to_vec()),
            )
            .await?;
        if !commit.success {
            anyhow::bail!(
                "Git could not capture the isolated patch as a temporary commit: {}",
                stderr_text(&commit)
            );
        }
        let child_commit = stdout_line(&commit)?;
        let preflight = self
            .runner
            .run(
                &workspace.repo_root,
                os_args(["apply", "--check", "--whitespace=nowarn"]),
                Some(patch.to_vec()),
            )
            .await?;
        if !preflight.success {
            anyhow::bail!(
                "automatic merge was rejected by Git conflict preflight: {}",
                stderr_text(&preflight)
            );
        }
        if self
            .git_ref(&workspace.repo_root, "CHERRY_PICK_HEAD")
            .await?
            .is_some()
        {
            anyhow::bail!("automatic merge was rejected because another cherry-pick is active");
        }

        let cherry_pick = self
            .runner
            .run(
                &workspace.repo_root,
                git_identity_args(
                    &hooks_path,
                    [
                        OsString::from("cherry-pick"),
                        OsString::from("--no-gpg-sign"),
                        OsString::from(&child_commit),
                    ],
                ),
                None,
            )
            .await;
        match cherry_pick {
            Ok(output) if output.success => {}
            result => {
                let diagnostic = match &result {
                    Ok(output) => stderr_text(output),
                    Err(error) => error.to_string(),
                };
                let cleanup = self
                    .abort_cherry_pick_if_owned(workspace, &child_commit)
                    .await;
                anyhow::bail!("automatic cherry-pick failed{cleanup}: {diagnostic}");
            }
        }
        stdout_line(
            &self
                .run_checked(&workspace.repo_root, os_args(["rev-parse", "HEAD"]))
                .await?,
        )
    }

    async fn abort_cherry_pick_if_owned(
        &self,
        workspace: &IsolatedWorkspace,
        child_commit: &str,
    ) -> String {
        match self.git_ref(&workspace.repo_root, "CHERRY_PICK_HEAD").await {
            Ok(Some(head)) if head == child_commit => {
                match self
                    .runner
                    .run(
                        &workspace.repo_root,
                        os_args(["cherry-pick", "--abort"]),
                        None,
                    )
                    .await
                {
                    Ok(output) if output.success => "; Wisp aborted its partial cherry-pick".into(),
                    Ok(output) => format!(
                        "; Wisp could not abort its partial cherry-pick ({})",
                        stderr_text(&output)
                    ),
                    Err(error) => {
                        format!("; Wisp could not abort its partial cherry-pick ({error})")
                    }
                }
            }
            _ => String::new(),
        }
    }

    async fn git_ref(&self, cwd: &Path, name: &str) -> anyhow::Result<Option<String>> {
        let output = self
            .runner
            .run(
                cwd,
                vec![
                    "rev-parse".into(),
                    "--verify".into(),
                    "--quiet".into(),
                    name.into(),
                ],
                None,
            )
            .await?;
        if output.success {
            Ok(Some(stdout_line(&output)?))
        } else {
            Ok(None)
        }
    }

    async fn cleanup(&self, workspace: &IsolatedWorkspace) -> anyhow::Result<()> {
        let _repo = self.repo_lock.lock().await;
        let removed = self
            .runner
            .run(
                &workspace.repo_root,
                vec![
                    "worktree".into(),
                    "remove".into(),
                    "--force".into(),
                    workspace.worktree_root.as_os_str().to_owned(),
                ],
                None,
            )
            .await?;
        if !removed.success && workspace.worktree_root.exists() {
            let temp_root = std::fs::canonicalize(&self.temp_root)?;
            let target = std::fs::canonicalize(&workspace.worktree_root)?;
            if target.parent() != Some(temp_root.as_path()) {
                anyhow::bail!("refusing to clean an unexpected worktree path");
            }
            tokio::fs::remove_dir_all(&target).await?;
            let _ = self
                .runner
                .run(&workspace.repo_root, os_args(["worktree", "prune"]), None)
                .await;
        }
        let branch = self
            .runner
            .run(
                &workspace.repo_root,
                vec![
                    "branch".into(),
                    "-D".into(),
                    workspace.branch.clone().into(),
                ],
                None,
            )
            .await?;
        if !branch.success {
            let reference = format!("refs/heads/{}", workspace.branch);
            if self
                .runner
                .run(
                    &workspace.repo_root,
                    vec![
                        "show-ref".into(),
                        "--verify".into(),
                        "--quiet".into(),
                        reference.into(),
                    ],
                    None,
                )
                .await?
                .success
            {
                anyhow::bail!(
                    "isolated worktree was removed but its temporary branch could not be deleted: {}",
                    stderr_text(&branch)
                );
            }
        }
        Ok(())
    }

    async fn run_checked(
        &self,
        cwd: &Path,
        args: Vec<OsString>,
    ) -> anyhow::Result<GitCommandOutput> {
        let output = self.runner.run(cwd, args, None).await?;
        if !output.success {
            anyhow::bail!("Git command failed: {}", stderr_text(&output));
        }
        Ok(output)
    }
}

pub(crate) async fn git_worktree_available(project_root: &Path) -> bool {
    GitWorktreeIsolation::new(std::env::temp_dir().join("wisp-agent-worktrees-probe"))
        .available(project_root)
        .await
}

fn worktree_add_args(branch: &str, path: &Path, head: &str) -> Vec<OsString> {
    vec![
        "worktree".into(),
        "add".into(),
        "-b".into(),
        branch.into(),
        path.as_os_str().to_owned(),
        head.into(),
    ]
}

fn git_identity_args<const N: usize>(hooks_path: &Path, command: [OsString; N]) -> Vec<OsString> {
    let mut args = vec![
        "-c".into(),
        format!("user.name={GIT_AUTHOR_NAME}").into(),
        "-c".into(),
        format!("user.email={GIT_AUTHOR_EMAIL}").into(),
        "-c".into(),
        "commit.gpgSign=false".into(),
        "-c".into(),
        format!("core.hooksPath={}", hooks_path.to_string_lossy()).into(),
    ];
    args.extend(command);
    args
}

fn project_pathspec(relative: &Path) -> OsString {
    if relative.as_os_str().is_empty() {
        OsString::from(".")
    } else {
        relative.as_os_str().to_owned()
    }
}

fn os_args<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}

fn stdout_line(output: &GitCommandOutput) -> anyhow::Result<String> {
    let value = std::str::from_utf8(&output.stdout)?.trim();
    if value.is_empty() {
        anyhow::bail!("Git returned an empty value");
    }
    Ok(value.to_string())
}

fn stderr_text(output: &GitCommandOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        "no diagnostic output".into()
    } else {
        stderr
    }
}

fn nul_paths(value: &[u8]) -> Vec<String> {
    value
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, fs, process::Command, sync::Mutex as StdMutex};

    fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap()
    }

    fn test_repo(label: &str) -> (PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("wisp isolation {label} {}", uuid::Uuid::new_v4()));
        let repo = base.join("Project With Spaces");
        fs::create_dir_all(&repo).unwrap();
        assert!(git(&repo, &["init"]).status.success());
        fs::write(repo.join("a.txt"), "base a\n").unwrap();
        fs::write(repo.join("b.txt"), "base b\n").unwrap();
        assert!(git(&repo, &["add", "."]).status.success());
        assert!(git(
            &repo,
            &[
                "-c",
                "user.name=Wisp Test",
                "-c",
                "user.email=wisp-test@localhost",
                "commit",
                "-m",
                "base",
            ],
        )
        .status
        .success());
        let worktrees = base.join("Agent Worktrees With Spaces");
        (repo, worktrees)
    }

    #[tokio::test]
    async fn independent_changes_merge_and_every_worktree_is_cleaned() {
        let (repo, worktrees) = test_repo("independent");
        let isolation = GitWorktreeIsolation::new(worktrees.clone());
        let first = isolation.create(&repo).await.unwrap();
        let second = isolation.create(&repo).await.unwrap();
        let first_root = first.worktree_root().to_path_buf();
        let second_root = second.worktree_root().to_path_buf();
        fs::write(first.project_root.join("a.txt"), "first\n").unwrap();
        assert!(git(
            &first.project_root,
            &[
                "-c",
                "user.name=Child Agent",
                "-c",
                "user.email=child@localhost",
                "commit",
                "-am",
                "child-created commit",
            ],
        )
        .status
        .success());
        fs::write(second.project_root.join("new result.txt"), "second\n").unwrap();

        let first_result = isolation
            .finish(&first, IsolationFinish::Merge)
            .await
            .unwrap();
        let second_result = isolation
            .finish(&second, IsolationFinish::Merge)
            .await
            .unwrap();

        assert!(matches!(
            first_result.disposition,
            IsolationDisposition::Applied { .. }
        ));
        assert!(matches!(
            second_result.disposition,
            IsolationDisposition::Applied { .. }
        ));
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "first\n");
        assert_eq!(
            fs::read_to_string(repo.join("new result.txt")).unwrap(),
            "second\n"
        );
        assert!(!first_root.exists());
        assert!(!second_root.exists());
        assert!(!String::from_utf8_lossy(
            &git(&repo, &["branch", "--list", "wisp-agent/*"]).stdout
        )
        .contains("wisp-agent/"));
        let _ = fs::remove_dir_all(repo.parent().unwrap());
    }

    #[tokio::test]
    async fn conflicting_change_is_preserved_without_touching_the_merged_file() {
        let (repo, worktrees) = test_repo("conflict");
        let isolation = GitWorktreeIsolation::new(worktrees);
        let first = isolation.create(&repo).await.unwrap();
        let second = isolation.create(&repo).await.unwrap();
        let second_root = second.worktree_root().to_path_buf();
        fs::write(first.project_root.join("a.txt"), "winner\n").unwrap();
        fs::write(second.project_root.join("a.txt"), "conflict\n").unwrap();
        isolation
            .finish(&first, IsolationFinish::Merge)
            .await
            .unwrap();
        let result = isolation
            .finish(&second, IsolationFinish::Merge)
            .await
            .unwrap();

        assert!(matches!(
            result.disposition,
            IsolationDisposition::Rejected { .. }
        ));
        assert!(!result.patch.is_empty());
        assert_eq!(result.changed_files, ["a.txt"]);
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "winner\n");
        assert!(!second_root.exists());
        assert!(git(&repo, &["status", "--porcelain"]).stdout.is_empty());
        let _ = fs::remove_dir_all(repo.parent().unwrap());
    }

    #[tokio::test]
    async fn rejected_merge_never_aborts_the_users_existing_cherry_pick() {
        let (repo, worktrees) = test_repo("existing-cherry-pick");
        let isolation = GitWorktreeIsolation::new(worktrees);
        let workspace = isolation.create(&repo).await.unwrap();
        fs::write(workspace.project_root.join("b.txt"), "agent\n").unwrap();
        let main_branch =
            String::from_utf8_lossy(&git(&repo, &["branch", "--show-current"]).stdout)
                .trim()
                .to_string();

        assert!(git(&repo, &["checkout", "-b", "user-operation"])
            .status
            .success());
        fs::write(repo.join("a.txt"), "user operation\n").unwrap();
        assert!(git(
            &repo,
            &[
                "-c",
                "user.name=Wisp Test",
                "-c",
                "user.email=wisp-test@localhost",
                "commit",
                "-am",
                "user operation",
            ],
        )
        .status
        .success());
        assert!(git(&repo, &["checkout", &main_branch]).status.success());
        fs::write(repo.join("a.txt"), "main change\n").unwrap();
        assert!(git(
            &repo,
            &[
                "-c",
                "user.name=Wisp Test",
                "-c",
                "user.email=wisp-test@localhost",
                "commit",
                "-am",
                "main change",
            ],
        )
        .status
        .success());
        assert!(!git(
            &repo,
            &[
                "-c",
                "user.name=Wisp Test",
                "-c",
                "user.email=wisp-test@localhost",
                "cherry-pick",
                "user-operation",
            ],
        )
        .status
        .success());
        let user_cherry_pick = String::from_utf8_lossy(
            &git(&repo, &["rev-parse", "--verify", "CHERRY_PICK_HEAD"]).stdout,
        )
        .trim()
        .to_string();

        let result = isolation
            .finish(&workspace, IsolationFinish::Merge)
            .await
            .unwrap();

        assert!(matches!(
            result.disposition,
            IsolationDisposition::Rejected { .. }
        ));
        assert!(!result.patch.is_empty());
        assert_eq!(
            String::from_utf8_lossy(
                &git(&repo, &["rev-parse", "--verify", "CHERRY_PICK_HEAD"]).stdout
            )
            .trim(),
            user_cherry_pick
        );
        assert!(git(&repo, &["cherry-pick", "--abort"]).status.success());
        let _ = fs::remove_dir_all(repo.parent().unwrap());
    }

    #[tokio::test]
    async fn cancelled_and_failed_children_keep_patches_and_clean_up() {
        let (repo, worktrees) = test_repo("preserve");
        let isolation = GitWorktreeIsolation::new(worktrees);
        for reason in ["child was cancelled", "child failed"] {
            let workspace = isolation.create(&repo).await.unwrap();
            let root = workspace.worktree_root().to_path_buf();
            fs::write(workspace.project_root.join("a.txt"), format!("{reason}\n")).unwrap();
            let result = isolation
                .finish(
                    &workspace,
                    IsolationFinish::Preserve {
                        reason: reason.into(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                result.disposition,
                IsolationDisposition::Preserved {
                    reason: reason.into()
                }
            );
            assert!(!result.patch.is_empty());
            assert!(!root.exists());
            assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "base a\n");
        }
        let _ = fs::remove_dir_all(repo.parent().unwrap());
    }

    #[derive(Default)]
    struct FakeRunner {
        outputs: StdMutex<VecDeque<anyhow::Result<GitCommandOutput>>>,
        calls: StdMutex<Vec<(PathBuf, Vec<OsString>)>>,
    }

    #[async_trait]
    impl GitCommandRunner for FakeRunner {
        async fn run(
            &self,
            cwd: &Path,
            args: Vec<OsString>,
            _stdin: Option<Vec<u8>>,
        ) -> anyhow::Result<GitCommandOutput> {
            self.calls.lock().unwrap().push((cwd.to_path_buf(), args));
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| anyhow::bail!("unexpected fake Git command"))
        }
    }

    #[tokio::test]
    async fn missing_git_fails_closed_through_the_injected_runner() {
        let root = std::env::temp_dir().join(format!("wisp-missing-git-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let fake = Arc::new(FakeRunner::default());
        fake.outputs
            .lock()
            .unwrap()
            .push_back(Err(anyhow::anyhow!("git executable was not found")));
        let isolation = GitWorktreeIsolation::with_runner(fake.clone(), root.join("worktrees"));
        assert!(!isolation.available(&root).await);
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, os_args(["--version"]));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn non_git_and_dirty_projects_do_not_advertise_isolation() {
        let plain = std::env::temp_dir().join(format!("wisp-non-git-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&plain).unwrap();
        let isolation = GitWorktreeIsolation::new(plain.join("worktrees"));
        assert!(!isolation.available(&plain).await);
        let _ = fs::remove_dir_all(&plain);

        let (repo, worktrees) = test_repo("dirty");
        fs::write(repo.join("a.txt"), "uncommitted user change\n").unwrap();
        let isolation = GitWorktreeIsolation::new(worktrees);
        assert!(!isolation.available(&repo).await);
        assert!(isolation
            .create(&repo)
            .await
            .unwrap_err()
            .to_string()
            .contains("clean"));
        let _ = fs::remove_dir_all(repo.parent().unwrap());
    }

    #[test]
    fn worktree_paths_are_single_arguments_on_windows_and_macos() {
        for path in [
            Path::new(r"C:\Users\Researcher Name\Wisp Worktree"),
            Path::new("/Users/Researcher Name/Wisp Worktree"),
        ] {
            let args = worktree_add_args("wisp-agent/id", path, "abc123");
            assert_eq!(args.len(), 6);
            assert_eq!(args[4], path.as_os_str());
        }
    }
}
