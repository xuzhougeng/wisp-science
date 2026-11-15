//! Messages written while a turn runs. They wait here, in the order they
//! were written, and go one at a time as each turn ends. The queue lives in
//! the window: it is not saved, and it only moves while its conversation is
//! the one that is open.
use crate::i18n::tr;
use std::collections::{HashMap, VecDeque};

use crate::compose::Pending;

/// How many messages may wait in one conversation.
pub const LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
    pub id: u64,
    pub text: String,
    /// The files and references that were attached when it was written.
    pub carried: Pending,
}

#[derive(Default)]
pub struct Queue {
    waiting: HashMap<String, VecDeque<Queued>>,
    next_id: u64,
}

impl Queue {
    /// Adds a message to the end of `session`'s queue. Text is trimmed; a
    /// message needs text or something attached, and the queue has a limit.
    pub fn push(&mut self, session: &str, text: &str, carried: Pending) -> Result<u64, String> {
        let text = text.trim();
        if text.is_empty() && carried.attachments().is_empty() {
            return Err(tr("Write a message first").into());
        }
        let waiting = self.waiting.entry(session.to_owned()).or_default();
        if waiting.len() >= LIMIT {
            return Err(crate::i18n::fill(
                tr("{} messages are already waiting; remove one or wait for the turn to end"),
                &[&LIMIT],
            ));
        }
        self.next_id += 1;
        waiting.push_back(Queued {
            id: self.next_id,
            text: text.to_owned(),
            carried,
        });
        Ok(self.next_id)
    }

    /// What is waiting in `session`, first to go first.
    pub fn list(&self, session: &str) -> Vec<&Queued> {
        self.waiting
            .get(session)
            .map(|waiting| waiting.iter().collect())
            .unwrap_or_default()
    }

    /// Takes one message out, wherever it stands, and hands it back.
    pub fn remove(&mut self, session: &str, id: u64) -> Option<Queued> {
        let waiting = self.waiting.get_mut(session)?;
        let index = waiting.iter().position(|queued| queued.id == id)?;
        let removed = waiting.remove(index);
        if waiting.is_empty() {
            self.waiting.remove(session);
        }
        removed
    }

    /// Moves a message one place earlier. Returns whether it moved: the
    /// first one has nowhere to go.
    pub fn move_up(&mut self, session: &str, id: u64) -> bool {
        let Some(waiting) = self.waiting.get_mut(session) else {
            return false;
        };
        match waiting.iter().position(|queued| queued.id == id) {
            Some(index) if index > 0 => {
                waiting.swap(index - 1, index);
                true
            }
            _ => false,
        }
    }

    /// Takes the message that goes next.
    pub fn next(&mut self, session: &str) -> Option<Queued> {
        let first = self.waiting.get(session)?.front()?.id;
        self.remove(session, first)
    }

    /// Puts a message back at the front: it was taken to be sent and the
    /// host did not accept it.
    pub fn put_back(&mut self, session: &str, queued: Queued) {
        self.waiting
            .entry(session.to_owned())
            .or_default()
            .push_front(queued);
    }

    /// Drops what waits in a conversation that no longer exists.
    pub fn forget(&mut self, session: &str) {
        self.waiting.remove(session);
    }
}

/// Why the queue is not moving although no turn runs, in words for the
/// user. `failed` is the last turn ending in an error; `asking` is a
/// question or a plan that waits for the user's own answer.
pub fn hold(failed: bool, asking: bool) -> Option<&'static str> {
    if asking {
        // A queued message must not be taken for the answer.
        Some(tr("Waiting for your answer above"))
    } else if failed {
        Some(tr("Held because the last turn failed"))
    } else {
        None
    }
}

/// One line for a queued message in the strip above the composer.
pub fn label(queued: &Queued) -> String {
    let first = queued.text.lines().next().unwrap_or_default().trim();
    let attached = queued.carried.attachments().len();
    match (first.is_empty(), attached) {
        (true, n) => crate::files::count(n),
        (false, 0) => first.to_owned(),
        (false, n) => format!("{first} · {}", crate::files::count(n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_message_can_be_moved_ahead_of_the_one_before_it() {
        let mut queue = Queue::default();
        let ids: Vec<u64> = ["one", "two", "three"]
            .iter()
            .map(|text| queue.push("a", text, Pending::default()).unwrap())
            .collect();
        let order = |queue: &Queue| {
            queue
                .list("a")
                .iter()
                .map(|queued| queued.text.clone())
                .collect::<Vec<_>>()
        };
        assert!(queue.move_up("a", ids[2]));
        assert_eq!(order(&queue), ["one", "three", "two"]);
        assert!(queue.move_up("a", ids[2]));
        assert_eq!(order(&queue), ["three", "one", "two"]);
        // The first has nowhere to go; a stranger and another queue change nothing.
        assert!(!queue.move_up("a", ids[2]));
        assert!(!queue.move_up("a", 999));
        assert!(!queue.move_up("b", ids[0]));
        assert_eq!(order(&queue), ["three", "one", "two"]);
        assert_eq!(queue.next("a").unwrap().text, "three");
    }

    #[test]
    fn messages_wait_in_order_per_conversation_and_can_be_taken_out() {
        let mut queue = Queue::default();
        assert!(queue.push("a", "  ", Pending::default()).is_err());
        let first = queue.push("a", " first \n", Pending::default()).unwrap();
        let second = queue.push("a", "second", Pending::default()).unwrap();
        let other = queue.push("b", "elsewhere", Pending::default()).unwrap();
        assert_eq!(
            queue
                .list("a")
                .iter()
                .map(|q| q.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(queue.list("missing").is_empty());

        // One is taken back to be edited; the rest keep their order.
        let third = queue.push("a", "third", Pending::default()).unwrap();
        assert_eq!(queue.remove("a", second).unwrap().text, "second");
        assert_eq!(queue.remove("a", second), None);
        assert_eq!(queue.remove("b", first), None);
        assert_eq!(queue.next("a").unwrap().id, first);
        // A message the host refused goes back to the front, not the end.
        let taken = queue.next("a").unwrap();
        assert_eq!(taken.id, third);
        queue.push("a", "fourth", Pending::default()).unwrap();
        queue.put_back("a", taken);
        assert_eq!(queue.next("a").unwrap().text, "third");
        assert_eq!(queue.next("a").unwrap().text, "fourth");
        assert_eq!(queue.next("a"), None);

        // Another conversation's queue was never touched, until it goes.
        assert_eq!(queue.list("b")[0].id, other);
        queue.forget("b");
        assert!(queue.list("b").is_empty());
    }

    #[test]
    fn a_queue_is_bounded_and_holds_for_answers_and_failures() {
        let mut queue = Queue::default();
        for index in 0..LIMIT {
            queue
                .push("a", &format!("m{index}"), Pending::default())
                .unwrap();
        }
        assert!(queue.push("a", "one too many", Pending::default()).is_err());
        assert_eq!(queue.list("a").len(), LIMIT);
        // Room in one conversation is not room in another.
        assert!(queue.push("b", "fine", Pending::default()).is_ok());

        assert_eq!(hold(false, false), None);
        assert!(hold(true, false).unwrap().contains("failed"));
        // The question matters more: answering it is what unblocks.
        assert!(hold(true, true).unwrap().contains("answer"));

        let queued = |text: &str| Queued {
            id: 1,
            text: text.into(),
            carried: Pending::default(),
        };
        assert_eq!(label(&queued("first line\nsecond")), "first line");
    }
}
