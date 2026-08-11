//! Shared per-session agent control: cancellation, mid-turn steering, and the
//! parked follow-up queue. [`AgentControl`] is the single ownership point the
//! agent loop and the host both clone, so the host can steer or enqueue
//! follow-ups even while it holds the agent's run lock.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::agent::GuidanceQueue;

/// Identity for items parked in a [`FollowUpQueue`]. The host's queued-item
/// type implements this so Core can match items by id without naming host
/// types (attachments, composer references, ...).
pub trait FollowUpItem {
    fn id(&self) -> u64;
}

/// Host-agnostic FIFO queue of parked follow-up turns (#433) plus the driver
/// claim/release protocol. Clone shares the same underlying queue.
///
/// No lost wakeup: `enqueue` pushes and claims the driver slot in one lock
/// acquisition, and `driver_pop` releases the slot only when it observes an
/// empty queue under that same lock — so an enqueue can never strand behind a
/// driver that is exiting on an empty queue.
///
/// ponytail: the queue is in-memory only — items are lost on app restart,
/// same as the optimistic bubbles, which are never persisted either. This is
/// intentional for now.
pub struct FollowUpQueue<T> {
    state: Arc<Mutex<FollowUpState<T>>>,
}

struct FollowUpState<T> {
    items: VecDeque<T>,
    /// True while a driver task owns draining `items`. Flipped only under
    /// this lock, which is what makes the enqueue-vs-exit race safe.
    draining: bool,
}

impl<T> Clone for FollowUpQueue<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<T> Default for FollowUpQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FollowUpQueue<T> {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(FollowUpState {
                items: VecDeque::new(),
                draining: false,
            })),
        }
    }

    /// Park a follow-up at the back. Returns true when the caller must spawn
    /// a driver: the push and the driver-slot claim happen atomically, and
    /// the slot is only released by `driver_pop` on an empty queue.
    pub fn enqueue(&self, item: T) -> bool {
        let mut state = self.state.lock().unwrap();
        state.items.push_back(item);
        Self::claim_driver_locked(&mut state)
    }

    /// Claim the driver slot without pushing. Returns true when no driver was
    /// active, i.e. the caller now owns draining and must spawn one. Used when
    /// an item re-enters the queue outside `enqueue` (cut-in reclaim).
    pub fn claim_driver(&self) -> bool {
        Self::claim_driver_locked(&mut self.state.lock().unwrap())
    }

    fn claim_driver_locked(state: &mut FollowUpState<T>) -> bool {
        !std::mem::replace(&mut state.draining, true)
    }

    /// Driver-side pop: takes the next item FIFO, or — when the queue is
    /// empty — releases the driver slot under the same lock and returns None
    /// so the driver exits. An enqueue after that point re-claims the slot.
    pub fn driver_pop(&self) -> Option<T> {
        let mut state = self.state.lock().unwrap();
        match state.items.pop_front() {
            Some(item) => Some(item),
            None => {
                state.draining = false;
                None
            }
        }
    }

    /// Plain front pop without driver semantics.
    pub fn pop_front(&self) -> Option<T> {
        self.state.lock().unwrap().items.pop_front()
    }

    /// Put an item back at the front (reclaim of an unconsumed cut-in).
    pub fn insert_front(&self, item: T) {
        self.state.lock().unwrap().items.push_front(item);
    }

    pub fn is_empty(&self) -> bool {
        self.state.lock().unwrap().items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().items.len()
    }

    /// True while a driver task owns draining this queue.
    pub fn driver_active(&self) -> bool {
        self.state.lock().unwrap().draining
    }

    pub fn snapshot(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.state.lock().unwrap().items.iter().cloned().collect()
    }
}

impl<T: FollowUpItem> FollowUpQueue<T> {
    /// Replace part of a parked item in place (e.g. its text). Returns false
    /// when no item with `id` is parked.
    pub fn edit(&self, id: u64, edit: impl FnOnce(&mut T)) -> bool {
        let mut state = self.state.lock().unwrap();
        match state.items.iter_mut().find(|item| item.id() == id) {
            Some(item) => {
                edit(item);
                true
            }
            None => false,
        }
    }

    /// Drop a parked item. Returns false when no item with `id` was parked.
    pub fn cancel(&self, id: u64) -> bool {
        let mut state = self.state.lock().unwrap();
        let before = state.items.len();
        state.items.retain(|item| item.id() != id);
        state.items.len() != before
    }

    /// Pull an item out of the queue (cut-in into the running turn).
    pub fn take(&self, id: u64) -> Option<T> {
        let mut state = self.state.lock().unwrap();
        state
            .items
            .iter()
            .position(|item| item.id() == id)
            .and_then(|index| state.items.remove(index))
    }

    /// Swap with the previous neighbour; a no-op when already first.
    pub fn move_up(&self, id: u64) -> bool {
        self.swap_toward(id, true)
    }

    /// Swap with the next neighbour; a no-op when already last.
    pub fn move_down(&self, id: u64) -> bool {
        self.swap_toward(id, false)
    }

    fn swap_toward(&self, id: u64, up: bool) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(index) = state.items.iter().position(|item| item.id() == id) else {
            return false;
        };
        let target = if up {
            index.checked_sub(1)
        } else {
            (index + 1 < state.items.len()).then_some(index + 1)
        };
        match target {
            Some(other) => {
                state.items.swap(index, other);
                true
            }
            None => false,
        }
    }
}

/// Shared control state for one agent session. Clone freely — every clone
/// points at the same cancel flag, steering queue, and follow-up queue, so the
/// host can steer or enqueue while the agent loop is running.
pub struct AgentControl<T> {
    inner: Arc<AgentControlInner<T>>,
}

struct AgentControlInner<T> {
    cancel: Arc<AtomicBool>,
    // TODO(agent-message): the steering payload stays `(u64, String)` until the
    // structured AgentMessage migration; this queue is where it slots in.
    steering: GuidanceQueue,
    steering_seq: AtomicU64,
    follow_ups: FollowUpQueue<T>,
}

impl<T> Clone for AgentControl<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Default for AgentControl<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> AgentControl<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AgentControlInner {
                cancel: Arc::new(AtomicBool::new(false)),
                steering: GuidanceQueue::default(),
                steering_seq: AtomicU64::new(0),
                follow_ups: FollowUpQueue::new(),
            }),
        }
    }

    /// Borrow the cancel flag (e.g. `Some(control.cancel_flag())` for the
    /// agent loop).
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.inner.cancel
    }

    /// Share the cancel flag with a component that outlives this handle.
    pub fn cancel_arc(&self) -> Arc<AtomicBool> {
        self.inner.cancel.clone()
    }

    pub fn request_cancel(&self) {
        self.inner.cancel.store(true, Ordering::Relaxed);
    }

    pub fn reset_cancel(&self) {
        self.inner.cancel.store(false, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancel.load(Ordering::SeqCst)
    }

    /// The queue the agent loop drains at each iteration boundary; pass it as
    /// `Some(control.steering_queue())` to the agent loop.
    pub fn steering_queue(&self) -> &GuidanceQueue {
        &self.inner.steering
    }

    /// Park a steering message for the running loop. Returns its id so the
    /// sender can later tell whether the loop consumed it.
    pub fn push_steering(&self, text: impl Into<String>) -> u64 {
        let id = self.inner.steering_seq.fetch_add(1, Ordering::Relaxed);
        self.inner.steering.lock().unwrap().push((id, text.into()));
        id
    }

    /// Remove a steering message the loop has not consumed yet. Returns true
    /// when it was still parked (caller must run it as a normal turn); false
    /// means the loop already folded it into the running turn.
    pub fn reclaim_steering(&self, id: u64) -> bool {
        let mut steering = self.inner.steering.lock().unwrap();
        let before = steering.len();
        steering.retain(|(steering_id, _)| *steering_id != id);
        steering.len() != before
    }

    pub fn follow_ups(&self) -> &FollowUpQueue<T> {
        &self.inner.follow_ups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Item {
        id: u64,
        text: String,
    }

    impl FollowUpItem for Item {
        fn id(&self) -> u64 {
            self.id
        }
    }

    fn item(id: u64) -> Item {
        Item {
            id,
            text: format!("m{id}"),
        }
    }

    fn ids(queue: &FollowUpQueue<Item>) -> Vec<u64> {
        queue.snapshot().iter().map(|item| item.id).collect()
    }

    #[test]
    fn enqueue_claims_one_driver_and_drains_fifo() {
        let queue = FollowUpQueue::new();
        assert!(queue.enqueue(item(1)), "first enqueue claims the driver");
        assert!(
            !queue.enqueue(item(2)),
            "a driver is already running for later enqueues"
        );
        assert!(!queue.enqueue(item(3)));

        assert_eq!(queue.driver_pop().map(|item| item.id), Some(1));
        assert_eq!(queue.driver_pop().map(|item| item.id), Some(2));
        assert_eq!(queue.driver_pop().map(|item| item.id), Some(3));
        assert!(
            queue.driver_pop().is_none(),
            "empty queue releases the slot"
        );
        assert!(!queue.driver_active());
    }

    #[test]
    fn driver_slot_is_reclaimable_after_the_queue_empties() {
        let queue = FollowUpQueue::new();
        assert!(queue.enqueue(item(1)));
        assert_eq!(queue.driver_pop().map(|item| item.id), Some(1));
        assert!(queue.driver_pop().is_none());

        // A post-drain enqueue re-claims the slot rather than stranding.
        assert!(queue.enqueue(item(2)));
        assert_eq!(queue.driver_pop().map(|item| item.id), Some(2));
    }

    #[test]
    fn edit_by_id_replaces_in_place() {
        let queue = FollowUpQueue::new();
        queue.enqueue(item(1));
        queue.enqueue(item(2));
        assert!(queue.edit(2, |item| item.text = "edited".into()));
        assert!(!queue.edit(9, |item| item.text = "nope".into()));
        let snapshot = queue.snapshot();
        assert_eq!(snapshot[0].text, "m1");
        assert_eq!(snapshot[1].text, "edited");
    }

    #[test]
    fn cancel_by_id_removes_only_that_item() {
        let queue = FollowUpQueue::new();
        queue.enqueue(item(1));
        queue.enqueue(item(2));
        queue.enqueue(item(3));
        assert!(queue.cancel(2));
        assert!(!queue.cancel(2), "already gone");
        assert_eq!(ids(&queue), [1, 3]);
    }

    #[test]
    fn take_removes_and_insert_front_reclaims() {
        let queue = FollowUpQueue::new();
        queue.enqueue(item(1));
        queue.enqueue(item(2));
        let taken = queue.take(1).expect("item 1 is parked");
        assert_eq!(taken.text, "m1");
        assert_eq!(ids(&queue), [2]);
        queue.insert_front(taken);
        assert_eq!(ids(&queue), [1, 2]);
        assert!(queue.take(9).is_none());
    }

    #[test]
    fn move_swaps_with_neighbour_and_clamps_at_the_ends() {
        let queue = FollowUpQueue::new();
        queue.enqueue(item(1));
        queue.enqueue(item(2));
        queue.enqueue(item(3)); // A, B, C
        assert!(queue.move_up(3)); // C up → A, C, B
        assert_eq!(ids(&queue), [1, 3, 2]);
        assert!(queue.move_down(1)); // A down → C, A, B
        assert_eq!(ids(&queue), [3, 1, 2]);
        assert!(!queue.move_up(3), "already first — clamped");
        assert_eq!(ids(&queue), [3, 1, 2]);
        assert!(!queue.move_down(2), "already last — clamped");
        assert_eq!(ids(&queue), [3, 1, 2]);
        assert!(!queue.move_up(9), "unknown id");
    }

    /// The no-lost-wakeup invariant under real concurrency: enqueues from many
    /// threads race a driver that exits as soon as it sees an empty queue.
    /// Every enqueue that claims the slot spawns a fresh driver; at the end
    /// every item must have been consumed exactly once.
    #[test]
    fn concurrent_enqueue_never_strands_an_item_without_a_driver() {
        fn spawn_driver(
            queue: &FollowUpQueue<Item>,
            consumed: &Arc<Mutex<Vec<u64>>>,
            drivers: &Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
        ) {
            let queue = queue.clone();
            let consumed = consumed.clone();
            drivers.lock().unwrap().push(std::thread::spawn(move || {
                while let Some(item) = queue.driver_pop() {
                    consumed.lock().unwrap().push(item.id);
                }
            }));
        }

        let queue = FollowUpQueue::new();
        let consumed = Arc::new(Mutex::new(Vec::new()));
        let drivers = Arc::new(Mutex::new(Vec::new()));
        const PRODUCERS: u64 = 4;
        const PER_PRODUCER: u64 = 50;

        let producers: Vec<_> = (0..PRODUCERS)
            .map(|producer| {
                let queue = queue.clone();
                let consumed = consumed.clone();
                let drivers = drivers.clone();
                std::thread::spawn(move || {
                    for i in 0..PER_PRODUCER {
                        if queue.enqueue(item(producer * PER_PRODUCER + i)) {
                            spawn_driver(&queue, &consumed, &drivers);
                        }
                    }
                })
            })
            .collect();
        for producer in producers {
            producer.join().unwrap();
        }
        // No more enqueues past this point, so the driver set is final and
        // each one exits once it drains the queue.
        for driver in std::mem::take(&mut *drivers.lock().unwrap()) {
            driver.join().unwrap();
        }

        let mut consumed = std::mem::take(&mut *consumed.lock().unwrap());
        consumed.sort_unstable();
        assert_eq!(
            consumed,
            (0..PRODUCERS * PER_PRODUCER).collect::<Vec<_>>(),
            "every enqueued item was consumed exactly once"
        );
        assert!(queue.is_empty());
        assert!(!queue.driver_active());
    }

    #[test]
    fn steering_ids_are_unique_and_reclaim_reports_consumption() {
        let control = AgentControl::<Item>::new();
        let first = control.push_steering("one");
        let second = control.push_steering("two");
        assert_ne!(first, second);

        assert!(control.reclaim_steering(first), "still parked — reclaimed");
        assert!(
            !control.reclaim_steering(first),
            "already removed — treated as consumed"
        );
        // Consumed by the loop draining the queue.
        control.steering_queue().lock().unwrap().clear();
        assert!(!control.reclaim_steering(second));
    }

    #[test]
    fn clones_share_one_control_state() {
        let control = AgentControl::<Item>::new();
        let clone = control.clone();
        control.push_steering("shared");
        clone.follow_ups().enqueue(item(1));
        clone.request_cancel();

        assert_eq!(control.steering_queue().lock().unwrap().len(), 1);
        assert_eq!(control.follow_ups().len(), 1);
        assert!(control.is_cancelled());
        control.reset_cancel();
        assert!(!clone.is_cancelled());
        assert!(std::ptr::eq(control.cancel_flag(), clone.cancel_flag()));
    }
}
