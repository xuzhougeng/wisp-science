//! The console on a conversation's interpreter: what was run and what it
//! gave back, and how a variable reads in its list.
use std::sync::Arc;

use gpui::{Image, ImageFormat};

/// One run in the console.
pub struct Entry {
    pub code: String,
    pub text: String,
    pub plots: Vec<Arc<Image>>,
}

/// Runs kept on screen; older ones go.
pub const KEPT: usize = 20;

impl Entry {
    pub fn new(code: &str, output: wisp_server::desktop::RuntimeOutput) -> Self {
        Self {
            code: code.trim().to_owned(),
            text: output.text.trim_end().to_owned(),
            plots: output
                .plots
                .into_iter()
                .map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
                .collect(),
        }
    }
}

/// Adds a run, keeping at most [`KEPT`].
pub fn push(entries: &mut Vec<Entry>, entry: Entry) {
    entries.push(entry);
    let extra = entries.len().saturating_sub(KEPT);
    entries.drain(..extra);
}

/// A variable as its row says: "df · DataFrame · 120 × 4 (1.2 KB)".
pub fn object_line(object: &wisp_server::desktop::RuntimeObject) -> String {
    let mut line = format!("{} · {}", object.name, object.type_name);
    if !object.summary.trim().is_empty() {
        line.push_str(&format!(" · {}", object.summary.trim()));
    }
    if let Some(bytes) = object.size_bytes {
        line.push_str(&format!(" ({})", crate::files::size_label(bytes)));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::{RuntimeObject, RuntimeOutput};

    #[test]
    fn the_console_keeps_the_latest_runs_and_names_each_variable() {
        let mut entries = Vec::new();
        for index in 0..(KEPT + 3) {
            push(
                &mut entries,
                Entry::new(
                    &format!(" x = {index} "),
                    RuntimeOutput {
                        text: format!("{index}\n"),
                        plots: vec![vec![0x89, b'P', b'N', b'G']],
                    },
                ),
            );
        }
        assert_eq!(entries.len(), KEPT);
        assert_eq!(entries[0].code, "x = 3");
        assert_eq!(entries.last().unwrap().text, format!("{}", KEPT + 2));
        assert_eq!(entries[0].plots.len(), 1);
        let object = RuntimeObject {
            name: "df".into(),
            type_name: "DataFrame".into(),
            summary: "120 × 4".into(),
            size_bytes: Some(2_048),
        };
        assert_eq!(object_line(&object), "df · DataFrame · 120 × 4 (2.0 KB)");
        let bare = RuntimeObject {
            summary: " ".into(),
            size_bytes: None,
            ..object
        };
        assert_eq!(object_line(&bare), "df · DataFrame");
    }
}
