//! `/share` support: turn the transcript into a selectable, redactable list
//! of messages that the share overlay exports as a long PNG or HTML page.
//! Both formats reuse the live chat CSS (tokens + `.msg` / `.thread` / `.md`)
//! so the export matches what the user sees.

use crate::dto::ChatItem;
#[cfg(test)]
use crate::dto::{ShareSocialHighlight, ShareSocialPlatform, ShareSocialVariant};
use serde_json::{json, Value};

/// Skill attached when the share dialog asks for social copy.
#[cfg(test)]
pub(crate) const SOCIAL_SHARE_SKILL: &str = "social-note";
/// How many highlight screenshots to prepare when the share dialog opens.
#[cfg(test)]
pub(crate) const MAX_SHARE_CARDS: usize = 3;
/// Pair the previous user turn with an assistant highlight when it is short.
#[cfg(test)]
const SHARE_CARD_USER_PAIR_LIMIT: usize = 280;

/// One screenshot card: a small slice of the transcript rendered as a PNG.
#[cfg(test)]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ShareCardSpec {
    pub(crate) title: String,
    pub(crate) indexes: Vec<usize>,
}

/// Export target chosen in the share overlay.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareExportFormat {
    Png,
    Html,
    #[allow(dead_code)]
    Social,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareRole {
    User,
    Assistant,
    Thinking,
}

impl ShareRole {
    /// Stable wire tag consumed by the JS canvas renderer.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            ShareRole::User => "user",
            ShareRole::Assistant => "assistant",
            ShareRole::Thinking => "thinking",
        }
    }
}

#[derive(Clone)]
pub(crate) struct ShareMessage {
    pub(crate) role: ShareRole,
    pub(crate) text: String,
    pub(crate) selected: bool,
}

/// Build the exportable message list from the transcript. Only conversation
/// content is shareable (user, assistant, and thinking rows); tool calls,
/// usage rows, and other machinery never appear in the share image. Thinking
/// rows are present but deselected — hidden from the export by default.
fn shareable_row(item: &ChatItem) -> Option<(ShareRole, &str, bool)> {
    match item {
        ChatItem::User(text) => Some((ShareRole::User, text.as_str(), true)),
        ChatItem::Assistant { text, .. } => Some((ShareRole::Assistant, text.as_str(), true)),
        ChatItem::Reasoning(text) => Some((ShareRole::Thinking, text.as_str(), false)),
        _ => None,
    }
}

/// True when the transcript has at least one user, assistant, or thinking
/// row that `/share` can put in the overlay.
pub(crate) fn transcript_has_shareable(items: &[ChatItem]) -> bool {
    items
        .iter()
        .any(|item| shareable_row(item).is_some_and(|(_, text, _)| !text.trim().is_empty()))
}

pub(crate) fn share_messages(items: &[ChatItem]) -> Vec<ShareMessage> {
    items
        .iter()
        .filter_map(shareable_row)
        .filter(|(_, text, _)| !text.trim().is_empty())
        .map(|(role, text, selected)| ShareMessage {
            role,
            text: text.trim().to_string(),
            selected,
        })
        .collect()
}

/// Prompt sent with `social-note` so the agent writes from the same redacted
/// selection the long-image export would use, for the platform the user picked.
/// Currently only reachable from tests: the social-copy flow stays hidden.
#[cfg(test)]
pub(crate) fn social_skill_prompt(
    selected: &[&ShareMessage],
    redact: &[String],
    platform: ShareSocialPlatform,
    zh: bool,
) -> String {
    let id = platform.as_str();
    let label = social_skill_platform_label(platform, zh);
    let mut excerpt = if zh {
        format!(
            "请按已附加的 {SOCIAL_SHARE_SKILL} 技能，把下面勾选的对话写成可直接发到{label}（{id}）的文案。不要改发到其他平台。\n\n对话摘录：\n"
        )
    } else {
        format!(
            "Follow the attached {SOCIAL_SHARE_SKILL} skill. Write paste-ready copy for {label} ({id}) from the selected turns below. Do not switch platforms.\n\nExcerpt:\n"
        )
    };
    for (index, message) in selected.iter().enumerate() {
        excerpt.push_str(&format!(
            "[{}] {}\n{}\n\n",
            index + 1,
            message.role.tag(),
            redact_text(&message.text, redact)
        ));
    }
    excerpt.push_str(if zh {
        "不要编造摘录里没有的数据、论文或结论。"
    } else {
        "Do not invent data, papers, or conclusions that are not in the excerpt."
    });
    excerpt
}

#[cfg(test)]
fn social_skill_platform_label(platform: ShareSocialPlatform, zh: bool) -> &'static str {
    match (platform, zh) {
        (ShareSocialPlatform::Xiaohongshu, true) => "小红书",
        (ShareSocialPlatform::Xiaohongshu, false) => "Xiaohongshu",
        (ShareSocialPlatform::Wechat, true) => "微信",
        (ShareSocialPlatform::Wechat, false) => "WeChat",
        (ShareSocialPlatform::WechatMp, true) => "微信公众号",
        (ShareSocialPlatform::WechatMp, false) => "WeChat official account",
        (ShareSocialPlatform::Twitter, true) => "Twitter / X",
        (ShareSocialPlatform::Twitter, false) => "Twitter / X",
    }
}

/// Pick 1–3 screenshot cards from the selected turns so the user does not
/// have to crop the conversation themselves. Prefers the latest assistant
/// replies and, when the previous user turn is short, keeps the Q&A pair.
#[cfg(test)]
pub(crate) fn share_key_cards(messages: &[ShareMessage]) -> Vec<ShareCardSpec> {
    let assistant: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.selected && message.role == ShareRole::Assistant)
        .map(|(index, _)| index)
        .collect();
    let mut picked: Vec<usize> = assistant
        .iter()
        .rev()
        .take(MAX_SHARE_CARDS)
        .copied()
        .collect();
    picked.reverse();
    if picked.is_empty() {
        return messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, message)| message.selected)
            .map(|(index, message)| {
                vec![ShareCardSpec {
                    title: share_card_title(&message.text),
                    indexes: vec![index],
                }]
            })
            .unwrap_or_default();
    }
    picked
        .into_iter()
        .map(|index| {
            let mut indexes = Vec::new();
            if let Some(previous) =
                messages[..index]
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(prev_index, message)| {
                        if message.role == ShareRole::Thinking {
                            return None;
                        }
                        Some((prev_index, message))
                    })
            {
                if previous.1.selected
                    && previous.1.role == ShareRole::User
                    && previous.1.text.chars().count() <= SHARE_CARD_USER_PAIR_LIMIT
                {
                    indexes.push(previous.0);
                }
            }
            indexes.push(index);
            ShareCardSpec {
                title: share_card_title(&messages[index].text),
                indexes,
            }
        })
        .collect()
}

/// Map 1-based excerpt indexes (`[1] user` in the model prompt) back onto
/// the selected rows of the full share draft.
#[cfg(test)]
pub(crate) fn map_excerpt_indexes(
    selected_draft_indexes: &[usize],
    excerpt_1based: &[usize],
) -> Vec<usize> {
    excerpt_1based
        .iter()
        .filter_map(|number| {
            let offset = number.checked_sub(1)?;
            selected_draft_indexes.get(offset).copied()
        })
        .collect()
}

/// Prefer model-picked highlight slices when they resolve to real rows.
#[cfg(test)]
pub(crate) fn share_cards_from_highlights(
    messages: &[ShareMessage],
    selected_draft_indexes: &[usize],
    highlights: &[ShareSocialHighlight],
) -> Vec<ShareCardSpec> {
    let mut cards = Vec::new();
    for highlight in highlights.iter().take(MAX_SHARE_CARDS) {
        let mut indexes = map_excerpt_indexes(selected_draft_indexes, &highlight.message_indexes);
        indexes.retain(|index| messages.get(*index).is_some_and(|message| message.selected));
        indexes.truncate(3);
        if indexes.is_empty() {
            continue;
        }
        let title = if highlight.title.trim().is_empty() {
            messages
                .get(*indexes.last().unwrap())
                .map(|message| share_card_title(&message.text))
                .unwrap_or_default()
        } else {
            highlight.title.trim().to_string()
        };
        cards.push(ShareCardSpec { title, indexes });
    }
    cards
}

/// Instant caption so the dialog is pasteable before the model returns.
#[cfg(test)]
pub(crate) fn share_fallback_caption(messages: &[ShareMessage], limit: usize) -> String {
    let raw = messages
        .iter()
        .rev()
        .find(|message| message.selected && message.role == ShareRole::Assistant)
        .or_else(|| messages.iter().rev().find(|message| message.selected))
        .map(|message| message.text.as_str())
        .unwrap_or("")
        .trim();
    clamp_share_chars(raw, limit)
}

#[cfg(test)]
fn share_card_title(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .trim_start_matches('#')
        .trim();
    clamp_share_chars(line, 36)
}

#[cfg(test)]
fn clamp_share_chars(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// Split the redaction input into keywords: comma (ASCII or fullwidth) and
/// newline separated, trimmed, deduplicated, longest first so a keyword is
/// never partially masked by one of its own substrings.
pub(crate) fn parse_redact_keywords(raw: &str) -> Vec<String> {
    let mut keywords: Vec<String> = raw
        .split(|c| c == ',' || c == '，' || c == '\n')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect();
    keywords.sort_by_key(|k| std::cmp::Reverse(k.chars().count()));
    keywords.dedup();
    let mut seen = std::collections::HashSet::new();
    keywords.retain(|k| seen.insert(k.clone()));
    keywords
}

/// Replace every (case-insensitive) occurrence of each keyword with `xxx`.
pub(crate) fn redact_text(text: &str, keywords: &[String]) -> String {
    keywords
        .iter()
        .filter(|k| !k.is_empty())
        .fold(text.to_string(), |acc, keyword| {
            replace_ci(&acc, keyword, "xxx")
        })
}

/// Case-insensitive literal replacement. Matches on full Unicode lowercase
/// equality so `Alice`, `ALICE`, and `alice` all mask with one keyword.
fn replace_ci(haystack: &str, needle: &str, replacement: &str) -> String {
    let needle_lower: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle_lower.is_empty() {
        return haystack.to_string();
    }
    let mut out = String::with_capacity(haystack.len());
    let mut rest = haystack;
    'outer: while !rest.is_empty() {
        if let Some(len) = ci_prefix_len(rest, &needle_lower) {
            out.push_str(replacement);
            rest = &rest[len..];
            continue 'outer;
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// Byte length of a prefix of `text` whose lowercase form equals
/// `needle_lower`, or `None` when `text` does not start with the needle.
fn ci_prefix_len(text: &str, needle_lower: &[char]) -> Option<usize> {
    let mut expected = needle_lower.iter();
    let mut len = 0;
    for ch in text.chars() {
        for lower in ch.to_lowercase() {
            if expected.next() != Some(&lower) {
                return None;
            }
        }
        len += ch.len_utf8();
        if expected.len() == 0 {
            return Some(len);
        }
    }
    None
}

// --- HTML export -------------------------------------------------------------

/// One selected row of the HTML export: a role, its localized label, and the
/// (already redacted) message text.
pub(crate) struct ShareHtmlRow {
    pub(crate) role: ShareRole,
    pub(crate) label: String,
    pub(crate) text: String,
}

/// Live theme captured from the running app so the standalone HTML matches
/// the current palette, font size, and conversation CSS instead of a second
/// skin. Empty harvested CSS falls back to `share-export.css`.
#[derive(Clone, Default, serde::Deserialize)]
pub(crate) struct ShareHtmlTheme {
    #[serde(default)]
    pub(crate) lang: String,
    #[serde(default)]
    pub(crate) root_css: String,
    #[serde(default)]
    pub(crate) harvested_css: String,
}

/// Minimal escaping for plain-text rows and metadata; assistant Markdown goes
/// through `md_to_html`, which is trusted the same way chat rendering is.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Fallback conversation stylesheet. Keep message/markdown declarations in
/// lockstep with `chat.css` / `base.css`; live export overlays harvested rules.
const SHARE_EXPORT_CSS: &str = include_str!("../styles/share-export.css");

/// Layout invariants that must win over the live stylesheet appended to an
/// export. The live rules provide the exact chat appearance, but they come
/// from an application viewport that clips horizontal overflow at `.chat`.
/// A standalone document instead keeps its content column bounded and wraps
/// long prose tokens while leaving intrinsically wide blocks scrollable.
const SHARE_EXPORT_LAYOUT_GUARDS: &str = r#"
.share-page, .thread, .msg, .user-bubble, .assistant-wrap, .msg .body {
  min-width: 0; max-width: 100%; box-sizing: border-box;
}
.share-page { width: 100%; }
.md :where(p, li, a, blockquote, h1, h2, h3, h4, h5, th, td) {
  min-width: 0; max-width: 100%; overflow-wrap: anywhere; word-break: break-word;
}
.md :where(pre, table, .math-display) { max-width: 100%; overflow-x: auto; }
"#;

/// Prevent a harvested rule from closing the inline `<style>` block.
fn sanitize_css(css: &str) -> String {
    css.replace("</", "<\\/")
}

/// Join a generated variant into the text the user pastes into a social app.
/// Title is omitted when it already prefixes the body; hashtags are appended
/// only when they are not already in the body.
#[cfg(test)]
pub(crate) fn share_social_pack_text(variant: &ShareSocialVariant) -> String {
    let title = variant.title.trim();
    let body = variant.body.trim();
    let mut parts = Vec::new();
    if !title.is_empty() && !body.starts_with(title) {
        parts.push(title.to_string());
    }
    if !body.is_empty() {
        parts.push(body.to_string());
    }
    let tags: Vec<String> = variant
        .hashtags
        .iter()
        .map(|tag| normalize_share_hashtag(tag))
        .filter(|tag| !tag.is_empty())
        .collect();
    if !tags.is_empty() {
        let joined = tags.join(" ");
        if !body.contains(&joined) && tags.iter().any(|tag| !body.contains(tag.as_str())) {
            parts.push(joined);
        }
    }
    parts.join("\n\n")
}

#[cfg(test)]
pub(crate) fn normalize_share_hashtag(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('#').trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("#{trimmed}")
    }
}

/// Default canvas width for the long PNG and the accepted input range.
pub(crate) const SHARE_PNG_DEFAULT_WIDTH: u32 = 840;
pub(crate) const SHARE_PNG_MIN_WIDTH: u32 = 320;
pub(crate) const SHARE_PNG_MAX_WIDTH: u32 = 2400;

/// Parse the width field in the share dialog: blank or unparsable input falls
/// back to the default; numbers are clamped to the supported range.
pub(crate) fn share_png_width(raw: &str) -> u32 {
    raw.trim()
        .parse::<u32>()
        .unwrap_or(SHARE_PNG_DEFAULT_WIDTH)
        .clamp(SHARE_PNG_MIN_WIDTH, SHARE_PNG_MAX_WIDTH)
}

/// JSON payload consumed by the JS canvas renderer for the long PNG.
pub(crate) fn share_png_payload(
    title: &str,
    subtitle: &str,
    footer: &str,
    rows: &[Value],
    width: u32,
) -> String {
    json!({
        "title": title,
        "subtitle": subtitle,
        "footer": footer,
        "width": width,
        "messages": rows,
    })
    .to_string()
}

/// One selected row for the PNG renderer. Assistant Markdown is sent as the
/// same HTML chat uses (`md_to_html`) so the canvas can rasterize tables and
/// KaTeX instead of flattening them. `blocks` stay as a fallback if the HTML
/// snapshot fails.
pub(crate) fn share_png_row(role: ShareRole, label: &str, text: &str) -> Value {
    let mut row = json!({
        "kind": role.tag(),
        "label": label,
    });
    if role == ShareRole::Assistant {
        row["html"] = json!(crate::text::md_to_html(text));
        row["blocks"] = json!(share_markdown_blocks(text));
    } else {
        row["text"] = json!(text);
    }
    row
}

/// Build a self-contained HTML document of the selected conversation using
/// the same DOM as chat (`.thread` / `.msg` / `.body.md`). Assistant Markdown
/// is rendered the same way as the live thread. `theme` freezes live `:root`
/// tokens and can append harvested chat/md rules.
pub(crate) fn share_html_document(
    title: &str,
    subtitle: &str,
    footer: &str,
    messages: &[ShareHtmlRow],
    theme: &ShareHtmlTheme,
) -> String {
    use std::fmt::Write;
    let lang = if theme.lang.trim().is_empty() {
        "en"
    } else {
        theme.lang.trim()
    };
    let mut out = String::with_capacity(8192);
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{}\n",
        escape_html(lang),
        escape_html(title),
        SHARE_EXPORT_CSS,
    );
    if !theme.root_css.trim().is_empty() {
        let _ = write!(
            out,
            "\n:root {{ {} }}\n",
            sanitize_css(theme.root_css.trim())
        );
    }
    if !theme.harvested_css.trim().is_empty() {
        out.push('\n');
        out.push_str(&sanitize_css(theme.harvested_css.trim()));
        out.push('\n');
    }
    out.push_str(SHARE_EXPORT_LAYOUT_GUARDS);
    let _ = write!(
        out,
        "</style>\n</head>\n<body>\n<main class=\"share-page\">\n\
         <header class=\"share-head\">\n<h1>{}</h1>\n<p class=\"share-date\">{}</p>\n</header>\n\
         <div class=\"thread\">\n",
        escape_html(title),
        escape_html(subtitle),
    );
    for message in messages {
        match message.role {
            ShareRole::User => {
                let _ = write!(
                    out,
                    "<article class=\"msg user\">\n<div class=\"role\">{}</div>\n\
                     <div class=\"user-bubble\"><div class=\"body\">{}</div></div>\n</article>\n",
                    escape_html(&message.label),
                    escape_html(&message.text),
                );
            }
            ShareRole::Assistant => {
                let _ = write!(
                    out,
                    "<article class=\"msg assistant\">\n<div class=\"role\"><span class=\"role-brand\">{}</span></div>\n\
                     <div class=\"assistant-wrap\"><div class=\"body md\">{}</div></div>\n</article>\n",
                    escape_html(&message.label),
                    crate::text::md_to_html(&message.text),
                );
            }
            ShareRole::Thinking => {
                let _ = write!(
                    out,
                    "<article class=\"msg reasoning\">\n<div class=\"role\">{}</div>\n\
                     <div class=\"body\">{}</div>\n</article>\n",
                    escape_html(&message.label),
                    escape_html(&message.text),
                );
            }
        }
    }
    let _ = write!(
        out,
        "</div>\n<footer class=\"share-foot\">{}</footer>\n</main>\n</body>\n</html>\n",
        escape_html(footer),
    );
    out
}

// --- Markdown → canvas blocks ----------------------------------------------

/// Inline style flags carried by each text run. The JS canvas renderer maps
/// them to font weight/style/family and link color.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    code: bool,
    link: bool,
}

/// Which container block is currently collecting inline runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CurrentBlock {
    Heading(u8),
    Paragraph,
    Item { index: Option<u64> },
}

/// GFM table being flattened into monospace lines (`cell | cell` per row).
struct TableState {
    lines: Vec<String>,
    row: Vec<String>,
    cell: String,
}

/// Parse assistant Markdown into the flat block list used only when the HTML
/// snapshot path cannot run. Headings, paragraphs, lists, quotes, code, and
/// rules are preserved; tables stay flattened to monospace lines. Prefer the
/// `html` field on the PNG row so the export matches chat (tables + KaTeX).
pub(crate) fn share_markdown_blocks(text: &str) -> Vec<Value> {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_MATH);

    let mut blocks: Vec<Value> = vec![];
    let mut runs: Vec<(String, InlineStyle)> = vec![];
    let mut style = InlineStyle::default();
    let mut current: Option<CurrentBlock> = None;
    // Tight lists omit Paragraph tags; loose lists nest them inside items.
    // A Paragraph start while an item is open is structural, not a new block.
    let mut item_para = false;
    let mut code_buf: Option<String> = None;
    let mut quote_depth = 0usize;
    let mut lists: Vec<Option<u64>> = vec![];
    let mut table: Option<TableState> = None;
    let mut image_alt: Option<String> = None;

    let push_run = |runs: &mut Vec<(String, InlineStyle)>, text: &str, style: InlineStyle| {
        if text.is_empty() {
            return;
        }
        if let Some((last_text, last_style)) = runs.last_mut() {
            if *last_style == style {
                last_text.push_str(text);
                return;
            }
        }
        runs.push((text.to_string(), style));
    };

    macro_rules! flush_runs {
        () => {{
            let value = runs_to_json(&runs);
            runs.clear();
            value
        }};
    }

    for event in Parser::new_ext(text, opts) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if matches!(current, Some(CurrentBlock::Item { .. })) {
                        item_para = true;
                    } else {
                        current = Some(CurrentBlock::Paragraph);
                    }
                }
                Tag::Heading { level, .. } => {
                    let level = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        _ => 3,
                    };
                    current = Some(CurrentBlock::Heading(level));
                }
                Tag::BlockQuote(..) => quote_depth += 1,
                Tag::CodeBlock(_) => code_buf = Some(String::new()),
                Tag::List(start) => lists.push(start),
                Tag::Item => {
                    let index = match lists.last_mut() {
                        Some(Some(next)) => {
                            let i = *next;
                            *next += 1;
                            Some(i)
                        }
                        _ => None,
                    };
                    current = Some(CurrentBlock::Item { index });
                }
                Tag::Strong => style.bold = true,
                Tag::Emphasis => style.italic = true,
                Tag::Link { .. } => style.link = true,
                Tag::Image { .. } => image_alt = Some(String::new()),
                Tag::Table(_) => {
                    table = Some(TableState {
                        lines: vec![],
                        row: vec![],
                        cell: String::new(),
                    })
                }
                Tag::TableRow | Tag::TableHead => {
                    if let Some(t) = table.as_mut() {
                        t.row.clear();
                    }
                }
                Tag::TableCell => {
                    if let Some(t) = table.as_mut() {
                        t.cell.clear();
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    if item_para {
                        item_para = false;
                    } else if matches!(current, Some(CurrentBlock::Paragraph)) {
                        let value = flush_runs!();
                        if !value.is_empty() {
                            blocks.push(json!({"t": "p", "quote": quote_depth > 0, "runs": value}));
                        }
                        current = None;
                    }
                }
                TagEnd::Heading(_) => {
                    if let Some(CurrentBlock::Heading(level)) = current {
                        let value = flush_runs!();
                        if !value.is_empty() {
                            blocks.push(json!({"t": "h", "level": level, "runs": value}));
                        }
                        current = None;
                    }
                }
                TagEnd::BlockQuote(..) => quote_depth = quote_depth.saturating_sub(1),
                TagEnd::CodeBlock => {
                    if let Some(buf) = code_buf.take() {
                        blocks.push(json!({"t": "code", "text": buf.trim_end_matches('\n')}));
                    }
                }
                TagEnd::List(_) => {
                    lists.pop();
                }
                TagEnd::Item => {
                    if let Some(CurrentBlock::Item { index }) = current {
                        let value = flush_runs!();
                        if !value.is_empty() {
                            blocks.push(json!({
                                "t": "li",
                                "ordered": index.is_some(),
                                "index": index.unwrap_or(0),
                                "depth": lists.len(),
                                "quote": quote_depth > 0,
                                "runs": value,
                            }));
                        }
                        current = None;
                    }
                }
                TagEnd::Strong => style.bold = false,
                TagEnd::Emphasis => style.italic = false,
                TagEnd::Link => style.link = false,
                TagEnd::Image => {
                    if let Some(alt) = image_alt.take() {
                        let label = if alt.trim().is_empty() {
                            "[image]".to_string()
                        } else {
                            format!("[{}]", alt.trim())
                        };
                        push_run(
                            &mut runs,
                            &label,
                            InlineStyle {
                                code: true,
                                ..style
                            },
                        );
                    }
                }
                TagEnd::Table => {
                    if let Some(t) = table.take() {
                        blocks.push(json!({"t": "code", "text": t.lines.join("\n")}));
                    }
                }
                TagEnd::TableRow | TagEnd::TableHead => {
                    if let Some(t) = table.as_mut() {
                        t.lines.push(t.row.join(" | "));
                    }
                }
                TagEnd::TableCell => {
                    if let Some(t) = table.as_mut() {
                        let cell = t.cell.trim().to_string();
                        t.row.push(cell);
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some(alt) = image_alt.as_mut() {
                    alt.push_str(&text);
                } else if let Some(t) = table.as_mut() {
                    t.cell.push_str(&text);
                } else if let Some(buf) = code_buf.as_mut() {
                    buf.push_str(&text);
                } else {
                    push_run(&mut runs, &text, style);
                }
            }
            Event::Code(text) => {
                if let Some(t) = table.as_mut() {
                    t.cell.push_str(&text);
                } else {
                    push_run(
                        &mut runs,
                        &text,
                        InlineStyle {
                            code: true,
                            ..style
                        },
                    );
                }
            }
            Event::InlineMath(text) => {
                push_run(
                    &mut runs,
                    &text,
                    InlineStyle {
                        code: true,
                        ..style
                    },
                );
            }
            Event::DisplayMath(text) => {
                if current.is_some() {
                    // `$$...$$` inside a paragraph stays inline.
                    push_run(
                        &mut runs,
                        &text,
                        InlineStyle {
                            code: true,
                            ..style
                        },
                    );
                } else {
                    // Display math is its own block (no Paragraph wrapper), so
                    // it must not leak into the inline run buffer.
                    blocks.push(json!({"t": "code", "text": text.as_ref()}));
                }
            }
            Event::SoftBreak => push_run(&mut runs, " ", style),
            Event::HardBreak => push_run(&mut runs, "\n", style),
            Event::Rule => blocks.push(json!({"t": "hr"})),
            Event::TaskListMarker(checked) => {
                push_run(&mut runs, if checked { "☑ " } else { "☐ " }, style);
            }
            Event::FootnoteReference(name) => {
                push_run(&mut runs, &format!("[{name}]"), style);
            }
            _ => {}
        }
    }
    blocks
}

/// Serialize inline runs, trimming surrounding whitespace and dropping runs
/// that carry no visible text.
fn runs_to_json(runs: &[(String, InlineStyle)]) -> Vec<Value> {
    let mut out: Vec<Value> = runs
        .iter()
        .map(|(text, style)| {
            let mut run = json!({"text": text});
            if style.bold {
                run["b"] = json!(true);
            }
            if style.italic {
                run["i"] = json!(true);
            }
            if style.code {
                run["c"] = json!(true);
            }
            if style.link {
                run["a"] = json!(true);
            }
            run
        })
        .collect();
    // Trim leading/trailing whitespace so bubbles do not start or end with a
    // stray space from source formatting.
    if let Some(first) = out.first_mut() {
        if let Some(text) = first["text"].as_str() {
            first["text"] = json!(text.trim_start());
        }
    }
    if let Some(last) = out.last_mut() {
        if let Some(text) = last["text"].as_str() {
            last["text"] = json!(text.trim_end());
        }
    }
    out.retain(|run| !run["text"].as_str().unwrap_or("").is_empty());
    out
}

#[cfg(test)]
mod share_tests {
    use super::*;

    fn assistant(text: &str) -> ChatItem {
        ChatItem::Assistant {
            text: text.into(),
            model: None,
            resources: vec![],
        }
    }

    #[test]
    fn builds_shareable_rows_with_thinking_deselected() {
        let items = vec![
            ChatItem::User("检查这个峰".into()),
            ChatItem::Reasoning("先比对参考谱库".into()),
            ChatItem::Tool {
                name: "shell".into(),
                ok: Some(true),
                input: "ls".into(),
                output: "ok".into(),
                started_at_ms: None,
                duration_ms: None,
            },
            assistant("这是主峰的解释。"),
            ChatItem::Reasoning("   ".into()),
            assistant(""),
        ];
        let rows = share_messages(&items);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].role, ShareRole::User) && rows[0].selected);
        assert!(matches!(rows[1].role, ShareRole::Thinking) && !rows[1].selected);
        assert!(matches!(rows[2].role, ShareRole::Assistant) && rows[2].selected);
        assert!(transcript_has_shareable(&items));
        assert!(!transcript_has_shareable(&[]));
        let selected: Vec<&ShareMessage> = rows.iter().filter(|row| row.selected).collect();
        let prompt = social_skill_prompt(
            &selected,
            &parse_redact_keywords("主峰"),
            ShareSocialPlatform::Wechat,
            true,
        );
        assert!(prompt.contains(SOCIAL_SHARE_SKILL));
        assert!(prompt.contains("微信（wechat）"));
        assert!(!prompt.contains("小红书"));
        assert!(prompt.contains("[1] user"));
        assert!(prompt.contains("xxx的解释"));
        assert!(!prompt.contains("先比对参考谱库"));
        let english = social_skill_prompt(&selected, &[], ShareSocialPlatform::Twitter, false);
        assert!(english.contains("Twitter / X (twitter)"));
        assert!(english.contains("Do not switch platforms"));
        assert!(!transcript_has_shareable(&[ChatItem::Tool {
            name: "shell".into(),
            ok: Some(true),
            input: "ls".into(),
            output: "ok".into(),
            started_at_ms: None,
            duration_ms: None,
        }]));
    }

    #[test]
    fn parses_keywords_longest_first_without_duplicates() {
        let keywords = parse_redact_keywords("  alice , bob，alice\nalice smith,, ");
        assert_eq!(keywords, vec!["alice smith", "alice", "bob"]);
    }

    #[test]
    fn redacts_case_insensitively_and_in_cjk_text() {
        let keywords = parse_redact_keywords("Alice,张三");
        assert_eq!(
            redact_text("ALICE told alice about 张三 and Alicete", &keywords),
            "xxx told xxx about xxx and xxxte"
        );
        assert_eq!(redact_text("张三丰不是张三", &keywords), "xxx丰不是xxx");
    }

    #[test]
    fn longer_keywords_mask_before_their_substrings() {
        let keywords = parse_redact_keywords("alice smith, alice");
        assert_eq!(
            redact_text("alice smith met alice", &keywords),
            "xxx met xxx"
        );
    }

    #[test]
    fn empty_keywords_leave_text_untouched() {
        assert_eq!(redact_text("nothing to hide", &[]), "nothing to hide");
        assert!(parse_redact_keywords(" ,，\n").is_empty());
    }

    #[test]
    fn png_width_falls_back_and_clamps() {
        assert_eq!(share_png_width(""), SHARE_PNG_DEFAULT_WIDTH);
        assert_eq!(share_png_width("  "), SHARE_PNG_DEFAULT_WIDTH);
        assert_eq!(share_png_width("abc"), SHARE_PNG_DEFAULT_WIDTH);
        assert_eq!(share_png_width("640"), 640);
        assert_eq!(share_png_width(" 640 "), 640);
        assert_eq!(share_png_width("10"), SHARE_PNG_MIN_WIDTH);
        assert_eq!(share_png_width("99999"), SHARE_PNG_MAX_WIDTH);
    }

    #[test]
    fn png_row_includes_chat_html_for_tables_and_math() {
        let row = share_png_row(
            ShareRole::Assistant,
            "助手",
            "| 项目 | 数值 |\n| --- | --- |\n| A | 1 |\n| B | 2 |\n\n质能 $E = mc^2$\n\n$$\\int_0^1 x^2 dx$$\n",
        );
        let html = row["html"]
            .as_str()
            .expect("assistant row should carry chat HTML");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<th>"), "{html}");
        assert!(html.contains("项目"), "{html}");
        assert!(html.contains(r#"class="math math-inline""#), "{html}");
        assert!(html.contains(r#"class="math math-display""#), "{html}");
        assert!(html.contains("E = mc^2"), "{html}");
        assert!(row["blocks"].is_array());
    }

    #[test]
    fn png_payload_carries_the_requested_width() {
        let payload: Value = serde_json::from_str(&share_png_payload(
            "wisp-science",
            "2026-08-19",
            "footer",
            &[share_png_row(ShareRole::User, "You", "hi")],
            640,
        ))
        .unwrap();
        assert_eq!(payload["width"], json!(640));
        assert_eq!(payload["messages"][0]["kind"], json!("user"));
    }

    #[test]
    fn markdown_blocks_cover_headings_styles_and_lists() {
        let blocks = share_markdown_blocks(
            "## 拟合结果\n\n**主峰** 在 *530 nm*，见 `fit()` 和 [报告](https://x)。\n\n1. 第一步\n2. 第二步\n\n- 备注\n",
        );
        assert_eq!(blocks[0]["t"], json!("h"));
        assert_eq!(blocks[0]["level"], json!(2));
        let runs = blocks[1]["runs"].as_array().unwrap();
        assert_eq!(runs[0], json!({"text": "主峰", "b": true}));
        assert_eq!(runs[2], json!({"text": "530 nm", "i": true}));
        assert!(runs
            .iter()
            .any(|r| r["c"] == json!(true) && r["text"] == "fit()"));
        assert!(runs
            .iter()
            .any(|r| r["a"] == json!(true) && r["text"] == "报告"));
        assert_eq!(blocks[2]["t"], json!("li"));
        assert_eq!(blocks[2]["ordered"], json!(true));
        assert_eq!(blocks[2]["index"], json!(1));
        assert_eq!(blocks[3]["index"], json!(2));
        assert_eq!(blocks[4]["ordered"], json!(false));
    }

    #[test]
    fn markdown_blocks_flatten_code_quotes_rules_and_tables() {
        let blocks = share_markdown_blocks(
            "> 引用一句\n\n```python\nfit(x)\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        assert_eq!(
            blocks[0],
            json!({"t": "p", "quote": true, "runs": [{"text": "引用一句"}]})
        );
        assert_eq!(blocks[1], json!({"t": "code", "text": "fit(x)"}));
        assert_eq!(blocks[2], json!({"t": "hr"}));
        assert_eq!(blocks[3]["t"], json!("code"));
        assert_eq!(blocks[3]["text"], json!("a | b\n1 | 2"));
    }

    #[test]
    fn html_document_renders_markdown_and_escapes_plain_rows() {
        let rows = vec![
            ShareHtmlRow {
                role: ShareRole::User,
                label: "我".into(),
                text: "<script>alert(1)</script>\n第二行".into(),
            },
            ShareHtmlRow {
                role: ShareRole::Assistant,
                label: "助手".into(),
                text: "结果 **加粗** 见 `fit()`。\n\n## 小结\n- 一项\n".into(),
            },
            ShareHtmlRow {
                role: ShareRole::Thinking,
                label: "思考".into(),
                text: "a & b".into(),
            },
        ];
        let html = share_html_document(
            "wisp-science",
            "2026-08-14",
            "Shared",
            &rows,
            &ShareHtmlTheme::default(),
        );
        assert!(html.contains("<title>wisp-science</title>"));
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("<article class=\"msg user\">"));
        assert!(html.contains("user-bubble"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("<article class=\"msg assistant\">"));
        assert!(html.contains("body md"));
        assert!(html.contains("<strong>加粗</strong>"));
        assert!(html.contains("<h2>小结</h2>"));
        assert!(html.contains("<li>一项</li>"));
        assert!(html.contains("<article class=\"msg reasoning\">"));
        assert!(html.contains("a &amp; b"));
        assert!(html.contains("<footer class=\"share-foot\">Shared</footer>"));
        assert!(html.contains("--bg-panel"));
        assert!(html.contains("18px 18px 6px 18px"));
        assert!(!html.contains("#2f6fed"));
        assert!(!html.contains("class=\"card\""));
        assert!(!html.contains("class=\"bubble\""));
        let themed = share_html_document(
            "wisp-science",
            "2026-08-14",
            "Shared",
            &rows,
            &ShareHtmlTheme {
                lang: "zh".into(),
                root_css: "--bg-app: rgb(23, 22, 20)".into(),
                harvested_css: ".thread { gap: 20px; }".into(),
            },
        );
        assert!(themed.contains("<html lang=\"zh\">"));
        assert!(themed.contains("--bg-app: rgb(23, 22, 20)"));
        assert!(themed.contains(".thread { gap: 20px; }"));
        let live_rule = themed.find(".thread { gap: 20px; }").unwrap();
        let layout_guard = themed.find(".share-page, .thread, .msg").unwrap();
        assert!(layout_guard > live_rule, "layout guards must win over live CSS");
        assert!(themed.contains("overflow-wrap: anywhere"));
        assert!(themed.contains(".math-display) { max-width: 100%; overflow-x: auto; }"));
    }

    #[test]
    fn share_export_css_tracks_chat_message_rules() {
        let chat = include_str!("../styles/chat.css");
        let share = include_str!("../styles/share-export.css");
        for needle in [
            "background: var(--bg-panel); border: 0; padding: 10px 15px;",
            "border-radius: 18px 18px 6px 18px; box-shadow: inset 0 0 0 1px var(--border);",
            "border-left: 2px solid var(--border-strong); padding-left: 12px;",
            "font-family: var(--font-response);",
            "font-size: calc(var(--ui-font-size, 14px) + 1px); line-height: 1.62;",
        ] {
            assert!(chat.contains(needle), "chat.css missing {needle}");
            assert!(
                share.contains(needle),
                "share-export.css drifted from chat.css: {needle}"
            );
        }
    }

    #[test]
    fn markdown_blocks_keep_hard_breaks_and_task_markers() {
        let blocks = share_markdown_blocks("第一行  \n第二行\n\n- [x] 已完成\n- [ ] 待办\n");
        let texts: Vec<&str> = blocks[0]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["text"].as_str().unwrap())
            .collect();
        // Same-style runs merge; the JS wrapper still honors the embedded \n.
        assert_eq!(texts, vec!["第一行\n第二行"]);
        assert_eq!(blocks[1]["runs"][0]["text"], json!("☑ 已完成"));
        assert_eq!(blocks[2]["runs"][0]["text"], json!("☐ 待办"));
    }

    #[test]
    fn social_pack_skips_duplicate_title_and_normalizes_tags() {
        let variant = ShareSocialVariant {
            title: "主峰".into(),
            body: "主峰在 530 nm，谱图很干净。".into(),
            hashtags: vec!["RNA".into(), "#谱图".into(), "  ".into()],
        };
        assert_eq!(
            share_social_pack_text(&variant),
            "主峰在 530 nm，谱图很干净。\n\n#RNA #谱图"
        );
        assert_eq!(normalize_share_hashtag("  ##RNA  "), "#RNA");
    }

    #[test]
    fn social_pack_keeps_title_when_body_starts_differently() {
        let variant = ShareSocialVariant {
            title: "今天的拟合".into(),
            body: "530 nm 的峰是主峰。".into(),
            hashtags: vec![],
        };
        assert_eq!(
            share_social_pack_text(&variant),
            "今天的拟合\n\n530 nm 的峰是主峰。"
        );
    }

    #[test]
    fn key_cards_pair_short_user_with_latest_assistant() {
        let messages = vec![
            ShareMessage {
                role: ShareRole::User,
                text: "看一下主峰".into(),
                selected: true,
            },
            ShareMessage {
                role: ShareRole::Thinking,
                text: "先比对谱库".into(),
                selected: false,
            },
            ShareMessage {
                role: ShareRole::Assistant,
                text: "## 拟合结果\n主峰在 530 nm。".into(),
                selected: true,
            },
        ];
        let cards = share_key_cards(&messages);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].indexes, vec![0, 2]);
        assert_eq!(cards[0].title, "拟合结果");
    }

    #[test]
    fn key_cards_keep_the_last_three_assistant_turns() {
        let messages: Vec<ShareMessage> = (0..5)
            .map(|index| ShareMessage {
                role: ShareRole::Assistant,
                text: format!("结果 {index}"),
                selected: true,
            })
            .collect();
        let cards = share_key_cards(&messages);
        assert_eq!(
            cards
                .iter()
                .map(|card| card.indexes.clone())
                .collect::<Vec<_>>(),
            vec![vec![2], vec![3], vec![4]]
        );
    }

    #[test]
    fn highlight_indexes_map_from_the_excerpt_onto_the_draft() {
        let messages = vec![
            ShareMessage {
                role: ShareRole::User,
                text: "问".into(),
                selected: true,
            },
            ShareMessage {
                role: ShareRole::Thinking,
                text: "想".into(),
                selected: false,
            },
            ShareMessage {
                role: ShareRole::Assistant,
                text: "答".into(),
                selected: true,
            },
        ];
        let highlight = ShareSocialHighlight {
            title: "Clean peak".into(),
            why: "unambiguous".into(),
            message_indexes: vec![1, 2],
        };
        let cards = share_cards_from_highlights(&messages, &[0, 2], &[highlight]);
        assert_eq!(cards[0].indexes, vec![0, 2]);
        assert_eq!(cards[0].title, "Clean peak");
    }

    #[test]
    fn fallback_caption_uses_the_latest_assistant_reply() {
        let messages = vec![
            ShareMessage {
                role: ShareRole::User,
                text: "问".into(),
                selected: true,
            },
            ShareMessage {
                role: ShareRole::Assistant,
                text: "主峰在 530 nm。".into(),
                selected: true,
            },
        ];
        assert_eq!(share_fallback_caption(&messages, 80), "主峰在 530 nm。");
        assert!(share_fallback_caption(&messages, 6).ends_with('…'));
    }
}
