//! Questions asked aside: what was asked about a conversation and what was
//! answered, kept beside it and never part of it. One thread per
//! conversation, in memory; it holds no graphics state.
use wisp_server::desktop::AsideExchange;

/// How many exchanges a thread keeps; older ones fall off the top.
pub const KEPT: usize = 50;

/// One question and how it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub question: String,
    /// The answer, or why there is none.
    pub answer: Result<String, String>,
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
        self.entries.push(Entry { question, answer });
        if self.entries.len() > KEPT {
            let extra = self.entries.len() - KEPT;
            self.entries.drain(..extra);
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
        "Ask about this conversation. The answer comes from what has been said in it, uses \
         no tools, and nothing is added to the conversation."
    } else {
        "Open a conversation to ask about it."
    }
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
}
