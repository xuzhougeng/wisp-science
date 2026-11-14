//! Questions the user could ask next, suggested by the conversation's model
//! once a turn has ended on an answer.
//!
//! The switch and its key are the WebView desktop's, so a database means
//! the same thing to both. The suggestions are kept in memory only: they
//! belong to the answer that was just given and are gone when the next turn
//! starts or the app restarts.

use anyhow::{bail, Context, Result};
use wisp_llm::{Message, Provider, Role};
use wisp_store::Store;

pub(crate) const SETTING: &str = "follow_up_questions";
/// How many of the latest turns the model is shown.
pub(crate) const TURNS: usize = 4;
/// The most the suggestion call may write.
pub(crate) const MAX_TOKENS: u64 = 1_024;
const QUESTIONS: usize = 3;
/// A suggestion is a question to click, not a paragraph.
const QUESTION_LIMIT: usize = 240;
/// What is shown of one message, of one tool result, and in all.
const MESSAGE_LIMIT: usize = 2_000;
const TOOL_LIMIT: usize = 600;
const TRANSCRIPT_LIMIT: usize = 24_000;
const WAIT: std::time::Duration = std::time::Duration::from_secs(45);
const ASK: &str = "Suggest exactly three concise, useful questions the user could ask next. \
                   Return only a JSON array of three strings. Do not answer them.";

/// On until someone turns it off.
pub(crate) async fn enabled(store: &Store) -> bool {
    store
        .get_setting(SETTING)
        .await
        .ok()
        .flatten()
        .is_none_or(|value| value == "true")
}

/// Asks `model` what could be asked next about `messages`, the latest turns
/// of a conversation. Nothing is suggested, and the model is not called,
/// when the conversation did not end on an answer.
pub(crate) async fn suggest(model: &dyn Provider, messages: &[Message]) -> Result<Vec<String>> {
    if !ended_on_an_answer(messages) {
        return Ok(vec![]);
    }
    let ask = [Message::system(ASK), Message::user(transcript(messages))];
    let reply = tokio::time::timeout(WAIT, model.complete(&ask, &[]))
        .await
        .context("The model did not answer in time")??;
    parse(&reply.content)
}

/// The turn ended on something said to the user: the model's own words or
/// a completion result. A turn cut off after a tool call, or waiting on a
/// question to the user, has nothing to follow up on.
fn ended_on_an_answer(messages: &[Message]) -> bool {
    messages
        .iter()
        .rev()
        .find(|message| message.role != Role::System)
        .is_some_and(|message| {
            let answer = match message.role {
                Role::Assistant => message.tool_calls.is_empty(),
                Role::Tool => message.tool_name.as_deref() == Some("attempt_completion"),
                _ => false,
            };
            answer && !message.content.as_text().trim().is_empty()
        })
}

/// The conversation as the suggestion call reads it: who said what, with
/// long messages and tool results cut short. When it is still too long the
/// oldest part goes, because the questions follow from where it ended.
fn transcript(messages: &[Message]) -> String {
    let mut blocks = Vec::new();
    for message in messages {
        let text = message.content.as_text();
        let (label, limit) = match message.role {
            Role::System => continue,
            Role::User => ("USER".to_owned(), MESSAGE_LIMIT),
            Role::Assistant => ("ASSISTANT".to_owned(), MESSAGE_LIMIT),
            Role::Tool => match message.tool_name.as_deref() {
                Some("attempt_completion") => ("ASSISTANT".to_owned(), MESSAGE_LIMIT),
                name => (format!("TOOL {}", name.unwrap_or("tool")), TOOL_LIMIT),
            },
        };
        if !text.trim().is_empty() {
            blocks.push(format!("[{label}]\n{}", clipped(text.trim(), limit)));
        }
    }
    let whole = blocks.join("\n\n");
    let extra = whole.chars().count().saturating_sub(TRANSCRIPT_LIMIT);
    if extra == 0 {
        return whole;
    }
    let tail: String = whole.chars().skip(extra).collect();
    format!("[Earlier part left out]\n…{tail}")
}

fn clipped(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

/// Reads the model's reply: the first JSON array in it, as up to three
/// different questions. A reply with no usable question is an error, so a
/// model that answered the conversation instead is not shown as advice.
pub(crate) fn parse(raw: &str) -> Result<Vec<String>> {
    let body = raw
        .find('[')
        .zip(raw.rfind(']'))
        .and_then(|(start, end)| raw.get(start..=end))
        .context("The model did not return a list of questions")?;
    let values: Vec<String> =
        serde_json::from_str(body).context("The model's list of questions could not be read")?;
    let mut questions: Vec<String> = Vec::new();
    for value in values {
        // One line each: a question is shown as a button.
        let question = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if question.is_empty() || question.chars().count() > QUESTION_LIMIT {
            continue;
        }
        if !questions.contains(&question) {
            questions.push(question);
        }
        if questions.len() == QUESTIONS {
            break;
        }
    }
    if questions.is_empty() {
        bail!("The model suggested no usable question");
    }
    Ok(questions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use wisp_llm::{Completion, FunctionCall, StreamSink, ToolCall, ToolSchema};

    #[test]
    fn a_reply_is_read_as_up_to_three_different_questions() {
        let read = |raw: &str| parse(raw).map_err(|error| error.to_string());
        assert_eq!(
            read(r#"["Why?", "How many?", "What next?"]"#).unwrap(),
            ["Why?", "How many?", "What next?"]
        );
        // Models wrap the list in prose or a code fence, repeat themselves
        // and offer more than was asked for.
        assert_eq!(
            read("Here you go:\n```json\n[\" Why? \", \"Why?\", \"\", \"How\\n many?\", \"A\", \"B\"]\n```")
                .unwrap(),
            ["Why?", "How many?", "A"]
        );
        // Fewer than three is still worth showing.
        assert_eq!(read(r#"["Only one?"]"#).unwrap(), ["Only one?"]);
        // An essay is not a question to click.
        let essay = "x".repeat(QUESTION_LIMIT + 1);
        assert_eq!(
            read(&format!(r#"["{essay}", "Short?"]"#)).unwrap(),
            ["Short?"]
        );

        assert!(read("The answer is 42.").unwrap_err().contains("list"));
        assert!(read("] backwards [").unwrap_err().contains("list"));
        assert!(read(r#"[1, 2, 3]"#)
            .unwrap_err()
            .contains("could not be read"));
        assert!(read(r#"["", "  "]"#).unwrap_err().contains("no usable"));
    }

    fn calling(name: &str) -> Message {
        let mut message = Message::assistant("");
        message.tool_calls = vec![ToolCall {
            id: "c1".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
        }];
        message
    }

    #[test]
    fn only_a_turn_that_ended_on_an_answer_is_followed_up() {
        let asked = Message::user("count the reads");
        assert!(ended_on_an_answer(&[
            asked.clone(),
            Message::assistant("There are 100.")
        ]));
        assert!(ended_on_an_answer(&[
            asked.clone(),
            calling("attempt_completion"),
            Message::tool("c1", "attempt_completion", "There are 100."),
        ]));
        // Nothing said, stopped after a tool, or waiting on the user.
        assert!(!ended_on_an_answer(&[]));
        assert!(!ended_on_an_answer(&[asked.clone()]));
        assert!(!ended_on_an_answer(&[
            asked.clone(),
            Message::assistant("  ")
        ]));
        assert!(!ended_on_an_answer(&[
            asked.clone(),
            calling("shell"),
            Message::tool("c1", "shell", "100 reads.fq"),
        ]));
        assert!(!ended_on_an_answer(&[asked, calling("ask_user")]));
    }

    #[test]
    fn the_model_reads_a_short_transcript_that_keeps_the_end() {
        let messages = [
            Message::system("You are Wisp."),
            Message::user("count the reads"),
            calling("shell"),
            Message::tool("c1", "shell", "y".repeat(TOOL_LIMIT + 50)),
            Message::assistant("There are 100."),
        ];
        let text = transcript(&messages);
        assert!(!text.contains("You are Wisp."));
        assert!(text.starts_with("[USER]\ncount the reads\n\n[TOOL shell]\n"));
        assert!(text.contains(&format!("{}…\n\n", "y".repeat(TOOL_LIMIT))));
        assert!(text.ends_with("[ASSISTANT]\nThere are 100."));

        // Too long in all: the oldest part goes and the end is whole.
        let long: Vec<_> = (0..20)
            .map(|turn| Message::user(format!("{turn} {}", "z".repeat(MESSAGE_LIMIT))))
            .chain([Message::assistant("The last answer.")])
            .collect();
        let text = transcript(&long);
        assert!(text.starts_with("[Earlier part left out]\n…"));
        assert!(text.ends_with("[ASSISTANT]\nThe last answer."));
        assert!(text.chars().count() < TRANSCRIPT_LIMIT + 40);
    }

    struct Scripted {
        reply: &'static str,
        asked: Mutex<Vec<Vec<Message>>>,
    }
    #[async_trait]
    impl Provider for Scripted {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            assert!(tools.is_empty(), "a suggestion call has no tools");
            self.asked.lock().unwrap().push(messages.to_vec());
            Ok(Completion {
                content: self.reply.into(),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }

    #[tokio::test]
    async fn the_model_is_asked_only_when_there_is_an_answer_to_follow() {
        let model = Scripted {
            reply: r#"["Why?", "How?", "What next?"]"#,
            asked: Mutex::default(),
        };
        // Stopped after a tool: not asked, nothing suggested.
        let cut_off = [Message::user("count"), calling("shell")];
        assert!(suggest(&model, &cut_off).await.unwrap().is_empty());
        assert!(model.asked.lock().unwrap().is_empty());

        let answered = [Message::user("count"), Message::assistant("100.")];
        assert_eq!(
            suggest(&model, &answered).await.unwrap(),
            ["Why?", "How?", "What next?"]
        );
        let asked = model.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0][0].role, Role::System);
        assert_eq!(
            asked[0][1].content.as_text(),
            "[USER]\ncount\n\n[ASSISTANT]\n100."
        );
    }
}
