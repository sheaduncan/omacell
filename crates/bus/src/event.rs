//! Bounded in-process event subscriptions.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;

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
    byte_cap: usize,
    queued_bytes: usize,
    filter: BTreeSet<String>,
    queue: VecDeque<QueuedEvent>,
    dropped: u64,
}

#[derive(Clone)]
struct QueuedEvent {
    event: Event,
    bytes: usize,
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
            byte_cap: self.byte_cap,
            queued_bytes: self.queued_bytes,
            filter: self.filter.clone(),
            queue: self.queue.clone(),
            dropped: self.dropped,
        }
    }
}

impl std::fmt::Debug for Sub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sub")
            .field("cap", &self.cap)
            .field("byte_cap", &self.byte_cap)
            .field("queued_bytes", &self.queued_bytes)
            .field("filter", &self.filter)
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
        self.subscribe_filtered(cap, usize::MAX, &[])
    }

    /// Subscribe with count/byte limits and an optional event-type allowlist.
    pub(crate) fn subscribe_filtered(
        &mut self,
        cap: usize,
        byte_cap: usize,
        filter: &[String],
    ) -> SubscriberId {
        let id = self.next;
        self.next += 1;
        self.subs.insert(
            id,
            Sub {
                cap: cap.max(1),
                byte_cap: byte_cap.max(1),
                queued_bytes: 0,
                filter: filter.iter().cloned().collect(),
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
            match event_type_name(&event) {
                Some(event_type) if !sub.filter.is_empty() && !sub.filter.contains(event_type) => {
                    continue;
                }
                None if !sub.filter.is_empty() => continue,
                _ => {}
            }
            let bytes = queued_event_bytes(&event, sub.byte_cap);
            push_event(sub, event.clone(), bytes);
        }
    }

    /// Drain queued events for `id`.
    pub fn drain(&mut self, id: SubscriberId) -> Vec<Event> {
        self.subs
            .get_mut(&id.0)
            .map(|sub| {
                sub.queued_bytes = 0;
                sub.queue.drain(..).map(|queued| queued.event).collect()
            })
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

fn push_event(sub: &mut Sub, event: Event, bytes: usize) {
    if bytes > sub.byte_cap {
        sub.dropped = sub.dropped.saturating_add(1);
        return;
    }
    while sub.queue.len() >= sub.cap || sub.queued_bytes.saturating_add(bytes) > sub.byte_cap {
        if let Some(oldest) = sub.queue.pop_front() {
            sub.queued_bytes = sub.queued_bytes.saturating_sub(oldest.bytes);
            sub.dropped = sub.dropped.saturating_add(1);
        } else {
            break;
        }
    }
    sub.queued_bytes = sub.queued_bytes.saturating_add(bytes);
    sub.queue.push_back(QueuedEvent { event, bytes });
}

fn queued_event_bytes(event: &Event, byte_cap: usize) -> usize {
    if byte_cap == usize::MAX {
        return 0;
    }
    let mut counter = ByteCounter(64); // Conservative server-record envelope allowance.
    if serde_json::to_writer(&mut counter, event).is_err() {
        return usize::MAX;
    }
    counter.0
}

struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Frozen wire tag for a known event variant.
#[must_use]
pub(crate) fn event_type_name(event: &Event) -> Option<&'static str> {
    match event {
        Event::WorkbookOpened { .. } => Some("workbook_opened"),
        Event::CellChanged { .. } => Some("cell_changed"),
        Event::RecalcDone { .. } => Some("recalc_done"),
        Event::BeforeSave { .. } => Some("before_save"),
        Event::FileSaved { .. } => Some("file_saved"),
        Event::ChangesetProposed { .. } => Some("changeset_proposed"),
        Event::ChangesetApplied { .. } => Some("changeset_applied"),
        Event::ChangesetReverted { .. } => Some("changeset_reverted"),
        Event::ThemeChanged { .. } => Some("theme_changed"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_and_oversized_events_never_enter_the_queue() {
        let mut bus = EventBus::new();
        let filter = vec!["recalc_done".to_string()];
        let filtered = bus.subscribe_filtered(2, 256, &filter);
        bus.emit(Event::ThemeChanged {
            name: "ignored".into(),
        });
        assert!(bus.drain(filtered).is_empty());
        assert_eq!(bus.dropped(filtered), 0);

        let all = bus.subscribe_filtered(2, 256, &[]);
        bus.emit(Event::ThemeChanged {
            name: "x".repeat(512),
        });
        assert!(bus.drain(all).is_empty());
        assert_eq!(bus.dropped(all), 1);
    }
}
