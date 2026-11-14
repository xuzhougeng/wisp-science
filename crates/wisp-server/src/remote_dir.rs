//! One folder of an SSH host, listed for the Files tab. The host is asked
//! with a small POSIX script over the connection the run manager uses; the
//! answer is NUL-separated so a file name can hold anything a name can.
//!
//! The script and the reading of its answer are the WebView desktop's
//! (`src-tauri/src/file_browser.rs`), which keeps its own copy until that
//! backend goes. Nothing is changed on the host: this only looks.

use anyhow::{bail, ensure, Context, Result};
use wisp_dto::DirEntry;
use wisp_runs::{RunCommand, RunCommandOutput};

const MARKER: &str = "WISP_REMOTE_DIR_V1\0";
pub(crate) const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// More entries than this are not listed; the folder says it was cut.
const ENTRY_LIMIT: usize = 2_000;

/// A folder on a host: its real path and what is in it, folders first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDirectory {
    pub alias: String,
    /// The folder's real path on the host.
    pub path: String,
    pub entries: Vec<DirEntry>,
    /// There was more than is listed.
    pub cut: bool,
}

fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// The folder as the script names it. Empty is the home folder; `~/x` is
/// under it; anything else is taken as written, quoted as data.
fn folder_expression(path: &str) -> Result<String> {
    let path = match path.trim() {
        "" => "~",
        path => path,
    };
    ensure!(path.len() <= 4096, "That path is too long");
    ensure!(
        !path.contains(['\0', '\n', '\r']),
        "A path cannot have line breaks in it"
    );
    Ok(match path.strip_prefix("~/") {
        _ if path == "~" => "\"$HOME\"".into(),
        Some(rest) => format!("\"$HOME\"/{}", quoted(rest)),
        None => quoted(path),
    })
}

/// The script that lists `path`. It only reads.
pub(crate) fn script(path: &str) -> Result<String> {
    let folder = folder_expression(path)?;
    // One more than is listed, so a folder with more says so.
    let limit = ENTRY_LIMIT + 1;
    Ok(format!(
        r#"LC_ALL=C
dir={folder}
case "$dir" in -*) dir="./$dir" ;; esac
if ! CDPATH= cd "$dir" 2>/dev/null; then
  printf 'Cannot open the folder: %s\n' "$dir" >&2
  exit 66
fi
printf 'WISP_REMOTE_DIR_V1\000%s\000' "$(pwd -P)"
count=0
for entry in ./*; do
  if [ ! -e "$entry" ] && [ ! -L "$entry" ]; then
    continue
  fi
  count=$((count + 1))
  if [ "$count" -gt {limit} ]; then
    break
  fi
  name=${{entry#./}}
  mtime=$(stat -c '%Y' "$entry" 2>/dev/null) ||
    mtime=$(stat -f '%m' "$entry" 2>/dev/null) ||
    mtime=0
  if [ -d "$entry" ]; then
    kind=d
    size=0
  else
    kind=f
    size=$(stat -c '%s' "$entry" 2>/dev/null) ||
      size=$(stat -f '%z' "$entry" 2>/dev/null) ||
      size=0
  fi
  printf '%s\000%s\000%s\000%s\000' "$kind" "$size" "$mtime" "$name"
done"#
    ))
}

/// What is run on this computer to ask the host.
pub(crate) fn command(
    connection: &wisp_runs::ssh_hosts::SshConnection,
    path: &str,
) -> Result<RunCommand> {
    let mut args = connection.ssh_args().map_err(anyhow::Error::msg)?;
    args.push("sh -s".into());
    Ok(RunCommand {
        context_id: format!("ssh:{}", connection.alias),
        program: "ssh".into(),
        args,
        script: "list a folder".into(),
        cwd: None,
        stdin: Some(script(path)?),
        envs: wisp_runs::ssh_hosts::auth_envs_for_connection(connection)
            .map_err(anyhow::Error::msg)?,
    })
}

/// Reads the host's answer. A login banner before the marker is passed
/// over; an answer that is not the script's is refused whole.
pub(crate) fn parse(alias: &str, output: RunCommandOutput) -> Result<RemoteDirectory> {
    if output.exit_code != 0 {
        let said = match output.stderr.trim() {
            "" => "the host gave no reason",
            said => said,
        };
        bail!("The folder could not be listed: {said}");
    }
    let (_, answer) = output
        .stdout
        .split_once(MARKER)
        .context("The host's answer was not a folder listing")?;
    let mut fields: Vec<&str> = answer.split('\0').collect();
    // Every field ends with a NUL, so the last piece is empty. If it is
    // not, the answer was cut short (only so much of a command's output is
    // kept) and that piece is half a field.
    let whole = fields.pop() == Some("");
    let (path, records) = fields
        .split_first()
        .filter(|(path, _)| !path.is_empty())
        .context("The host's answer did not say which folder it is")?;
    // Whatever is left of an entry that was cut is not listed.
    let complete = records.len() - records.len() % 4;
    let mut cut = !whole || complete != records.len();
    let records = &records[..complete];
    let mut entries = Vec::new();
    for record in records.chunks_exact(4) {
        let is_dir = match record[0] {
            "d" => true,
            "f" => false,
            _ => bail!("The host's answer had an entry that is neither file nor folder"),
        };
        let number = |text: &str| text.trim().parse::<u64>().ok();
        let (Some(size), Some(modified)) = (number(record[1]), number(record[2])) else {
            bail!("The host's answer had a size or a time that is not a number");
        };
        entries.push(DirEntry {
            name: record[3].to_owned(),
            is_dir,
            size,
            modified_unix_millis: (modified > 0).then(|| modified.saturating_mul(1000)),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    cut |= entries.len() > ENTRY_LIMIT;
    entries.truncate(ENTRY_LIMIT);
    Ok(RemoteDirectory {
        alias: alias.to_owned(),
        path: (*path).to_owned(),
        entries,
        cut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(stdout: &str) -> RunCommandOutput {
        RunCommandOutput {
            exit_code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn a_path_is_handed_to_the_script_as_data() {
        assert!(script("").unwrap().contains("dir=\"$HOME\"\n"));
        assert!(script(" ~ ").unwrap().contains("dir=\"$HOME\"\n"));
        assert!(script("~/data/run 1")
            .unwrap()
            .contains("dir=\"$HOME\"/'data/run 1'\n"));
        // Quotes and commands in a name stay a name.
        assert!(script("/data/it's; rm -rf ~")
            .unwrap()
            .contains("dir='/data/it'\"'\"'s; rm -rf ~'\n"));
        assert!(script("/a\nrm -rf /").is_err());
        assert!(script(&"x".repeat(5000)).is_err());
        // The script never writes.
        let whole = script("/data").unwrap();
        for word in ["rm ", "mv ", "> ", "mkdir", "touch"] {
            assert!(!whole.contains(word), "{word}");
        }
    }

    #[test]
    fn the_answer_is_read_past_a_banner_with_folders_first() {
        let stdout = "Welcome to gpu01\nLast login: yesterday\n\
            WISP_REMOTE_DIR_V1\0/home/me/data\0\
            f\x00120\x001794636300\x00reads.fq\0\
            d\x000\x001794636000\x00results\0\
            f\x007\x000\x00a name\nwith a break\0\
            d\x000\x001794630000\x00Alignments\0";
        let listed = parse("gpu01", answer(stdout)).unwrap();
        assert_eq!(listed.alias, "gpu01");
        assert_eq!(listed.path, "/home/me/data");
        let names: Vec<_> = listed.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["Alignments", "results", "a name\nwith a break", "reads.fq"]
        );
        assert!(listed.entries[0].is_dir);
        assert_eq!(listed.entries[3].size, 120);
        assert_eq!(
            listed.entries[3].modified_unix_millis,
            Some(1_794_636_300_000)
        );
        // A time the host could not tell is no time.
        assert_eq!(listed.entries[2].modified_unix_millis, None);
        assert!(!listed.cut);
        // An empty folder is a folder.
        let empty = parse("gpu01", answer("WISP_REMOTE_DIR_V1\0/scratch\0")).unwrap();
        assert!(empty.entries.is_empty() && empty.path == "/scratch");
    }

    #[test]
    fn an_answer_that_is_not_the_scripts_is_refused() {
        let refused = |stdout: &str| parse("h", answer(stdout)).unwrap_err().to_string();
        assert!(refused("Permission denied").contains("not a folder listing"));
        assert!(refused("WISP_REMOTE_DIR_V1\0").contains("which folder"));
        // An answer that was cut short lists what is whole and says so.
        let short = parse(
            "h",
            answer("WISP_REMOTE_DIR_V1\0/x\0f\x001\x002\x00a.txt\0f\x003\x004\x00b.t"),
        )
        .unwrap();
        assert_eq!(short.entries.len(), 1);
        assert_eq!(short.entries[0].name, "a.txt");
        assert!(short.cut);
        assert!(refused("WISP_REMOTE_DIR_V1\0/x\0l\x001\x001\x00n\0").contains("neither"));
        assert!(refused("WISP_REMOTE_DIR_V1\0/x\0f\x00big\x001\x00n\0").contains("not a number"));
        let failed = RunCommandOutput {
            exit_code: 66,
            stdout: String::new(),
            stderr: "Cannot open the folder: /nope\n".into(),
        };
        assert!(parse("h", failed)
            .unwrap_err()
            .to_string()
            .ends_with("Cannot open the folder: /nope"));
    }
}
