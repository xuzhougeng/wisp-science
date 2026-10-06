//! A copy of each database, taken before another app version migrates it.
//! Migrations here only go forward; this copy is the way back (#1175).
//!
//! `PRAGMA user_version` records the app version that last migrated the
//! database. It is negated between taking the copy and finishing the
//! migration, so a migration that fails on every launch cannot rotate the
//! last good copy out.
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DIRECTORY: &str = "backups";
const KEPT: usize = 3;

/// 1.17.0 -> 1_017_000. Pre-release suffixes are ignored.
fn stamp_of(version: &str) -> i64 {
    let mut parts = version.split('.').map(|part| {
        let digits = part
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(part.len());
        part[..digits].parse::<i64>().unwrap_or(0)
    });
    let mut next = || parts.next().unwrap_or(0);
    next() * 1_000_000 + next() * 1_000 + next()
}

async fn stamp(pool: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?)
}

async fn database_file(pool: &SqlitePool) -> Result<Option<PathBuf>> {
    let file: String =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name='main'")
            .fetch_one(pool)
            .await?;
    Ok((!file.is_empty()).then(|| PathBuf::from(file)))
}

/// Call before migrating a live database. `owner` is the application database
/// whose directory keeps the copies: project folders are exported, synced and
/// scanned, so nothing is added to them. Failing to copy never blocks the
/// open; an upgrade must not become less reliable than it was without this.
pub(crate) async fn before_migration(pool: &SqlitePool, owner: &SqlitePool, project: Option<&str>) {
    if let Err(error) = copy_if_version_changed(pool, owner, project).await {
        tracing::warn!(%error, "Database was not backed up before migration");
    }
}

/// Record that this version finished migrating the database.
pub(crate) async fn migrated(pool: &SqlitePool) -> Result<()> {
    let current = stamp_of(VERSION);
    if stamp(pool).await? != current {
        sqlx::query(&format!("PRAGMA user_version={current}"))
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn copy_if_version_changed(
    pool: &SqlitePool,
    owner: &SqlitePool,
    project: Option<&str>,
) -> Result<()> {
    let current = stamp_of(VERSION);
    let last = stamp(pool).await?;
    if last.abs() == current {
        return Ok(());
    }
    // A database this process is creating has nothing to lose.
    let populated: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
    )
    .fetch_one(pool)
    .await?;
    let (true, Some(database), Some(owner)) = (
        populated,
        database_file(pool).await?,
        database_file(owner).await?,
    ) else {
        return Ok(());
    };
    if last.abs() > current {
        // Older versions are expected to keep working on additive schema
        // changes, so this is not refused. The copy keeps the newer state.
        let (major, minor, patch) = (
            last.abs() / 1_000_000,
            last.abs() / 1_000 % 1_000,
            last.abs() % 1_000,
        );
        tracing::warn!(
            database = %database.display(),
            last_migrated_by = %format!("{major}.{minor}.{patch}"),
            "Opening a database last migrated by a newer version"
        );
    }
    let directory = owner
        .parent()
        .context("database has no directory")?
        .join(DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    let label = match project {
        Some(id) => format!("project-{}", super::project_snapshots::file_label(id)),
        None => database
            .file_stem()
            .context("database has no name")?
            .to_string_lossy()
            .into_owned(),
    };
    // `<label>.<YYYYMMDD-HHMMSS>.pre-<version>.sqlite`: names sort by age.
    let name = format!(
        "{label}.{}.pre-{VERSION}.sqlite",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let pending = directory.join(format!("{name}.tmp"));
    let published = async {
        copy_database(pool, &database, &pending).await?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&pending)?
            .sync_all()?;
        Ok::<_, anyhow::Error>(std::fs::rename(&pending, directory.join(&name))?)
    }
    .await;
    if published.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    published?;
    prune(&directory, &label);
    sqlx::query(&format!("PRAGMA user_version={}", -current))
        .execute(pool)
        .await?;
    tracing::info!(database = %database.display(), backup = %name, "Backed up database before migration");
    Ok(())
}

/// A plain file copy is about 30x faster than `VACUUM INTO` (0.3s against 8s
/// for 600 MB), and the application database is copied while the desktop
/// window cannot paint yet. The file alone is the whole database only while
/// the WAL is empty and nobody else can write; otherwise take the slow path.
async fn copy_database(pool: &SqlitePool, database: &Path, target: &Path) -> Result<()> {
    match copy_file_while_quiet(pool, database, target).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(error) => tracing::debug!(%error, "Falling back to VACUUM INTO for the backup"),
    }
    let _ = std::fs::remove_file(target);
    sqlx::query("VACUUM INTO ?")
        .bind(target.to_string_lossy().as_ref())
        .execute(pool)
        .await?;
    Ok(())
}

/// `Ok(false)` when another connection still needs the WAL.
async fn copy_file_while_quiet(pool: &SqlitePool, database: &Path, target: &Path) -> Result<bool> {
    // A no-op for rollback-journal databases, which have no WAL to empty.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await?;
    // Holding the write lock: no commit can reach the WAL, and with nothing in
    // the WAL no checkpoint can touch the file being copied.
    let lock = pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut wal = database.as_os_str().to_owned();
    wal.push("-wal");
    let quiet = match std::fs::metadata(&wal) {
        Ok(metadata) => metadata.len() == 0,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    };
    let copied = quiet.then(|| std::fs::copy(database, target));
    lock.rollback().await?;
    Ok(copied.transpose()?.is_some())
}

fn prune(directory: &Path, label: &str) {
    let prefix = format!("{label}.");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut copies = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.get(15..))
            .is_some_and(|tail| tail.starts_with(".pre-"));
        if ours && name.ends_with(".sqlite") {
            copies.push(name);
        } else if ours && name.ends_with(".tmp") {
            // Left by a crash mid-copy; this run's own file is already renamed.
            let _ = std::fs::remove_file(entry.path());
        }
    }
    copies.sort_unstable_by(|a, b| b.cmp(a));
    for name in copies.into_iter().skip(KEPT) {
        // Inspecting a copy leaves WAL files beside it.
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(directory.join(format!("{name}{suffix}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn copies(directory: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(directory.join(DIRECTORY)) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with("-wal") && !name.ends_with("-shm"))
            .collect();
        names.sort();
        names
    }

    async fn raw(path: &Path) -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap()
    }

    /// A database with a record in it, left as `version` would have left it.
    async fn written_by(path: &Path, version: i64) {
        let store = Store::open(path).await.unwrap();
        store.set_setting("probe", "kept").await.unwrap();
        sqlx::query(&format!("PRAGMA user_version={version}"))
            .execute(&store.pool)
            .await
            .unwrap();
        Store::close_pool(&store.pool).await;
    }

    #[test]
    fn versions_order_numerically() {
        assert_eq!(stamp_of("1.17.0"), 1_017_000);
        assert_eq!(stamp_of("2.0.3-beta.1"), 2_000_003);
        assert!(stamp_of("1.9.0") < stamp_of("1.10.0"));
    }

    #[tokio::test]
    async fn a_new_database_is_stamped_without_a_copy() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("wisp.sqlite")).await.unwrap();
        assert_eq!(stamp(&store.pool).await.unwrap(), stamp_of(VERSION));
        assert!(copies(root.path()).is_empty());
    }

    #[tokio::test]
    async fn an_unversioned_database_is_copied_once_before_it_is_migrated() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wisp.sqlite");
        written_by(&path, 0).await;
        // Make one migration pending, so the copy must be the state before it.
        let legacy = raw(&path).await;
        sqlx::query("DROP TABLE session_reviews")
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(crate::SESSION_REVIEWS_MIGRATION)
            .execute(&legacy)
            .await
            .unwrap();
        Store::close_pool(&legacy).await;

        let store = Store::open(&path).await.unwrap();
        assert_eq!(stamp(&store.pool).await.unwrap(), stamp_of(VERSION));
        let names = copies(root.path());
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].starts_with("wisp."), "{names:?}");
        assert!(
            names[0].ends_with(&format!(".pre-{VERSION}.sqlite")),
            "{names:?}"
        );
        let has_reviews = "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='session_reviews')";
        assert!(sqlx::query_scalar::<_, bool>(has_reviews)
            .fetch_one(&store.pool)
            .await
            .unwrap());
        Store::close_pool(&store.pool).await;

        let copy = Store::open_read_only(&root.path().join(DIRECTORY).join(&names[0]))
            .await
            .unwrap();
        assert_eq!(
            copy.get_setting("probe").await.unwrap().as_deref(),
            Some("kept")
        );
        assert!(!sqlx::query_scalar::<_, bool>(has_reviews)
            .fetch_one(&copy.pool)
            .await
            .unwrap());
        // The copy says which version wrote it.
        assert_eq!(stamp(&copy.pool).await.unwrap(), 0);
        Store::close_pool(&copy.pool).await;

        Store::close_pool(&Store::open(&path).await.unwrap().pool).await;
        assert_eq!(copies(root.path()), names);
    }

    /// The manual steps in docs/upgrade-backups.md.
    #[tokio::test]
    async fn a_copy_put_back_in_place_opens_as_it_was() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wisp.sqlite");
        written_by(&path, 0).await;
        let store = Store::open(&path).await.unwrap();
        store.set_setting("probe", "after").await.unwrap();
        Store::close_pool(&store.pool).await;
        let names = copies(root.path());

        let aside = root.path().join("aside");
        std::fs::create_dir(&aside).unwrap();
        for file in ["wisp.sqlite", "wisp.sqlite-wal", "wisp.sqlite-shm"] {
            if root.path().join(file).exists() {
                std::fs::rename(root.path().join(file), aside.join(file)).unwrap();
            }
        }
        std::fs::copy(root.path().join(DIRECTORY).join(&names[0]), &path).unwrap();

        let store = Store::open(&path).await.unwrap();
        assert_eq!(
            store.get_setting("probe").await.unwrap().as_deref(),
            Some("kept")
        );
        let aside = Store::open_read_only(&aside.join("wisp.sqlite"))
            .await
            .unwrap();
        assert_eq!(
            aside.get_setting("probe").await.unwrap().as_deref(),
            Some("after")
        );
    }

    #[tokio::test]
    async fn a_newer_versions_database_is_copied_before_an_older_one_opens_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wisp.sqlite");
        written_by(&path, stamp_of(VERSION) + 1_000).await;
        let store = Store::open(&path).await.unwrap();
        assert_eq!(stamp(&store.pool).await.unwrap(), stamp_of(VERSION));
        assert_eq!(copies(root.path()).len(), 1);
    }

    #[tokio::test]
    async fn a_migration_that_failed_after_its_copy_is_retried_without_another() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wisp.sqlite");
        written_by(&path, -stamp_of(VERSION)).await;
        let store = Store::open(&path).await.unwrap();
        assert_eq!(stamp(&store.pool).await.unwrap(), stamp_of(VERSION));
        assert!(copies(root.path()).is_empty());
    }

    #[tokio::test]
    async fn only_the_newest_copies_of_a_database_are_kept() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wisp.sqlite");
        written_by(&path, 0).await;
        let directory = root.path().join(DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        let bystanders = [
            "library.20200101-000001.pre-1.0.0.sqlite",
            "wisp.notes.sqlite",
        ];
        for name in [
            "wisp.20200101-000001.pre-1.0.0.sqlite",
            "wisp.20200101-000002.pre-1.9.0.sqlite",
            "wisp.20200101-000003.pre-1.10.0.sqlite",
            "wisp.20200101-000000.pre-1.0.0.sqlite.tmp",
        ]
        .iter()
        .chain(&bystanders)
        {
            std::fs::write(directory.join(name), b"old").unwrap();
        }
        Store::open(&path).await.unwrap();
        let names = copies(root.path());
        assert_eq!(names.len(), KEPT + bystanders.len(), "{names:?}");
        for kept in [
            "wisp.20200101-000002.pre-1.9.0.sqlite",
            "wisp.20200101-000003.pre-1.10.0.sqlite",
        ]
        .iter()
        .chain(&bystanders)
        {
            assert!(names.iter().any(|name| name == kept), "{names:?}");
        }
    }

    #[tokio::test]
    async fn a_database_that_cannot_be_copied_still_opens() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wisp.sqlite");
        written_by(&path, 0).await;
        std::fs::write(root.path().join(DIRECTORY), b"in the way").unwrap();
        let store = Store::open(&path).await.unwrap();
        assert_eq!(stamp(&store.pool).await.unwrap(), stamp_of(VERSION));
        assert_eq!(
            store.get_setting("probe").await.unwrap().as_deref(),
            Some("kept")
        );
    }

    #[tokio::test]
    async fn a_wal_another_connection_still_reads_is_copied_consistently() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("live.sqlite");
        let pool = raw(&path).await;
        // Do not wait for the reader below to finish.
        sqlx::query("PRAGMA busy_timeout=0")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE notes (body TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO notes VALUES('checkpointed')")
            .execute(&pool)
            .await
            .unwrap();
        let reader = raw(&path).await;
        let mut pinned = reader.begin().await.unwrap();
        let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes")
            .fetch_one(&mut *pinned)
            .await
            .unwrap();
        // Committed, but the reader keeps it out of the database file.
        sqlx::query("INSERT INTO notes VALUES('only in the wal')")
            .execute(&pool)
            .await
            .unwrap();

        let target = root.path().join("copy.sqlite");
        copy_database(&pool, &path, &target).await.unwrap();
        drop(pinned);

        let copy = raw(&target).await;
        let bodies: Vec<String> = sqlx::query_scalar("SELECT body FROM notes ORDER BY rowid")
            .fetch_all(&copy)
            .await
            .unwrap();
        assert_eq!(bodies, ["checkpointed", "only in the wal"]);
        for pool in [copy, reader, pool] {
            Store::close_pool(&pool).await;
        }
    }

    #[tokio::test]
    async fn the_library_is_copied_like_the_application_database() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("library.sqlite");
        drop(crate::LibraryStore::open(&path).await.unwrap());
        assert!(copies(root.path()).is_empty());
        let older = raw(&path).await;
        assert_eq!(stamp(&older).await.unwrap(), stamp_of(VERSION));
        sqlx::query("PRAGMA user_version=1")
            .execute(&older)
            .await
            .unwrap();
        Store::close_pool(&older).await;
        drop(crate::LibraryStore::open(&path).await.unwrap());
        let current = raw(&path).await;
        assert_eq!(stamp(&current).await.unwrap(), stamp_of(VERSION));
        Store::close_pool(&current).await;
        let names = copies(root.path());
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].starts_with("library."), "{names:?}");
    }
}
