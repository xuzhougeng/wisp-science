//! A run's whole log, read from where the run wrote it. A run record keeps
//! only the last lines; a run on this computer writes `stdout.log` and
//! `stderr.log` into its own folder under the home folder, and those are
//! read here. A run on another machine keeps its log there, and that is
//! not read from here.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

/// As much of each stream as is read: its end, where a run says how it went.
pub(crate) const LIMIT: u64 = 512 * 1024;
const RUNS: &str = "~/.wisp-science/runs/";

/// What a run wrote, or the end of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunLog {
    pub stdout: String,
    pub stderr: String,
    /// One of the two is longer than what is here: its beginning is left out.
    pub cut: bool,
}

/// The folder a run on this computer writes into. Only the place the run
/// manager gives such a run is accepted, so a record cannot point the
/// reader at any other file.
pub(crate) fn folder(home: &Path, workdir: &str, run: &str) -> Result<PathBuf> {
    let valid = !run.is_empty()
        && run
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
    ensure!(
        valid && workdir.trim_end_matches('/') == format!("{RUNS}{run}"),
        "This run's log is not kept where it can be read from here"
    );
    Ok(home.join(".wisp-science/runs").join(run))
}

/// The last `limit` bytes of a file as text, and whether there was more.
/// A file that is not there reads as empty: a run that wrote nothing to a
/// stream has no file for it.
fn end_of(path: &Path, limit: u64) -> Result<(String, bool)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((String::new(), false))
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", path.display()))
        }
    };
    let size = file.metadata()?.len();
    let cut = size > limit;
    if cut {
        file.seek(SeekFrom::Start(size - limit))?;
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if cut {
        // Start at a whole line rather than in the middle of one.
        if let Some(line) = text.find('\n') {
            text.drain(..=line);
        }
    }
    Ok((text, cut))
}

pub(crate) fn read(folder: &Path) -> Result<RunLog> {
    ensure!(
        folder.is_dir(),
        "The run's folder is gone, and its log with it; the last lines are in the run's output"
    );
    let (stdout, cut_out) = end_of(&folder.join("stdout.log"), LIMIT)?;
    let (stderr, cut_err) = end_of(&folder.join("stderr.log"), LIMIT)?;
    Ok(RunLog {
        stdout,
        stderr,
        cut: cut_out || cut_err,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_runs_own_folder_under_home_is_read() {
        let home = Path::new("/home/me");
        let id = "710f9fe7-0843-4906-9953-6da23b32a4ba";
        assert_eq!(
            folder(home, &format!("~/.wisp-science/runs/{id}"), id).unwrap(),
            home.join(".wisp-science/runs").join(id)
        );
        assert!(folder(home, &format!("~/.wisp-science/runs/{id}/"), id).is_ok());
        // Somewhere else, another run's folder, or a way out of the folder.
        for (workdir, run) in [
            ("/etc", id),
            ("~/.wisp-science/runs/other", id),
            ("~/.wisp-science/runs/../../.ssh", "../../.ssh"),
            ("~/.wisp-science/runs/", ""),
            ("wisp-runs/abc", "abc"),
        ] {
            assert!(folder(home, workdir, run).is_err(), "{workdir}");
        }
    }

    #[test]
    fn a_log_is_read_whole_or_from_a_whole_line_near_its_end() {
        let temp = tempfile::tempdir().unwrap();
        // Nothing written to a stream: no file, and that is fine.
        assert_eq!(read(temp.path()).unwrap(), RunLog::default());
        std::fs::write(temp.path().join("stdout.log"), "started\ndone\n").unwrap();
        std::fs::write(temp.path().join("stderr.log"), b"warn \xff\n").unwrap();
        let log = read(temp.path()).unwrap();
        assert_eq!(log.stdout, "started\ndone\n");
        // Bytes that are not text are shown as such, not refused.
        assert_eq!(log.stderr, "warn \u{fffd}\n");
        assert!(!log.cut);

        let long = temp.path().join("long.log");
        std::fs::write(&long, "first line\nsecond line\nthird line\n").unwrap();
        let (text, cut) = end_of(&long, 18).unwrap();
        assert!(cut);
        assert_eq!(text, "third line\n");

        // A folder that was cleaned up says so.
        let gone = read(&temp.path().join("gone")).unwrap_err().to_string();
        assert!(gone.contains("folder is gone"), "{gone}");
    }
}
