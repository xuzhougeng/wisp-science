//! CardKit cards for Feishu replies that are shown while they are made, a
//! port of the WebView's `src-tauri/src/channels/feishu_card.rs`. The card
//! is drawn from the turn's progress as the host polls it, where the
//! WebView folded live events into the same picture.
//!
//! Only tool names and how they went are shown. Reasoning, command output
//! and tool results can hold secrets and never go into another app's card.

use serde_json::json;

use super::TurnProgress;

pub const PROGRESS_ELEMENT_ID: &str = "md";
const MAX_ASSISTANT_CHARS: usize = 5_500;

pub fn build_streaming_card(initial_markdown: &str) -> String {
    json!({
        "schema": "2.0",
        "config": { "streaming_mode": true },
        "body": {
            "elements": [{
                "tag": "markdown",
                "element_id": PROGRESS_ELEMENT_ID,
                "content": initial_markdown,
            }]
        }
    })
    .to_string()
}

/// The card's text for where the turn has got.
pub fn render(progress: &TurnProgress) -> String {
    let mut sections = vec![if progress.active {
        "**Wisp Science 正在处理**".to_owned()
    } else {
        "**已收到消息，正在准备…**".to_owned()
    }];
    let assistant = tail_chars(progress.assistant.trim(), MAX_ASSISTANT_CHARS);
    if !assistant.is_empty() {
        sections.push(assistant);
    }
    if !progress.tools.is_empty() {
        let rows = progress
            .tools
            .iter()
            .map(|(name, outcome)| {
                let name = safe_tool_name(name);
                match outcome {
                    None => format!("⏳ `{name}` 正在执行"),
                    Some((ok, duration_ms)) => format!(
                        "{} `{name}` 已{} · {}",
                        if *ok { "✅" } else { "⚠️" },
                        if *ok { "完成" } else { "失败" },
                        format_duration(*duration_ms)
                    ),
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(rows);
    }
    sections.push("_完成后，最终答案会固定在这张卡片中。_".to_owned());
    sections.join("\n\n")
}

fn safe_tool_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '`')
        .take(80)
        .collect();
    if cleaned.trim().is_empty() {
        "tool".into()
    } else {
        cleaned
    }
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let tail: String = text.chars().skip(count - max_chars).collect();
    format!("…{tail}")
}

fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1_000 {
        format!("{duration_ms} ms")
    } else {
        format!("{:.1} s", duration_ms as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_card_is_valid_and_addresses_markdown_element() {
        let card = build_streaming_card("处理中…");
        let value: serde_json::Value = serde_json::from_str(&card).unwrap();
        assert_eq!(value["schema"], "2.0");
        assert_eq!(value["config"]["streaming_mode"], true);
        assert_eq!(value["body"]["elements"][0]["element_id"], "md");
        assert_eq!(value["body"]["elements"][0]["content"], "处理中…");
    }

    #[test]
    fn progress_shows_the_answer_so_far_and_how_tools_went() {
        assert!(render(&TurnProgress::default()).contains("正在准备"));
        let rendered = render(&TurnProgress {
            active: true,
            assistant: "partial answer".into(),
            tools: vec![
                ("shell".into(), Some((true, 1_250))),
                ("wri`te\n".into(), Some((false, 40))),
                ("read".into(), None),
            ],
        });
        assert!(rendered.contains("正在处理"));
        assert!(rendered.contains("partial answer"));
        assert!(rendered.contains("✅ `shell` 已完成 · 1.2 s"));
        assert!(rendered.contains("⚠️ `write` 已失败 · 40 ms"));
        assert!(rendered.contains("⏳ `read` 正在执行"));
    }

    #[test]
    fn a_long_answer_keeps_its_end() {
        let rendered = render(&TurnProgress {
            active: true,
            assistant: format!("{}END", "x".repeat(MAX_ASSISTANT_CHARS + 10)),
            tools: Vec::new(),
        });
        assert!(rendered.contains("…"));
        assert!(rendered.contains("END"));
    }
}
