//! The interface in English and Chinese. The English text is written where
//! it is used and is also the key: `tr("Send")` returns it unchanged in
//! English and looks its translation up in Chinese, falling back to the
//! English when there is none yet. No graphics state; the language in use
//! is one value for the whole app.
use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

/// The language the interface is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    En,
    Zh,
}

/// What the user chose in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// Whatever the system is set to.
    #[default]
    System,
    English,
    Chinese,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::System, Language::English, Language::Chinese];

    /// Each language is named in itself, so it can be found by someone who
    /// cannot read the one that is showing.
    pub fn label(self) -> &'static str {
        match self {
            Language::System => "System",
            Language::English => "English",
            Language::Chinese => "中文",
        }
    }

    /// The word it is saved as.
    pub fn key(self) -> &'static str {
        match self {
            Language::System => "system",
            Language::English => "en",
            Language::Chinese => "zh",
        }
    }

    /// Reads a saved choice; anything else follows the system.
    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|choice| choice.key() == key.trim())
            .unwrap_or_default()
    }

    /// The language to show for this choice on a system set to `system`
    /// (a tag such as `zh-Hans-CN` or `en_US.UTF-8`).
    pub fn locale(self, system: Option<&str>) -> Locale {
        match self {
            Language::English => Locale::En,
            Language::Chinese => Locale::Zh,
            Language::System => match system {
                Some(tag) if tag.trim().to_ascii_lowercase().starts_with("zh") => Locale::Zh,
                _ => Locale::En,
            },
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Sets the language for everything drawn from now on.
pub fn set(locale: Locale) {
    CURRENT.store(locale as u8, Ordering::Relaxed);
}

pub fn current() -> Locale {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Locale::Zh,
        _ => Locale::En,
    }
}

/// Applies the user's choice, reading the system's language when it is
/// to be followed.
pub fn apply(choice: Language) {
    set(choice.locale(system_language().as_deref()));
}

/// The language the system is set to, as a tag, when it can be told. A
/// system it cannot be told on reads as English; the choice in Settings
/// is always there.
fn system_language() -> Option<String> {
    // macOS keeps it in the user's defaults, and gives an app started from
    // the Dock no `LANG`.
    #[cfg(target_os = "macos")]
    if let Ok(read) = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
    {
        let tag = String::from_utf8_lossy(&read.stdout).trim().to_owned();
        if read.status.success() && !tag.is_empty() {
            return Some(tag);
        }
    }
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|tag| !tag.trim().is_empty())
}

/// `text` in the language in use.
pub fn tr(text: &'static str) -> &'static str {
    in_locale(current(), text)
}

/// `text` in `locale`: itself in English, and in Chinese its translation
/// or, when there is none yet, itself.
pub fn in_locale(locale: Locale, text: &'static str) -> &'static str {
    match locale {
        Locale::En => text,
        Locale::Zh => chinese().get(text).copied().unwrap_or(text),
    }
}

fn chinese() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| crate::i18n_zh::ZH.iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n_zh::ZH;

    #[test]
    fn a_choice_is_saved_read_back_and_resolved_against_the_system() {
        for choice in Language::ALL {
            assert_eq!(Language::from_key(choice.key()), choice);
        }
        assert_eq!(Language::from_key("klingon"), Language::System);
        assert_eq!(Language::System.locale(Some("zh-Hans-CN")), Locale::Zh);
        assert_eq!(Language::System.locale(Some("ZH_TW.UTF-8")), Locale::Zh);
        assert_eq!(Language::System.locale(Some("en-US")), Locale::En);
        assert_eq!(Language::System.locale(Some("ja-JP")), Locale::En);
        assert_eq!(Language::System.locale(None), Locale::En);
        // A choice outranks the system.
        assert_eq!(Language::English.locale(Some("zh-CN")), Locale::En);
        assert_eq!(Language::Chinese.locale(Some("en-US")), Locale::Zh);
    }

    #[test]
    fn text_is_itself_in_english_and_translated_or_itself_in_chinese() {
        assert_eq!(in_locale(Locale::En, "Send"), "Send");
        assert_eq!(in_locale(Locale::Zh, "Send"), "发送");
        assert_eq!(
            in_locale(Locale::Zh, "No such text anywhere"),
            "No such text anywhere"
        );
    }

    #[test]
    fn the_table_has_each_text_once_and_says_something_for_it() {
        let mut seen = std::collections::HashSet::new();
        for (english, chinese) in ZH {
            assert!(seen.insert(*english), "listed twice: {english}");
            assert!(!chinese.trim().is_empty(), "no translation: {english}");
            // A translation that is the English again is a missing one.
            assert_ne!(english, chinese, "not translated: {english}");
            // What the English ends a sentence or opens a dialog with, the
            // Chinese does too.
            assert_eq!(
                english.ends_with('…'),
                chinese.ends_with('…'),
                "ellipsis differs: {english}"
            );
        }
    }

    /// Every text the views pass to `tr` has a translation, so a new
    /// string cannot be added in one language only.
    #[test]
    fn every_text_the_views_translate_is_in_the_table() {
        let listed: std::collections::HashSet<&str> = ZH.iter().map(|(text, _)| *text).collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut files = vec![root.join("workspace.rs")];
        files.extend(
            std::fs::read_dir(root.join("workspace"))
                .unwrap()
                .flatten()
                .map(|entry| entry.path()),
        );
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            for text in translated(&source) {
                if !listed.contains(text.as_str()) {
                    missing.push(format!("{}: {text}", file.display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "no Chinese for:\n{}",
            missing.join("\n")
        );
    }

    /// The string literals passed to `tr(` in `source`, as they read once
    /// compiled.
    fn translated(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find("tr(\"") {
            // `tr(` and not the end of a longer name.
            let named = rest[..at]
                .chars()
                .next_back()
                .is_some_and(|before| before.is_alphanumeric() || before == '_');
            rest = &rest[at + 4..];
            if named {
                continue;
            }
            let mut text = String::new();
            let mut chars = rest.chars();
            while let Some(ch) = chars.next() {
                match ch {
                    '"' => break,
                    '\\' => match chars.next() {
                        // A line continuation swallows the indentation.
                        Some('\n') => {
                            let after = chars.as_str().trim_start();
                            chars = after.chars();
                        }
                        Some('n') => text.push('\n'),
                        Some(other) => text.push(other),
                        None => break,
                    },
                    other => text.push(other),
                }
            }
            found.push(text);
        }
        found
    }

    #[test]
    fn the_scanner_reads_literals_the_way_the_compiler_does() {
        let source = "x.label(tr(\"Send\")).child(tr(\"Two \\\n             lines\")); \
                      attr(\"no\"); tr(\"Say \\\"hi\\\"\")";
        assert_eq!(translated(source), ["Send", "Two lines", "Say \"hi\""]);
    }
}
