//! File citations Codex writes into its answers,
//! `:codex-file-citation{path="..." purpose="output"}` (one to three
//! leading colons), turned into Markdown links before the answer is drawn
//! or exported, as the WebView desktop does (`ui/src/text.rs`).

use std::borrow::Cow;

const DIRECTIVE: &str = "codex-file-citation{";

/// `text` with each Codex file citation written as a Markdown link: the
/// file's name as its text, the path (as a `file:` address where it is an
/// absolute path, so a click opens it) as its target. Attributes may come
/// in any order and be quoted with either quote or not at all; a
/// citation without a path, an unknown directive and anything that does
/// not parse are left as they are.
pub fn file_citations_as_links(text: &str) -> Cow<'_, str> {
    if !text.contains(DIRECTIVE) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(at) = rest.find(DIRECTIVE) {
        let colons = rest[..at]
            .bytes()
            .rev()
            .take_while(|&byte| byte == b':')
            .count();
        if !(1..=3).contains(&colons) {
            out.push_str(&rest[..at + 1]);
            rest = &rest[at + 1..];
            continue;
        }
        let start = at - colons;
        let body = &rest[at + DIRECTIVE.len()..];
        match citation_path(body) {
            Some((path, used)) if !path.is_empty() => {
                out.push_str(&rest[..start]);
                out.push_str(&link(&path));
                rest = &body[used..];
                changed = true;
            }
            _ => {
                out.push_str(&rest[..at + DIRECTIVE.len()]);
                rest = body;
            }
        }
    }
    out.push_str(rest);
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

/// The path a citation's attributes give, and how much of `body` they
/// take up to and including the closing brace.
fn citation_path(body: &str) -> Option<(String, usize)> {
    let mut rest = body;
    let mut path = None;
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if let Some(after) = rest.strip_prefix('}') {
            return path.map(|path| (path, body.len() - after.len()));
        }
        if rest.is_empty() || rest.starts_with(['\n', '\r']) {
            return None;
        }
        let key_len = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
            .count();
        if key_len == 0 {
            return None;
        }
        let key = &rest[..key_len];
        rest = rest[key_len..].trim_start_matches([' ', '\t']);
        rest = rest.strip_prefix('=')?.trim_start_matches([' ', '\t']);
        let (value, after) = attribute_value(rest)?;
        if key == "path" {
            path = Some(value);
        }
        rest = after;
    }
}

fn attribute_value(rest: &str) -> Option<(String, &str)> {
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let inner = &rest[1..];
        let end = inner.find(quote)?;
        if inner[..end].contains(['\n', '\r']) {
            return None;
        }
        return Some((inner[..end].to_owned(), &inner[end + 1..]));
    }
    let end = rest
        .find([' ', '\t', '}', '\n', '\r'])
        .unwrap_or(rest.len());
    (end > 0).then(|| (rest[..end].to_owned(), &rest[end..]))
}

/// A Markdown link to `path`, named after the file.
fn link(path: &str) -> String {
    let name = path
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path);
    let name = name.replace('[', "\\[").replace(']', "\\]");
    format!("[{name}](<{}>)", target(path))
}

/// Where a link to `path` goes: a `file:` address for an absolute path
/// (`/…`, or a Windows drive `E:\…` / `E:/…`), with its separators as
/// slashes and spaces escaped; anything else as written.
fn target(path: &str) -> String {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    if !drive && !path.starts_with('/') {
        return path.to_owned();
    }
    let slashed = path
        .replace('\\', "/")
        .replace(' ', "%20")
        .replace('>', "%3E");
    if drive {
        format!("file:///{slashed}")
    } else {
        format!("file://{slashed}")
    }
}

#[cfg(test)]
mod tests {
    use super::file_citations_as_links as links;

    #[test]
    fn a_citation_becomes_a_link_named_after_its_file() {
        let text = concat!(
            r#":codex-file-citation{path="E:/cross-species-root/root-cap/results/panel_index.csv" purpose="output"}"#,
            "\n",
            r#"see ::codex-file-citation{purpose="output" path='results/layout_audit.json'} later"#,
        );
        assert_eq!(
            links(text),
            "[panel_index.csv](<file:///E:/cross-species-root/root-cap/results/panel_index.csv>)\n\
             see [layout_audit.json](<results/layout_audit.json>) later"
        );
    }

    #[test]
    fn windows_paths_and_unquoted_values_are_read() {
        assert_eq!(
            links(r#":codex-file-citation{path="E:\runs\my run\out.tsv"}"#),
            "[out.tsv](<file:///E:/runs/my%20run/out.tsv>)"
        );
        assert_eq!(
            links(":::codex-file-citation{ path = /home/me/a.txt purpose=output }."),
            "[a.txt](<file:///home/me/a.txt>)."
        );
        // A quote of the other kind inside the value stays in the path.
        assert_eq!(
            links(r#":codex-file-citation{path="E:/it's/x.csv"}"#),
            "[x.csv](<file:///E:/it's/x.csv>)"
        );
    }

    #[test]
    fn what_is_not_a_file_citation_is_left_alone() {
        for text in [
            "plain text",
            ":codex-other{path=\"a.txt\"}",
            ":codex-file-citation{purpose=\"output\"}",
            ":codex-file-citation{path=\"a.txt\"",
            "::::codex-file-citation{path=\"a.txt\"}",
            "codex-file-citation{path=\"a.txt\"}",
        ] {
            assert_eq!(links(text), text, "{text}");
        }
        assert!(matches!(links("plain"), std::borrow::Cow::Borrowed(_)));
    }
}
