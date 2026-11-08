//! Which file a launch writes its log to. Windows and macOS release builds log
//! to `wisp.log` in the app's logs directory and keep the launch before it as
//! `wisp.previous.log` (`SharedLogFile` in `lib.rs` is the only caller).
//!
//! Clicking the taskbar or Dock icon while Wisp is running starts a second
//! process that only hands its arguments to the running window and exits, but
//! logging starts before the single-instance plugin gets to do that. Rotating
//! unconditionally renamed the running window's log out from under it: the run
//! being measured kept writing into `wisp.previous.log`, and the next click
//! replaced that file too (#1690). Only the launch that holds `wisp.log.lock`
//! rotates now.
//!
//! The lock is the standard library's advisory file lock: `LockFileEx` on
//! Windows and `flock` on macOS. Both platforms release it when the owning
//! process exits, crashes included, and the handle is opened non-inheritable
//! (`O_CLOEXEC` on macOS), so neither a dead launch nor a child process it
//! spawned can keep a later launch from rotating.

use std::fs::{self, File, TryLockError};
use std::io;
use std::path::Path;

pub(crate) const LOG_FILE: &str = "wisp.log";
pub(crate) const PREVIOUS_LOG_FILE: &str = "wisp.previous.log";
/// Written by a launch that found another one already owning `wisp.log`. That
/// launch normally exits as soon as it has handed over, but if the hand-over
/// fails and it keeps running, its own log is still on disk.
pub(crate) const SECONDARY_LOG_FILE: &str = "wisp.secondary.log";
const LOCK_FILE: &str = "wisp.log.lock";

pub(crate) struct LaunchLog {
    file: File,
    /// Kept open for the life of the process so later launches see `wisp.log`
    /// as taken.
    _owner_lock: Option<File>,
}

impl LaunchLog {
    /// Open this launch's log file in `dir`.
    ///
    /// The launch that takes the lock moves the last `wisp.log` to
    /// `wisp.previous.log` and starts a fresh one: someone hitting a slow
    /// launch restarts the app before asking for help, so the run that has to
    /// be explained is always the previous one. A launch that finds the lock
    /// held touches neither file and writes `wisp.secondary.log` instead. When
    /// the lock cannot be taken for any other reason, rotate as before rather
    /// than leave the launch without a log.
    pub(crate) fn open(dir: &Path) -> io::Result<Self> {
        let owner_lock = match claim(dir) {
            Claim::Owner(lock) => Some(lock),
            Claim::HeldElsewhere => {
                return Ok(Self {
                    file: File::create(dir.join(SECONDARY_LOG_FILE))?,
                    _owner_lock: None,
                });
            }
            Claim::Unavailable => None,
        };
        let path = dir.join(LOG_FILE);
        let _ = fs::rename(&path, dir.join(PREVIOUS_LOG_FILE));
        Ok(Self {
            file: File::create(path)?,
            _owner_lock: owner_lock,
        })
    }
}

impl io::Write for LaunchLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

enum Claim {
    Owner(File),
    HeldElsewhere,
    /// The lock file could not be opened or locked (for example a filesystem
    /// without lock support), so ownership is unknown.
    Unavailable,
}

fn claim(dir: &Path) -> Claim {
    let Ok(lock) = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE))
    else {
        return Claim::Unavailable;
    };
    match lock.try_lock() {
        Ok(()) => Claim::Owner(lock),
        Err(TryLockError::WouldBlock) => Claim::HeldElsewhere,
        Err(TryLockError::Error(_)) => Claim::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_logs_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "wisp-launch-log-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn owning_launch_keeps_the_last_log_as_previous() {
        let dir = temp_logs_dir("owner");
        fs::write(dir.join(LOG_FILE), "last launch\n").unwrap();
        fs::write(dir.join(PREVIOUS_LOG_FILE), "launch before that\n").unwrap();

        let mut log = LaunchLog::open(&dir).unwrap();
        writeln!(log, "this launch").unwrap();

        assert_eq!(read(&dir, PREVIOUS_LOG_FILE), "last launch\n");
        assert_eq!(read(&dir, LOG_FILE), "this launch\n");
        assert!(!dir.join(SECONDARY_LOG_FILE).exists());
        drop(log);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn second_launch_leaves_the_running_log_and_previous_log_alone() {
        let dir = temp_logs_dir("second");
        fs::write(dir.join(LOG_FILE), "last launch\n").unwrap();
        let mut running = LaunchLog::open(&dir).unwrap();
        writeln!(running, "running window").unwrap();

        let mut second = LaunchLog::open(&dir).unwrap();
        writeln!(second, "handing over").unwrap();
        writeln!(running, "still running").unwrap();

        assert_eq!(read(&dir, LOG_FILE), "running window\nstill running\n");
        assert_eq!(read(&dir, PREVIOUS_LOG_FILE), "last launch\n");
        assert_eq!(read(&dir, SECONDARY_LOG_FILE), "handing over\n");
        drop(second);
        drop(running);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn next_launch_rotates_once_the_owner_has_exited() {
        let dir = temp_logs_dir("released");
        let mut first = LaunchLog::open(&dir).unwrap();
        writeln!(first, "first launch").unwrap();
        drop(first);

        let next = LaunchLog::open(&dir).unwrap();

        assert_eq!(read(&dir, PREVIOUS_LOG_FILE), "first launch\n");
        assert_eq!(read(&dir, LOG_FILE), "");
        assert!(!dir.join(SECONDARY_LOG_FILE).exists());
        drop(next);
        fs::remove_dir_all(&dir).unwrap();
    }
}
