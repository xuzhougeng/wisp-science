//! What freezing a research archive does to the project's files, shared by
//! every host: which recorded local files a draft lists and whether each
//! may be deleted, the checks on what the researcher froze, the snapshots
//! and the report written under `.wisp/research-archives/<id>/`, and the
//! permanent deletion of the files chosen for it, with a receipt per file.
//!
//! Moved unchanged from the WebView desktop's `src-tauri` so the GPUI host
//! freezes an archive by the same rules. Errors are the messages the
//! WebView shows.
use crate::Store;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use wisp_dto::{ArchiveFile, ArchiveScript, ConfirmResearchArchive, ResearchArchive};

/// What every draft says about the files it lists and its scripts.
pub const DRAFT_WARNINGS: [&str; 2] = [
    "Only recorded local files are listed. Unregistered and remote files are left untouched. / 仅列出已登记的本地文件；未登记及远程文件保持原样。",
    "Scripts document recorded operations; they have not been rerun. / 脚本整理自操作记录，未重新运行。",
];

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Marks for deletion the files the draft's model recommended, among those
/// that may be deleted at all. Every other file keeps its snapshot.
pub fn recommend_deletions(files: &mut [ArchiveFile], delete_paths: &[String]) {
    for file in files {
        if file.can_delete && delete_paths.contains(&file.path) {
            file.action = "delete".into();
        }
    }
}

/// Refuse links/junctions, traversal and external paths, including Windows ADS.
pub fn local_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains(':')
        || relative.contains('\\')
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("Only ordinary project-relative local files can be archived or cleaned".into());
    }
    let root = dunce::canonicalize(root).map_err(err)?;
    let mut target = root.clone();
    for part in path.components() {
        target.push(part);
        let meta = std::fs::symlink_metadata(&target).map_err(err)?;
        if meta.file_type().is_symlink() {
            return Err("Symbolic links cannot be archived or cleaned".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Reparse points cannot be archived or cleaned".into());
            }
        }
    }
    let real = dunce::canonicalize(&target).map_err(err)?;
    if !real.starts_with(&root) || !real.is_file() {
        return Err("File is outside the project or is not a regular file".into());
    }
    Ok(real)
}

pub fn file_hash(path: &Path) -> Result<(String, u64), String> {
    let mut file = std::fs::File::open(path).map_err(err)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hash.finalize()), size))
}

fn normalize_recorded_path(root: &Path, path: &str) -> Option<String> {
    let p = Path::new(path);
    let relative = if p.is_absolute() {
        p.strip_prefix(root).ok()?
    } else {
        p
    };
    Some(relative.to_string_lossy().replace('\\', "/"))
}

/// The recorded local files of the conversation `frame_id`, each with its
/// checksum, whether it may be deleted and why, and the default choice of a
/// snapshot.
pub async fn candidates(
    store: &Store,
    root: &Path,
    frame_id: &str,
) -> Result<Vec<ArchiveFile>, String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for recorded in store.research_archive_paths(frame_id).await.map_err(err)? {
        let Some(path) = normalize_recorded_path(root, &recorded) else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        // Archive snapshots are not candidates in later archives.
        if path.starts_with(".wisp/research-archives/") || path.starts_with(".git/") {
            continue;
        }
        let Ok(real) = local_file(root, &path) else {
            continue;
        };
        let (checksum, size_bytes) = file_hash(&real)?;
        let can_delete = !path.starts_with(".wisp/")
            && !path.starts_with("uploads/")
            && store
                .archive_path_deletable(frame_id, &recorded)
                .await
                .map_err(err)?;
        files.push(ArchiveFile {
            path,
            checksum,
            size_bytes,
            action: "snapshot".into(),
            can_delete,
            reason: if can_delete {
                "Exclusive recorded creation / 本会话创建"
            } else {
                "Input, shared evidence or uncertain ownership / 输入、共享证据或归属不明"
            }
            .into(),
            snapshot_path: None,
            cleanup_status: String::new(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn validate_content(
    title: &str,
    report: &str,
    scripts: &[ArchiveScript],
) -> Result<(), String> {
    if title.trim().is_empty()
        || title.chars().count() > 200
        || report.trim().is_empty()
        || report.len() > 512_000
        || scripts.len() > 32
    {
        return Err("Archive requires a title (up to 200 characters), a report (up to 512 KB), and at most 32 scripts.".into());
    }
    let mut names = HashSet::new();
    for script in scripts {
        if script.filename.is_empty()
            || script.filename.len() > 120
            || script.filename.contains(['/', '\\', ':'])
            || script.filename == "."
            || script.filename == ".."
            || !names.insert(script.filename.to_lowercase())
            || script.content.len() > 512_000
        {
            return Err(
                "Each script needs a unique plain filename and at most 512 KB of content.".into(),
            );
        }
    }
    Ok(())
}

/// Create directories without traversing any existing link/junction.
fn archive_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    Uuid::parse_str(id).map_err(err)?;
    let mut dir = dunce::canonicalize(root).map_err(err)?;
    for part in [".wisp", "research-archives", id] {
        dir.push(part);
        if !dir.exists() {
            std::fs::create_dir(&dir).map_err(err)?;
        }
        let meta = std::fs::symlink_metadata(&dir).map_err(err)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err("Archive directory is not an ordinary directory".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("Archive directory is a reparse point".into());
            }
        }
    }
    Ok(dir)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)
}

/// A failed snapshot must not strand large partial copies. This attempt owns
/// a newly created, flat directory; cleanup never recursively removes a tree.
struct ArchiveStaging {
    directory: PathBuf,
    report: Option<PathBuf>,
    committed: bool,
}
impl Drop for ArchiveStaging {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(&self.directory) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let _ = std::fs::remove_dir(&self.directory);
        if let Some(report) = &self.report {
            let _ = std::fs::remove_file(report);
        }
    }
}

/// Freezes the saved draft `archive` as the researcher reviewed it in
/// `input`: snapshots, scripts, the notebook and the report are written,
/// the record is frozen, and only then are the files chosen for deletion
/// removed for good, each with a receipt. Any failure before the record is
/// frozen leaves the draft open and every project file as it was.
pub async fn freeze_files(
    store: &Store,
    root: &Path,
    mut archive: ResearchArchive,
    input: ConfirmResearchArchive,
) -> Result<ResearchArchive, String> {
    validate_content(&input.title, &input.report, &input.scripts)?;
    if input.id != archive.id || archive.frozen_at.is_some() {
        return Err("Reopen the current archive review".into());
    }
    let (source, hash) = store
        .research_archive_source(&archive.frame_id)
        .await
        .map_err(err)?;
    if hash != archive.source_hash {
        return Err("The notebook changed. Generate a new archive draft before confirming.".into());
    }
    if input.files.len() != archive.files.len() {
        return Err("The file review is incomplete".into());
    }
    let mut seen = HashSet::new();
    for choice in &input.files {
        if !seen.insert(&choice.path) {
            return Err("Duplicate file choice".into());
        }
        let file = archive
            .files
            .iter_mut()
            .find(|f| f.path == choice.path)
            .ok_or("Unknown file in archive review")?;
        if !["snapshot", "reference", "delete"].contains(&choice.action.as_str()) {
            return Err("Unknown archive file action".into());
        }
        let real = local_file(root, &file.path)?;
        if file_hash(&real)? != (file.checksum.clone(), file.size_bytes) {
            return Err(format!("File changed since review: {}", file.path));
        }
        if choice.action == "delete"
            && (!file.can_delete
                || file.path.starts_with('.')
                || !store
                    .archive_path_deletable(&archive.frame_id, &file.path)
                    .await
                    .map_err(err)?)
        {
            return Err(format!(
                "File is shared, protected or no longer owned: {}",
                file.path
            ));
        }
        file.action = choice.action.clone();
        if file.action == "delete" {
            file.cleanup_status = "pending".into();
        }
    }
    archive.title = input.title;
    archive.report = input.report;
    archive.scripts = input.scripts;
    // Every freeze attempt uses a fresh directory. A failed snapshot cannot
    // overwrite a previous attempt or cause deletion before the DB commit.
    let attempt = Uuid::new_v4().to_string();
    let directory = archive_dir(root, &archive.id)?;
    let output = directory.join(&attempt);
    std::fs::create_dir(&output).map_err(err)?;
    let mut staging = ArchiveStaging {
        directory: output.clone(),
        report: None,
        committed: false,
    };
    for (index, file) in archive.files.iter_mut().enumerate() {
        if file.action != "snapshot" {
            continue;
        }
        let real = local_file(root, &file.path)?;
        let filename = format!(
            "file-{index}-{}",
            Path::new(&file.path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let dest = output.join(&filename);
        let mut input = std::fs::File::open(real).map_err(err)?;
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&dest)
            .map_err(err)?;
        std::io::copy(&mut input, &mut out).map_err(err)?;
        out.sync_all().map_err(err)?;
        if file_hash(&dest)? != (file.checksum.clone(), file.size_bytes) {
            return Err(format!("File changed while saving: {}", file.path));
        }
        file.snapshot_path = Some(format!(
            ".wisp/research-archives/{}/{attempt}/{filename}",
            archive.id
        ));
    }
    for script in &archive.scripts {
        write_new(
            &output.join(format!("script-{}", script.filename)),
            script.content.as_bytes(),
        )?;
    }
    write_new(&output.join("notebook.json"), source.as_bytes())?;
    // REPORT.md is the stable, read-on-demand address used by subsequent agents.
    // Failed attempts leave the draft open; a retry verifies/replaces only our
    // own regular report file before the record has ever been frozen.
    let report_path = directory.join("REPORT.md");
    if report_path.exists() {
        let rel = format!(".wisp/research-archives/{}/REPORT.md", archive.id);
        local_file(root, &rel)?;
        std::fs::remove_file(&report_path).map_err(err)?;
    }
    let mut report = format!(
        "# {}\n\n{}\n\n## Materials / 材料\n",
        archive.title, archive.report
    );
    for file in &archive.files {
        report.push_str(&format!(
            "- {} — {}{}\n",
            file.path,
            file.action,
            file.snapshot_path
                .as_ref()
                .map(|p| format!(" → {p}"))
                .unwrap_or_default()
        ));
    }
    for script in &archive.scripts {
        report.push_str(&format!(
            "- .wisp/research-archives/{}/{attempt}/script-{}\n",
            archive.id, script.filename
        ));
    }
    staging.report = Some(report_path.clone());
    write_new(&report_path, report.as_bytes())?;
    archive.frozen_at = Some(chrono::Utc::now().timestamp());
    write_new(
        &output.join("manifest.json"),
        serde_json::to_string_pretty(&archive)
            .map_err(err)?
            .as_bytes(),
    )?;
    store.freeze_research_archive(&archive).await.map_err(err)?;
    staging.committed = true;
    cleanup_files(store, root, &archive).await
}

/// Deletes, for good, the files a frozen archive chose to delete that are
/// not deleted yet, rechecking each first; one that changed or became
/// protected is kept. Each outcome is saved as the file's receipt.
pub async fn cleanup_files(
    store: &Store,
    root: &Path,
    archive: &ResearchArchive,
) -> Result<ResearchArchive, String> {
    let mut receipts = Vec::new();
    for file in archive
        .files
        .iter()
        .filter(|f| f.action == "delete" && f.cleanup_status != "deleted")
    {
        let result = async {
            if !file.can_delete
                || file.path.starts_with('.')
                || !store
                    .archive_path_deletable(&archive.frame_id, &file.path)
                    .await
                    .map_err(err)?
            {
                return Err("protected".into());
            }
            let real = local_file(root, &file.path)?;
            if file_hash(&real)? != (file.checksum.clone(), file.size_bytes) {
                return Err("changed since confirmation".into());
            }
            std::fs::remove_file(real).map_err(err)?;
            Ok::<_, String>(())
        }
        .await;
        receipts.push((
            file.path.clone(),
            match result {
                Ok(()) => "deleted".into(),
                Err(e) => format!("not deleted: {e}"),
            },
        ));
        // Persist each receipt so interruption never hides completed deletions.
        store
            .record_archive_cleanup(&archive.frame_id, &receipts)
            .await
            .map_err(err)?;
    }
    store
        .research_archive(&archive.frame_id)
        .await
        .map_err(err)?
        .ok_or("Archive no longer exists".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_llm::Message;

    async fn fixture() -> (Store, PathBuf, ResearchArchive) {
        let root = std::env::temp_dir().join(format!("wisp-research-archive-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store
            .create_project("p", "Research", root.to_str().unwrap())
            .await
            .unwrap();
        store.create_frame("f", "p", "OPERON", "m").await.unwrap();
        store
            .append_message(
                "f",
                1,
                &Message::user("Compare parameters and retain the selected result"),
            )
            .await
            .unwrap();
        for name in ["result.csv", "scaffold.R"] {
            record_creation(&store, &root, name, &format!("original {name}")).await;
        }
        let archive = draft(&store, &root).await;
        (store, root, archive)
    }

    /// Writes `name` and records it as made by the conversation.
    async fn record_creation(store: &Store, root: &Path, name: &str, text: &str) {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let (hash, _) = file_hash(&path).unwrap();
        store
            .save_turn_file_undo("f", 1, name, false, None, None, Some(&hash), true, None)
            .await
            .unwrap();
    }

    async fn draft(store: &Store, root: &Path) -> ResearchArchive {
        let archive = ResearchArchive {
            id: Uuid::new_v4().to_string(),
            project_id: "p".into(),
            frame_id: "f".into(),
            source_hash: store.research_archive_source("f").await.unwrap().1,
            title: "Selected parameters".into(),
            report: "Result and selection rationale. No rerun requested.".into(),
            scripts: vec![ArchiveScript {
                filename: "analysis.R".into(),
                content: "# Recorded operations\nprint(1)".into(),
            }],
            files: candidates(store, root, "f").await.unwrap(),
            created_at: chrono::Utc::now().timestamp(),
            frozen_at: None,
            warnings: vec![],
        };
        store.save_research_archive_draft(&archive).await.unwrap();
        archive
    }

    fn choose(
        a: &ResearchArchive,
        action: impl Fn(&str) -> &'static str,
    ) -> ConfirmResearchArchive {
        ConfirmResearchArchive {
            id: a.id.clone(),
            title: a.title.clone(),
            report: a.report.clone(),
            scripts: a.scripts.clone(),
            files: a
                .files
                .iter()
                .map(|f| wisp_dto::ArchiveFileChoice {
                    path: f.path.clone(),
                    action: action(&f.path).into(),
                })
                .collect(),
        }
    }

    fn confirmation(a: &ResearchArchive) -> ConfirmResearchArchive {
        choose(a, |path| {
            if path == "scaffold.R" {
                "delete"
            } else {
                "snapshot"
            }
        })
    }

    async fn frozen(store: &Store) -> bool {
        store
            .research_archive("f")
            .await
            .unwrap()
            .unwrap()
            .frozen_at
            .is_some()
    }

    #[tokio::test]
    async fn research_archive_freezes_before_cleanup_and_retains_notebook() {
        let (store, root, draft) = fixture().await;
        let archived = freeze_files(&store, &root, draft.clone(), confirmation(&draft))
            .await
            .unwrap();
        assert!(archived.frozen_at.is_some());
        assert!(!root.join("scaffold.R").exists());
        assert!(root.join("result.csv").exists());
        let saved = archived
            .files
            .iter()
            .find(|f| f.path == "result.csv")
            .unwrap()
            .snapshot_path
            .as_ref()
            .unwrap();
        std::fs::write(root.join("result.csv"), "later research").unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(saved)).unwrap(),
            "original result.csv"
        );
        assert_eq!(store.load_messages("f").await.unwrap().len(), 1);
        assert!(store
            .append_message("f", 2, &Message::user("rewrite history"))
            .await
            .is_err());
        assert!(store.replace_messages("f", &[]).await.is_err());
        assert!(store.delete_session("f", "p").await.is_err());
        assert!(store
            .move_session_to_project("f", "p", "q", "moved")
            .await
            .is_err());
        let now = chrono::Utc::now().timestamp();
        assert!(store
            .research_journey(&crate::StateScope::mainline("p"), now - 60, now + 60)
            .await
            .unwrap()
            .entries
            .iter()
            .any(|e| e.kind == "archive"));
        assert!(store
            .research_archive_index("p")
            .await
            .unwrap()
            .contains("REPORT.md"));
        assert_eq!(
            archived
                .files
                .iter()
                .find(|f| f.path == "scaffold.R")
                .unwrap()
                .cleanup_status,
            "deleted"
        );
        store.delete_project("p").await.unwrap();
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn research_archive_stale_file_or_notebook_cannot_delete_anything() {
        let (store, root, draft) = fixture().await;
        std::fs::write(root.join("scaffold.R"), "changed by another tool").unwrap();
        assert!(
            freeze_files(&store, &root, draft.clone(), confirmation(&draft))
                .await
                .unwrap_err()
                .contains("changed")
        );
        assert!(!frozen(&store).await);
        assert!(root.join("scaffold.R").exists());
        store
            .append_message("f", 2, &Message::user("New evidence"))
            .await
            .unwrap();
        assert!(
            freeze_files(&store, &root, draft.clone(), confirmation(&draft))
                .await
                .unwrap_err()
                .contains("notebook changed")
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn research_archive_rechecks_shared_use_and_forged_review() {
        let (store, root, draft) = fixture().await;
        store
            .create_frame("other", "p", "OPERON", "m")
            .await
            .unwrap();
        store
            .save_turn_file_undo(
                "other",
                1,
                "scaffold.R",
                true,
                None,
                None,
                None,
                false,
                None,
            )
            .await
            .unwrap();
        assert!(
            freeze_files(&store, &root, draft.clone(), confirmation(&draft))
                .await
                .unwrap_err()
                .contains("shared")
        );
        let mut input = confirmation(&draft);
        input.files[0].path = "../outside".into();
        assert!(freeze_files(&store, &root, draft, input).await.is_err());
        assert!(root.join("scaffold.R").exists());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn research_archive_cleanup_retry_never_removes_changed_bytes() {
        let (store, root, mut draft) = fixture().await;
        draft.frozen_at = Some(chrono::Utc::now().timestamp());
        for f in &mut draft.files {
            if f.path == "scaffold.R" {
                f.action = "delete".into();
                f.cleanup_status = "pending".into();
            }
        }
        store.freeze_research_archive(&draft).await.unwrap();
        std::fs::write(root.join("scaffold.R"), "new independent work").unwrap();
        let result = cleanup_files(&store, &root, &draft).await.unwrap();
        assert!(result
            .files
            .iter()
            .find(|f| f.path == "scaffold.R")
            .unwrap()
            .cleanup_status
            .contains("changed"));
        assert!(result.frozen_at.is_some());
        assert_eq!(
            std::fs::read_to_string(root.join("scaffold.R")).unwrap(),
            "new independent work"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn research_archive_snapshot_failure_does_not_lock_or_delete() {
        let (store, root, draft) = fixture().await;
        std::fs::write(root.join(".wisp"), "not a directory").unwrap();
        assert!(
            freeze_files(&store, &root, draft.clone(), confirmation(&draft))
                .await
                .is_err()
        );
        assert!(!frozen(&store).await);
        assert!(root.join("scaffold.R").exists());
        assert!(store
            .append_message("f", 2, &Message::user("continue after failure"))
            .await
            .is_ok());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn research_archive_late_failure_removes_partial_snapshots_only() {
        let (store, root, draft) = fixture().await;
        let directory = archive_dir(&root, &draft.id).unwrap();
        let report = directory.join("REPORT.md");
        std::fs::create_dir(&report).unwrap();
        assert!(
            freeze_files(&store, &root, draft.clone(), confirmation(&draft))
                .await
                .is_err()
        );
        assert!(!frozen(&store).await);
        assert_eq!(
            std::fs::read_to_string(root.join("scaffold.R")).unwrap(),
            "original scaffold.R"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("result.csv")).unwrap(),
            "original result.csv"
        );
        let remaining: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(remaining, vec![report.clone()]);
        assert!(report.is_dir());
        assert!(store
            .append_message("f", 2, &Message::user("continue after late failure"))
            .await
            .is_ok());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn research_archive_paths_and_script_names_cannot_escape() {
        let root = std::env::temp_dir();
        for path in [
            "../outside",
            "C:/outside",
            "file.txt:stream",
            "a\\b",
            "/tmp/outside",
        ] {
            assert!(local_file(&root, path).is_err());
        }
        assert!(validate_content(
            "title",
            "report",
            &[ArchiveScript {
                filename: "../run.R".into(),
                content: String::new()
            }]
        )
        .is_err());
    }

    #[tokio::test]
    async fn each_file_choice_does_what_it_says_and_nothing_else_is_touched() {
        let (store, root, _) = fixture().await;
        record_creation(&store, &root, "plot.png", "original plot.png").await;
        // Not recorded by the conversation: never listed, never touched.
        std::fs::write(root.join("notes.txt"), "the researcher's own").unwrap();
        let draft = draft(&store, &root).await;
        let listed: Vec<_> = draft.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(listed, ["plot.png", "result.csv", "scaffold.R"]);
        // A snapshot by default; recommended deletions only where allowed.
        assert!(draft.files.iter().all(|f| f.action == "snapshot"));
        let mut files = draft.files.clone();
        recommend_deletions(&mut files, &["scaffold.R".into(), "missing.txt".into()]);
        let actions: Vec<_> = files.iter().map(|f| f.action.as_str()).collect();
        assert_eq!(actions, ["snapshot", "snapshot", "delete"]);
        let input = choose(&draft, |path| match path {
            "plot.png" => "reference",
            "result.csv" => "snapshot",
            _ => "delete",
        });
        let archived = freeze_files(&store, &root, draft.clone(), input)
            .await
            .unwrap();
        let file = |path: &str| archived.files.iter().find(|f| f.path == path).unwrap();
        // Kept in place: there, as it was, with no copy.
        assert_eq!(file("plot.png").action, "reference");
        assert_eq!(file("plot.png").snapshot_path, None);
        assert_eq!(
            std::fs::read_to_string(root.join("plot.png")).unwrap(),
            "original plot.png"
        );
        // A snapshot: the original stays and a copy is in the archive.
        let copy = file("result.csv").snapshot_path.clone().unwrap();
        assert!(copy.starts_with(&format!(".wisp/research-archives/{}/", archived.id)));
        assert_eq!(
            std::fs::read_to_string(root.join(&copy)).unwrap(),
            "original result.csv"
        );
        assert!(root.join("result.csv").is_file());
        // Deleted for good, with its receipt.
        assert!(!root.join("scaffold.R").exists());
        assert_eq!(file("scaffold.R").cleanup_status, "deleted");
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "the researcher's own"
        );
        // The report lists what became of each file, and the scripts.
        let report = std::fs::read_to_string(
            root.join(format!(".wisp/research-archives/{}/REPORT.md", archived.id)),
        )
        .unwrap();
        assert!(report.contains("- plot.png — reference\n"), "{report}");
        assert!(report.contains("- scaffold.R — delete\n"), "{report}");
        assert!(report.contains("/script-analysis.R\n"), "{report}");
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_wisp_file_is_listed_but_never_deleted() {
        let (store, root, _) = fixture().await;
        record_creation(&store, &root, ".wisp/notes.txt", "kept").await;
        let draft = draft(&store, &root).await;
        let wisp = draft
            .files
            .iter()
            .find(|f| f.path == ".wisp/notes.txt")
            .unwrap();
        assert!(!wisp.can_delete);
        let mut files = draft.files.clone();
        recommend_deletions(&mut files, &[".wisp/notes.txt".into()]);
        assert!(files.iter().all(|f| f.action == "snapshot"));
        // A forged choice to delete it is refused, and nothing is deleted.
        let input = choose(&draft, |path| match path {
            "result.csv" => "snapshot",
            _ => "delete",
        });
        let refused = freeze_files(&store, &root, draft, input).await.unwrap_err();
        assert!(refused.contains(".wisp/notes.txt"), "{refused}");
        assert!(!frozen(&store).await);
        assert!(root.join(".wisp/notes.txt").is_file());
        assert!(root.join("scaffold.R").is_file());
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_path_outside_the_project_is_refused_and_nothing_is_deleted() {
        let (store, root, draft) = fixture().await;
        let outside = std::env::temp_dir().join(format!("wisp-outside-{}", Uuid::new_v4()));
        std::fs::write(&outside, "someone else's").unwrap();
        for forged in [
            "../outside".to_owned(),
            outside.to_string_lossy().into_owned(),
        ] {
            let mut input = confirmation(&draft);
            input.files[0].path = forged.clone();
            input.files[0].action = "delete".into();
            assert!(
                freeze_files(&store, &root, draft.clone(), input)
                    .await
                    .is_err(),
                "{forged}"
            );
        }
        // One more choice than the review listed is incomplete, not extra.
        let mut input = confirmation(&draft);
        input.files.push(wisp_dto::ArchiveFileChoice {
            path: outside.to_string_lossy().into_owned(),
            action: "delete".into(),
        });
        assert!(freeze_files(&store, &root, draft.clone(), input)
            .await
            .is_err());
        assert!(!frozen(&store).await);
        assert!(outside.is_file());
        assert!(root.join("scaffold.R").is_file());
        let _ = std::fs::remove_file(outside);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symbolic_link_out_of_the_project_is_neither_listed_nor_followed() {
        let (store, root, _) = fixture().await;
        let outside = std::env::temp_dir().join(format!("wisp-outside-{}", Uuid::new_v4()));
        std::fs::write(&outside, "original scaffold.R").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.R")).unwrap();
        store
            .save_turn_file_undo("f", 1, "link.R", false, None, None, None, true, None)
            .await
            .unwrap();
        let draft = draft(&store, &root).await;
        assert!(draft.files.iter().all(|f| f.path != "link.R"));
        // A listed file swapped for a link after the review is refused.
        std::fs::remove_file(root.join("scaffold.R")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("scaffold.R")).unwrap();
        let refused = freeze_files(&store, &root, draft.clone(), confirmation(&draft))
            .await
            .unwrap_err();
        assert!(refused.contains("Symbolic links"), "{refused}");
        assert!(!frozen(&store).await);
        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            "original scaffold.R"
        );
        assert!(root.join("result.csv").is_file());
        let _ = std::fs::remove_file(outside);
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_cleanup_that_fails_part_way_leaves_the_archive_frozen_with_receipts() {
        let (store, root, _) = fixture().await;
        record_creation(&store, &root, "tmp.csv", "original tmp.csv").await;
        let mut draft = draft(&store, &root).await;
        draft.frozen_at = Some(chrono::Utc::now().timestamp());
        for f in &mut draft.files {
            if f.path == "scaffold.R" || f.path == "tmp.csv" {
                f.action = "delete".into();
                f.cleanup_status = "pending".into();
            }
        }
        store.freeze_research_archive(&draft).await.unwrap();
        std::fs::write(root.join("tmp.csv"), "changed after the review").unwrap();
        let result = cleanup_files(&store, &root, &draft).await.unwrap();
        let status = |path: &str| {
            result
                .files
                .iter()
                .find(|f| f.path == path)
                .unwrap()
                .cleanup_status
                .clone()
        };
        assert_eq!(status("scaffold.R"), "deleted");
        assert_eq!(status("tmp.csv"), "not deleted: changed since confirmation");
        assert_eq!(status("result.csv"), "");
        assert!(result.frozen_at.is_some());
        assert!(!root.join("scaffold.R").exists());
        assert!(root.join("tmp.csv").is_file());
        assert!(root.join("result.csv").is_file());
        // A retry tries only what is not deleted, and keeps changed bytes.
        let again = cleanup_files(&store, &root, &result).await.unwrap();
        assert!(again
            .files
            .iter()
            .all(|f| f.path != "tmp.csv" || f.cleanup_status.contains("changed")));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
