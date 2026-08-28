//! Bounded in-process event subscriptions.

use std::collections::{BTreeMap, VecDeque};

use omacell_core::event::Event;

/// Handle returned by [`EventBus::subscribe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriberId(u64);

impl SubscriberId {
    /// Numeric id.
    #[must_use]
    pub const fn index(self) -> u64 {
        self.0
    }
}

struct Sub {
    cap: usize,
    queue: VecDeque<Event>,
    dropped: u64,
}

/// Deterministic event fan-out. Emit never waits on a subscriber.
#[derive(Clone, Debug)]
pub struct EventBus {
    next: u64,
    subs: BTreeMap<u64, Sub>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Sub {
    fn clone(&self) -> Self {
        Self {
            cap: self.cap,
            queue: self.queue.clone(),
            dropped: self.dropped,
        }
    }
}

impl std::fmt::Debug for Sub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sub")
            .field("cap", &self.cap)
            .field("queued", &self.queue.len())
            .field("dropped", &self.dropped)
            .finish()
    }
}

impl EventBus {
    /// Empty bus.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 1,
            subs: BTreeMap::new(),
        }
    }

    /// Subscribe with a bounded queue. `cap` is at least 1.
    pub fn subscribe(&mut self, cap: usize) -> SubscriberId {
        let id = self.next;
        self.next += 1;
        self.subs.insert(
            id,
            Sub {
                cap: cap.max(1),
                queue: VecDeque::new(),
                dropped: 0,
            },
        );
        SubscriberId(id)
    }

    /// Drop a subscriber.
    pub fn unsubscribe(&mut self, id: SubscriberId) {
        self.subs.remove(&id.0);
    }

    /// Push `event` to every subscriber without blocking.
    ///
    /// A full queue drops the oldest event and increments the overflow counter.
    pub fn emit(&mut self, event: Event) {
        for sub in self.subs.values_mut() {
            if sub.queue.len() >= sub.cap {
                let _ = sub.queue.pop_front();
                sub.dropped = sub.dropped.saturating_add(1);
            }
            sub.queue.push_back(event.clone());
        }
    }

    /// Drain queued events for `id`.
    pub fn drain(&mut self, id: SubscriberId) -> Vec<Event> {
        self.subs
            .get_mut(&id.0)
            .map(|sub| sub.queue.drain(..).collect())
            .unwrap_or_default()
    }

    /// Events dropped because the subscriber was stalled.
    #[must_use]
    pub fn dropped(&self, id: SubscriberId) -> u64 {
        self.subs.get(&id.0).map(|sub| sub.dropped).unwrap_or(0)
    }

    /// Total queued events across subscribers (tests / dry-run).
    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.subs.values().map(|sub| sub.queue.len()).sum()
    }
}
