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

/// `template`, already in the language in use (`fill(tr("…"), …)`), with
/// `values` set into it: each `{}`
/// takes the next value, and `{0}`, `{1}` take one by position, so a
/// translation may put them in another order. A place with no value for it
/// is left as written.
pub fn fill(template: &str, values: &[&dyn std::fmt::Display]) -> String {
    let mut filled = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        filled.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            rest = &rest[open..];
            break;
        };
        let place = &after[..close];
        let value = match place {
            "" => {
                next += 1;
                values.get(next - 1)
            }
            numbered => numbered.parse::<usize>().ok().and_then(|at| values.get(at)),
        };
        match value {
            Some(value) => filled.push_str(&value.to_string()),
            None => filled.push_str(&rest[open..open + close + 2]),
        }
        rest = &after[close + 1..];
    }
    filled.push_str(rest);
    filled
}

/// A message the host wrote, such as a refusal, in the language in use:
/// its translation when the table has this exact message, else itself.
/// Messages that carry a value (a path, an address) stay as written.
pub fn host(message: &str) -> String {
    host_in(current(), message)
}

fn host_in(locale: Locale, message: &str) -> String {
    match locale {
        Locale::En => message.to_owned(),
        Locale::Zh => chinese()
            .get(message)
            .map_or_else(|| message.to_owned(), |zh| (*zh).to_owned()),
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
    fn a_host_message_is_translated_when_the_table_has_it_exactly() {
        let refusal = "This decision model service needs an API key.";
        assert_eq!(host_in(Locale::En, refusal), refusal);
        assert_eq!(
            host_in(Locale::Zh, refusal),
            "此决策模型服务需要 API 密钥。"
        );
        assert_eq!(
            host_in(Locale::Zh, "The model did not return a draft"),
            "模型没有返回草稿"
        );
        // One that carries a value is left as the host wrote it.
        let named = "not a host: host is not a valid domain or URL";
        assert_eq!(host_in(Locale::Zh, named), named);
        // Text the app already put in Chinese goes through unchanged.
        assert_eq!(host_in(Locale::Zh, "模型没有返回草稿"), "模型没有返回草稿");
    }

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

    #[test]
    fn values_are_set_into_a_sentence_in_order_or_by_position() {
        assert_eq!(fill("Context {}%", &[&42]), "Context 42%");
        assert_eq!(
            fill("{} could not be read: {}", &[&"a.txt", &"denied"]),
            "a.txt could not be read: denied"
        );
        // A translation may turn the order round.
        assert_eq!(
            fill("{1} 中的 {0}", &[&"file", &"folder"]),
            "folder 中的 file"
        );
        // Too few values, or a brace that is not a place, is left alone.
        assert_eq!(fill("{} and {}", &[&"one"]), "one and {}");
        assert_eq!(fill("a { b", &[&1]), "a { b");
        assert_eq!(fill("{name}", &[&1]), "{name}");
        assert_eq!(fill("plain", &[]), "plain");
    }

    #[test]
    fn counts_and_sentences_with_values_read_in_chinese() {
        let zh = |text| in_locale(Locale::Zh, text);
        // A count says its number before the noun, whatever the English plural.
        assert_eq!(fill(zh("1 file"), &[]), "1 个文件");
        assert_eq!(fill(zh("{} files"), &[&4]), "4 个文件");
        assert_eq!(fill(zh("{} min {} s"), &[&3, &"05"]), "3 分 05 秒");
        // Where Chinese puts the whole first, the values go by position.
        assert_eq!(fill(zh("{} on of {}"), &[&2, &3]), "共 3 个，2 个开启");
        assert_eq!(
            fill(zh("{} checked of {}"), &[&1, &2]),
            "共 2 个，1 个已检查"
        );
        assert_eq!(
            fill(
                zh("{} is in {} in the project folder. Nothing was erased."),
                &[&"a.txt", &".trash"]
            ),
            "a.txt 已在项目文件夹的 .trash 中。没有删除任何内容。"
        );
        assert_eq!(
            fill(
                zh("{} messages, about {} tokens by length."),
                &[&13, &"4.3k"]
            ),
            "13 条消息，按长度估算约 4.3k 个 token。"
        );
    }

    #[test]
    fn a_translated_sentence_takes_the_values_its_english_takes() {
        let places = |text: &str| text.matches('{').count();
        for (english, chinese) in ZH {
            assert_eq!(places(english), places(chinese), "places differ: {english}");
        }
    }

    /// Every text the app passes to `tr` has a translation, so a new
    /// string cannot be added in one language only.
    #[test]
    fn every_text_the_app_translates_is_in_the_table() {
        let listed: std::collections::HashSet<&str> = ZH.iter().map(|(text, _)| *text).collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let sources = |folder: std::path::PathBuf| {
            std::fs::read_dir(folder)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|kind| kind == "rs"))
                // This file's own tests pass made-up text to `tr`.
                .filter(|path| !path.ends_with("i18n.rs"))
                .collect::<Vec<_>>()
        };
        let mut files = sources(root.clone());
        files.extend(sources(root.join("workspace")));
        assert!(files.len() > 40, "the sources were not found");
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
        while let Some(at) = rest.find("tr(") {
            // `tr(` and not the end of a longer name.
            let named = rest[..at]
                .chars()
                .next_back()
                .is_some_and(|before| before.is_alphanumeric() || before == '_');
            rest = &rest[at + 3..];
            // The literal may start on the next line, as rustfmt puts a
            // long one there.
            let Some(literal) = rest.trim_start().strip_prefix('"') else {
                continue;
            };
            rest = literal;
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
                      attr(\"no\"); tr(\"Say \\\"hi\\\"\"); tr(\n    \"Next line\"); tr(x)";
        assert_eq!(
            translated(source),
            ["Send", "Two lines", "Say \"hi\"", "Next line"]
        );
    }
}
