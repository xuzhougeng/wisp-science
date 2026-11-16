//! One file of an SSH host, read for the Files tab to show, the way the
//! WebView desktop's preview reads it (`read_remote_file` in
//! `src-tauri/src/file_browser.rs`): text as its first megabyte with the
//! file's whole size, a picture whole up to the remote preview cap (32 MB),
//! refused on the host when it is larger. The scripts, their quoting and
//! the reading of their answers are `wisp_runs::remote_file`, shared with
//! the WebView. Nothing is kept: the bytes go to the window and no further,
//! so a preview is not a copy into the project.

use std::path::Path;
use wisp_runs::mime::{is_text_preview_bytes, mime_for_path};
use wisp_runs::remote_file::{
    read_remote_file_whole, read_remote_text_head, RemoteRunner, REMOTE_FILE_MAX_BYTES,
};

/// What a text preview reads of a file: its first megabyte, as the
/// WebView's text, code and table previews do.
pub const TEXT_HEAD: u64 = 1024 * 1024;
/// The largest picture fetched, as the WebView's image preview.
pub const PICTURE_LIMIT: u64 = REMOTE_FILE_MAX_BYTES;

/// A file of a host, as far as it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    pub alias: String,
    pub path: String,
    /// Its size on the host, when that is known.
    pub size: Option<u64>,
    pub body: RemoteFileBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteFileBody {
    /// Text, cut to its first megabyte when `truncated`.
    Text { text: String, truncated: bool },
    /// A picture, whole.
    Picture {
        content_type: String,
        bytes: Vec<u8>,
    },
    /// A picture larger than a preview fetches; nothing was fetched.
    TooLarge { limit: u64 },
    /// Not text and not a picture: copy it here to look at it.
    NotShown,
}

/// What is fetched of a file before anything is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    /// The whole picture, of this content type.
    Picture(&'static str),
    /// Nothing: the picture is larger than this.
    TooLarge(u64),
    /// Its first megabyte, to see whether it is text.
    Head,
    /// Nothing: no bytes of it could be shown here.
    NotShown,
}

/// What to fetch of `path`, whose size the folder listing said, when it
/// did. A picture by its name is fetched whole; a kind whose bytes are
/// never text (a PDF, an Office file, an archive by its name) is not
/// fetched at all; anything else has its head read.
pub(crate) fn plan(path: &str, listed_size: Option<u64>) -> Plan {
    if let Some(kind) = crate::desktop::image_type(Path::new(path)) {
        return match listed_size {
            Some(size) if size > PICTURE_LIMIT => Plan::TooLarge(PICTURE_LIMIT),
            _ => Plan::Picture(kind),
        };
    }
    // A text kind is text whatever its bytes; an unnamed one is when its
    // bytes are UTF-8, which the empty head is.
    if is_text_preview_bytes(mime_for_path(Path::new(path)), b"") {
        Plan::Head
    } else {
        Plan::NotShown
    }
}

/// Reads `path` of `context`'s host through `runner`, as [`plan`] says.
/// The host must already have been found ready for work.
pub(crate) fn fetch(
    alias: &str,
    context: &wisp_store::ExecutionContext,
    path: &str,
    listed_size: Option<u64>,
    runner: &mut dyn RemoteRunner,
) -> Result<RemoteFile, String> {
    let (size, body) = match plan(path, listed_size) {
        Plan::NotShown => (listed_size, RemoteFileBody::NotShown),
        Plan::TooLarge(limit) => (listed_size, RemoteFileBody::TooLarge { limit }),
        Plan::Picture(content_type) => {
            let bytes = read_remote_file_whole(context, path, PICTURE_LIMIT, runner)?;
            (
                Some(bytes.len() as u64),
                RemoteFileBody::Picture {
                    content_type: content_type.to_owned(),
                    bytes,
                },
            )
        }
        Plan::Head => {
            let (head, total) = read_remote_text_head(context, path, TEXT_HEAD, runner)?;
            let body = if is_text_preview_bytes(mime_for_path(Path::new(path)), &head) {
                RemoteFileBody::Text {
                    truncated: total > head.len() as u64,
                    text: String::from_utf8_lossy(&head).into_owned(),
                }
            } else {
                RemoteFileBody::NotShown
            };
            (Some(total), body)
        }
    };
    Ok(RemoteFile {
        alias: alias.to_owned(),
        path: path.to_owned(),
        size,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_runs::remote_file::{
        remote_file_script, remote_text_file_script, RemoteCommand, RemoteOutput,
    };

    struct Answers {
        outputs: Vec<RemoteOutput>,
        asked: Vec<RemoteCommand>,
    }

    impl RemoteRunner for Answers {
        fn run(&mut self, command: &RemoteCommand) -> Result<RemoteOutput, String> {
            self.asked.push(command.clone());
            assert!(!self.outputs.is_empty(), "asked more than was answered");
            Ok(self.outputs.remove(0))
        }
    }

    fn answering(stdout: &[u8]) -> Answers {
        Answers {
            outputs: vec![RemoteOutput {
                status: 0,
                stdout: stdout.to_vec(),
                stderr: String::new(),
            }],
            asked: Vec::new(),
        }
    }

    /// A key-signed host whose key is a temporary file: nothing of the
    /// user's is read, and the fake runner means nothing connects.
    fn host(dir: &tempfile::TempDir) -> wisp_store::ExecutionContext {
        let key = dir.path().join("id_test");
        std::fs::write(&key, b"test-key\n").unwrap();
        let mut context = wisp_store::ExecutionContext::new("ssh:gpu", "gpu").unwrap();
        context.config_json = serde_json::json!({
            "alias": "gpu.example",
            "user": "lab",
            "identity_file": key.to_string_lossy(),
        })
        .to_string();
        context.last_probe_status = Some("ok".into());
        context
    }

    #[test]
    fn what_is_fetched_follows_the_kind_and_the_listed_size() {
        assert_eq!(plan("/p/a.png", Some(10)), Plan::Picture("image/png"));
        assert_eq!(plan("/p/a.SVG", None), Plan::Picture("image/svg+xml"));
        assert_eq!(
            plan("/p/huge.tif.jpg", Some(PICTURE_LIMIT + 1)),
            Plan::TooLarge(PICTURE_LIMIT)
        );
        assert_eq!(
            plan("/p/exact.jpg", Some(PICTURE_LIMIT)),
            Plan::Picture("image/jpeg")
        );
        for text in [
            "/p/a.csv",
            "/p/a.tsv",
            "/p/notes.md",
            "/p/run.py",
            "/p/README",
            "/p/x.toml",
        ] {
            assert_eq!(plan(text, Some(u64::MAX)), Plan::Head, "{text}");
        }
        for never in ["/p/paper.pdf", "/p/t.docx", "/p/s.xlsx"] {
            assert_eq!(plan(never, Some(10)), Plan::NotShown, "{never}");
        }
    }

    #[test]
    fn text_is_its_first_megabyte_and_says_when_it_was_cut() {
        let dir = tempfile::tempdir().unwrap();
        let context = host(&dir);
        let path = "/data/it's; $(rm -rf ~) -x.csv";
        let mut runner = answering(b"motd\nWISP_REMOTE_FILE_TEXT_V1\x005000000\0gene,n\nA,1\n");
        let file = fetch("gpu", &context, path, Some(5_000_000), &mut runner).unwrap();
        assert_eq!(file.size, Some(5_000_000));
        assert_eq!(
            file.body,
            RemoteFileBody::Text {
                text: "gene,n\nA,1\n".into(),
                truncated: true
            }
        );
        // One ssh, whose remote command is the shared head script with the
        // name quoted as data.
        assert_eq!(runner.asked.len(), 1);
        let asked = &runner.asked[0];
        assert_eq!(asked.program, "ssh");
        assert_eq!(
            asked.args.last().unwrap(),
            &remote_text_file_script(path, TEXT_HEAD).unwrap()
        );
        assert!(asked
            .args
            .last()
            .unwrap()
            .contains("f='/data/it'\"'\"'s; $(rm -rf ~) -x.csv'\n"));
        // A whole short file is not cut.
        let mut whole = answering(b"WISP_REMOTE_FILE_TEXT_V1\x003\0hi\n");
        let file = fetch("gpu", &context, "~/a.txt", Some(3), &mut whole).unwrap();
        assert!(matches!(
            file.body,
            RemoteFileBody::Text {
                truncated: false,
                ..
            }
        ));
        // An unnamed kind whose head is not UTF-8 is not shown.
        let mut binary = answering(b"WISP_REMOTE_FILE_TEXT_V1\x004\0\0\x01\x02\x03");
        let file = fetch("gpu", &context, "/data/reads.bam", None, &mut binary).unwrap();
        assert_eq!(file.body, RemoteFileBody::NotShown);
        assert_eq!(file.size, Some(4));
    }

    #[test]
    fn a_picture_comes_whole_and_one_too_large_is_never_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let context = host(&dir);
        let png = b"\x89PNG\r\n\x1a\n\0\xff";
        let mut stdout = b"WISP_REMOTE_FILE_V1\0".to_vec();
        stdout.extend_from_slice(png);
        let mut runner = answering(&stdout);
        let file = fetch("gpu", &context, "/fig/a b.png", Some(10), &mut runner).unwrap();
        assert_eq!(
            file.body,
            RemoteFileBody::Picture {
                content_type: "image/png".into(),
                bytes: png.to_vec()
            }
        );
        assert_eq!(
            runner.asked[0].args.last().unwrap(),
            &remote_file_script("/fig/a b.png", PICTURE_LIMIT).unwrap()
        );
        // Over the cap by the listing: nothing is asked.
        let mut idle = Answers {
            outputs: Vec::new(),
            asked: Vec::new(),
        };
        let file = fetch(
            "gpu",
            &context,
            "/fig/big.png",
            Some(PICTURE_LIMIT + 1),
            &mut idle,
        )
        .unwrap();
        assert_eq!(
            file.body,
            RemoteFileBody::TooLarge {
                limit: PICTURE_LIMIT
            }
        );
        // Nor for a kind that is never shown.
        let file = fetch("gpu", &context, "/p/paper.pdf", Some(9), &mut idle).unwrap();
        assert_eq!(file.body, RemoteFileBody::NotShown);
        assert!(idle.asked.is_empty());
        // The host refusing it (it grew since the listing) is an error.
        let mut refused = Answers {
            outputs: vec![RemoteOutput {
                status: 67,
                stdout: Vec::new(),
                stderr: "Remote file exceeds 33554432 byte limit: /fig/a.png".into(),
            }],
            asked: Vec::new(),
        };
        assert!(fetch("gpu", &context, "/fig/a.png", Some(1), &mut refused)
            .unwrap_err()
            .contains("byte limit"));
        // A path with a line break never reaches the host.
        let mut idle = Answers {
            outputs: Vec::new(),
            asked: Vec::new(),
        };
        assert!(fetch("gpu", &context, "/a\nrm -rf /.txt", None, &mut idle).is_err());
        assert!(idle.asked.is_empty());
    }
}
