use std::sync::mpsc::{self, Receiver, Sender};
#[cfg(test)]
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex};

/// In-process fan-out bus for unified-service events. The fetcher publishes,
/// and any number of consumers (inbox writer, HTTP/SSE server, dispatcher)
/// subscribe a fresh receiver. No dependency on the tokio broadcast model —
/// we just keep a vector of mpsc senders and prune on the next publish when
/// the receiver has gone.
#[derive(Clone, Debug)]
pub enum Event {
    InboxUpdated {
        last_poll: String,
        total: usize,
        new_count: usize,
    },
    Activity(String),
}

#[derive(Clone, Debug, Default)]
pub struct Bus {
    senders: Arc<Mutex<Vec<Sender<Event>>>>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn subscribe(&self) -> BusReceiver {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut guard) = self.senders.lock() {
            guard.push(tx);
        }
        BusReceiver { inner: rx }
    }

    pub fn publish(&self, event: Event) {
        let Ok(mut guard) = self.senders.lock() else {
            return;
        };
        guard.retain(|sender| sender.send(event.clone()).is_ok());
    }

    #[cfg(test)]
    pub fn subscriber_count(&self) -> usize {
        self.senders
            .lock()
            .map(|guard| guard.len())
            .unwrap_or_default()
    }
}

pub struct BusReceiver {
    inner: Receiver<Event>,
}

impl BusReceiver {
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<Event> {
        self.inner.recv_timeout(timeout).ok()
    }

    #[cfg(test)]
    pub fn try_recv(&self) -> Result<Event, TryRecvError> {
        self.inner.try_recv()
    }
}

#[cfg(test)]
mod tests {
    use super::{Bus, Event};
    use std::time::Duration;

    #[test]
    fn publish_reaches_all_subscribers() {
        let bus = Bus::new();
        let a = bus.subscribe();
        let b = bus.subscribe();
        bus.publish(Event::InboxUpdated {
            last_poll: "t".to_string(),
            total: 2,
            new_count: 1,
        });
        let received_a = a.recv_timeout(Duration::from_millis(50));
        let received_b = b.recv_timeout(Duration::from_millis(50));
        assert!(matches!(
            received_a,
            Some(Event::InboxUpdated { total: 2, new_count: 1, .. })
        ));
        assert!(matches!(
            received_b,
            Some(Event::InboxUpdated { total: 2, new_count: 1, .. })
        ));
    }

    #[test]
    fn dropped_receivers_are_pruned_on_next_publish() {
        let bus = Bus::new();
        {
            let _short = bus.subscribe();
            assert_eq!(bus.subscriber_count(), 1);
        }
        // Publishing after drop should detect the closed channel and prune it.
        bus.publish(Event::Activity("x".to_string()));
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn second_publish_after_prune_still_reaches_live_subscriber() {
        let bus = Bus::new();
        let alive = bus.subscribe();
        {
            let _short = bus.subscribe();
        }
        bus.publish(Event::Activity("drop-stale".to_string()));
        bus.publish(Event::Activity("deliver".to_string()));
        // Should receive both events, not lose the second one to the retain.
        let first = alive.recv_timeout(Duration::from_millis(50));
        let second = alive.recv_timeout(Duration::from_millis(50));
        assert!(matches!(first, Some(Event::Activity(_))));
        assert!(matches!(second, Some(Event::Activity(_))));
    }
}
