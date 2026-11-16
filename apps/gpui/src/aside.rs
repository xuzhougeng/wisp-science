//! Questions asked aside: what was asked about a conversation and what was
//! answered, kept beside it and never part of it. One thread per
//! conversation, in memory; it holds no graphics state.
use crate::i18n::tr;
use wisp_server::desktop::{AsideAnswer, AsideExchange, AsideSource};

/// How many exchanges a thread keeps; older ones fall off the top.
pub const KEPT: usize = 50;

/// One question and how it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub question: String,
    /// The answer, or why there is none.
    pub answer: Result<String, String>,
    /// The passages from earlier in the conversation it was given.
    pub sources: Vec<AsideSource>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Thread {
    pub entries: Vec<Entry>,
    /// The question that is with the model now.
    pub asking: Option<String>,
}

impl Thread {
    /// Starts a question. Returns the exchanges that go along with it, or
    /// nothing when there is no question or one is already being answered.
    pub fn ask(&mut self, question: &str) -> Option<Vec<AsideExchange>> {
        let question = question.trim();
        if question.is_empty() || self.asking.is_some() {
            return None;
        }
        self.asking = Some(question.to_owned());
        // A question that failed taught the model nothing.
        Some(
            self.entries
                .iter()
                .filter_map(|entry| {
                    Some(AsideExchange {
                        question: entry.question.clone(),
                        answer: entry.answer.clone().ok()?,
                    })
                })
                .collect(),
        )
    }

    /// Files the answer to the question that was being asked.
    pub fn answered(&mut self, answer: Result<String, String>) {
        let Some(question) = self.asking.take() else {
            // The thread was cleared while the model was answering.
            return;
        };
        self.entries.push(Entry {
            question,
            answer,
            sources: Vec::new(),
        });
        if self.entries.len() > KEPT {
            let extra = self.entries.len() - KEPT;
            self.entries.drain(..extra);
        }
    }

    /// Files an answer with the passages it was given.
    pub fn answered_with(&mut self, answer: Result<AsideAnswer, String>) {
        let sources = answer
            .as_ref()
            .map(|answer| answer.sources.clone())
            .unwrap_or_default();
        self.answered(answer.map(|answer| answer.text));
        if let Some(last) = self.entries.last_mut() {
            last.sources = sources;
        }
    }

    /// Forgets the thread. An answer still on its way is dropped.
    pub fn clear(&mut self) {
        *self = Thread::default();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.asking.is_none()
    }
}

/// The line under the tab's title.
pub fn hint(has_conversation: bool) -> &'static str {
    if has_conversation {
        tr(
            "Ask about this conversation. The answer comes from what has been said in it, uses \
         no tools, and nothing is added to the conversation.",
        )
    } else {
        tr("Open a conversation to ask about it.")
    }
}

/// The line above a passage: its label, who said it and the words of the
/// question it matched.
pub fn source_line(source: &AsideSource) -> String {
    let who = match source.role.as_str() {
        "user" => tr("You"),
        // The product's name is the same in every language.
        "assistant" => "Wisp",
        _ => tr("A tool"),
    };
    let matched: Vec<String> = source
        .matched
        .iter()
        .take(3)
        .map(|word| format!("“{word}”"))
        .collect();
    crate::i18n::fill(
        tr("[{}] {} · matched {}"),
        &[&source.label, &who, &matched.join(", ")],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_asks_one_question_at_a_time_and_carries_what_was_answered() {
        let mut thread = Thread::default();
        assert!(thread.is_empty());
        assert_eq!(thread.ask("   "), None);
        assert_eq!(thread.ask("  why 100?  "), Some(vec![]));
        assert_eq!(thread.asking.as_deref(), Some("why 100?"));
        assert!(!thread.is_empty());
        // One at a time.
        assert_eq!(thread.ask("and then?"), None);
        thread.answered(Ok("Because of the header.".into()));
        assert_eq!(thread.asking, None);

        assert!(thread.ask("which file?").is_some());
        thread.answered(Err("The model did not answer".into()));
        // The next question carries the answered exchange, not the failure.
        assert_eq!(
            thread.ask("and then?"),
            Some(vec![AsideExchange {
                question: "why 100?".into(),
                answer: "Because of the header.".into(),
            }])
        );
        // Cleared while the model answers: the late answer is not filed.
        thread.clear();
        thread.answered(Ok("late".into()));
        assert!(thread.is_empty());
    }

    #[test]
    fn a_long_thread_drops_its_oldest_exchanges() {
        let mut thread = Thread::default();
        for index in 0..KEPT + 2 {
            thread.ask(&format!("q{index}")).unwrap();
            thread.answered(Ok(format!("a{index}")));
        }
        assert_eq!(thread.entries.len(), KEPT);
        assert_eq!(thread.entries[0].question, "q2");
        assert_eq!(thread.entries[KEPT - 1].question, format!("q{}", KEPT + 1));
    }

    #[test]
    fn the_hint_says_what_asking_aside_does() {
        assert!(hint(true).contains("nothing is added"));
        assert!(hint(false).starts_with("Open a conversation"));
    }

    #[test]
    fn an_answer_keeps_the_passages_it_was_given() {
        let mut thread = Thread::default();
        thread.ask("why?").unwrap();
        let source = AsideSource {
            label: "E1".into(),
            role: "assistant".into(),
            excerpt: "Batch two is apart.".into(),
            matched: vec!["batch".into(), "two".into(), "apart".into(), "pca".into()],
        };
        thread.answered_with(Ok(AsideAnswer {
            text: "Because of batch two [E1].".into(),
            sources: vec![source.clone()],
        }));
        let entry = thread.entries.last().unwrap();
        assert_eq!(entry.answer.as_deref(), Ok("Because of batch two [E1]."));
        assert_eq!(entry.sources, [source.clone()]);
        assert_eq!(
            source_line(&source),
            "[E1] Wisp · matched “batch”, “two”, “apart”"
        );
        thread.ask("again?").unwrap();
        thread.answered_with(Err("no model".into()));
        assert!(thread.entries.last().unwrap().sources.is_empty());
    }
}
