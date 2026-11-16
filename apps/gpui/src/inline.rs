//! Pictures an answer points at. When the model writes a Markdown image
//! whose target is a picture in the project, the picture is shown under
//! the answer; this finds those targets, and the pictures the turn's tools
//! wrote that the answer does not show itself.

use wisp_dto::native_conversations::Item;

/// How many pictures one answer may show.
pub const PER_MESSAGE: usize = 4;
/// How many pictures a window keeps loaded for the open project.
pub const KEPT: usize = 40;

/// The project-relative paths of the pictures `text` shows with Markdown
/// image syntax, in order, without repeats, at most [`PER_MESSAGE`]. Web
/// addresses, absolute paths and anything that leaves the project are not
/// pictures of the project; code is not prose.
pub fn pictures(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut rest = line;
        while let Some(start) = rest.find("![") {
            rest = &rest[start + 2..];
            let Some(close) = rest.find("](") else {
                break;
            };
            let after = &rest[close + 2..];
            let Some(end) = after.find(')') else {
                break;
            };
            if let Some(path) = project_picture(&after[..end]) {
                if !found.contains(&path) && found.len() < PER_MESSAGE {
                    found.push(path);
                }
            }
            rest = &after[end + 1..];
        }
    }
    found
}

/// The project pictures the turn that the answer at `index` closes wrote
/// with `write` or `edit`, as the WebView shows a turn's files under its
/// answer: only under the turn's last answer, leaving out those the answer
/// shows itself, in order, without repeats, and no more than
/// [`PER_MESSAGE`] together with those. A call that failed wrote nothing.
pub fn written(rows: &[Item], index: usize) -> Vec<String> {
    let Some(row) = rows.get(index) else {
        return Vec::new();
    };
    if row.role != "assistant" || row.text.trim().is_empty() {
        return Vec::new();
    }
    let answered_later = rows[index + 1..]
        .iter()
        .take_while(|row| row.role != "user")
        .any(|row| row.role == "assistant" && !row.text.trim().is_empty());
    if answered_later {
        return Vec::new();
    }
    let start = rows[..index]
        .iter()
        .rposition(|row| row.role == "user")
        .map_or(0, |at| at + 1);
    let shown = pictures(&row.text);
    let mut found: Vec<String> = Vec::new();
    for tool in &rows[start..index] {
        let writes = tool.role == "tool"
            && matches!(tool.tool_name.as_deref(), Some("write" | "edit"))
            && tool.ok != Some(false);
        if !writes {
            continue;
        }
        let path = tool
            .input
            .as_deref()
            .and_then(|input| serde_json::from_str::<serde_json::Value>(input).ok())
            .and_then(|input| input.get("path")?.as_str().map(str::to_owned))
            .and_then(|path| project_picture(&path));
        if let Some(path) = path {
            if !shown.contains(&path)
                && !found.contains(&path)
                && shown.len() + found.len() < PER_MESSAGE
            {
                found.push(path);
            }
        }
    }
    found
}

/// A Markdown image target as a project-relative picture path, if it is one.
fn project_picture(target: &str) -> Option<String> {
    // `path "title"` and `<path>` are both Markdown.
    let target = target.trim();
    let target = target
        .strip_prefix('<')
        .and_then(|inner| inner.split('>').next())
        .unwrap_or_else(|| target.split_whitespace().next().unwrap_or_default());
    let path = target.replace("%20", " ");
    let path = path.strip_prefix("./").unwrap_or(&path);
    let inside = !path.is_empty()
        && !path.contains("://")
        && !path.contains('\\')
        && !path.starts_with('/')
        && !path.starts_with("data:")
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != ".." && part != ".");
    (inside && crate::favorites::is_image(path)).then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        role: &str,
        tool: Option<&str>,
        input: Option<&str>,
        text: &str,
        ok: Option<bool>,
    ) -> Item {
        serde_json::from_value(serde_json::json!({
            "role": role, "text": text, "tool_name": tool, "input": input,
            "ok": ok, "status": null
        }))
        .unwrap()
    }

    #[test]
    fn the_pictures_a_turns_tools_wrote_go_under_its_last_answer() {
        let rows = [
            row("user", None, None, "plot it", None),
            row(
                "tool",
                Some("write"),
                Some(r#"{"path":"figs/a.png","content":"x"}"#),
                "ok",
                Some(true),
            ),
            row("assistant", None, None, "Writing the second one.", None),
            row(
                "tool",
                Some("edit"),
                Some(r#"{"path":"figs/b.svg"}"#),
                "ok",
                Some(true),
            ),
            row(
                "tool",
                Some("write"),
                Some(r#"{"path":"figs/c.png"}"#),
                "denied",
                Some(false),
            ),
            row(
                "tool",
                Some("write"),
                Some(r#"{"path":"notes.md"}"#),
                "ok",
                Some(true),
            ),
            row(
                "tool",
                Some("write"),
                Some(r#"{"path":"/tmp/d.png"}"#),
                "ok",
                Some(true),
            ),
            row(
                "tool",
                Some("shell"),
                Some(r#"{"path":"figs/e.png"}"#),
                "ok",
                Some(true),
            ),
            row("assistant", None, None, "Done: ![b](figs/b.svg)", None),
            row("user", None, None, "thanks", None),
            row("assistant", None, None, "Any time.", None),
        ];
        // Under the last answer: what was written, less what it shows.
        assert_eq!(written(&rows, 8), ["figs/a.png"]);
        // Not under an earlier answer of the same turn, a question, or a
        // turn that wrote nothing.
        assert!(written(&rows, 2).is_empty());
        assert!(written(&rows, 0).is_empty());
        assert!(written(&rows, 10).is_empty());
        assert!(written(&rows, 99).is_empty());
    }

    #[test]
    fn an_answer_shows_the_project_pictures_it_points_at() {
        let text = "Here is the plot:\n\n![volcano](figs/volcano.png)\n\n\
                    and two more ![a](./figs/a.jpg \"first\") ![b](<figs/my plot.svg>), \
                    again ![volcano](figs/volcano.png) and ![c](figs/with%20space.png).";
        assert_eq!(
            pictures(text),
            [
                "figs/volcano.png",
                "figs/a.jpg",
                "figs/my plot.svg",
                "figs/with space.png"
            ]
        );
        // Never more than a message may show.
        let many: String = (0..9).map(|n| format!("![p](figs/{n}.png) ")).collect();
        assert_eq!(pictures(&many).len(), PER_MESSAGE);
    }

    #[test]
    fn what_is_not_a_picture_in_the_project_is_left_alone() {
        for text in [
            "no pictures here",
            "a link [plot](figs/plot.png) is not an image",
            "![web](https://example.org/plot.png)",
            "![abs](/etc/plot.png)",
            "![up](../other/plot.png)",
            "![up](figs/../../plot.png)",
            "![win](figs\\plot.png)",
            "![data](data:image/png;base64,AAAA)",
            "![notes](notes/readme.md)",
            "![empty]()",
            "![open](figs/plot.png",
            "```\n![in code](figs/plot.png)\n```",
        ] {
            assert!(pictures(text).is_empty(), "{text}");
        }
        // After the code block ends, prose is prose again.
        assert_eq!(
            pictures("```\ncode\n```\n![after](figs/after.png)"),
            ["figs/after.png"]
        );
    }
}
