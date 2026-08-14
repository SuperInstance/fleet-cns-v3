//! In-memory pub/sub bus with priority-aware broadcasting.
//!
//! Each channel has a list of broadcast receivers. Messages are pushed
//! to all subscribers. SQLite is the source of truth; the bus is the
//! real-time fan-out layer.

use crate::types::{Channel, CnsMessage};
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, trace};

/// Capacity of each channel's broadcast buffer.
/// Messages beyond this are dropped (subscribers can replay from SQLite).
const CHANNEL_CAPACITY: usize = 256;

pub struct Bus {
    /// One broadcast sender per channel.
    senders: DashMap<Channel, broadcast::Sender<Arc<CnsMessage>>>,
    /// Subscriber counts (derived from sender receiver_count, but tracked separately for stats).
    subscriber_counts: DashMap<Channel, usize>,
    /// Total messages published since start.
    total_published: std::sync::atomic::AtomicU64,
    /// Messages published per channel since start.
    per_channel: DashMap<Channel, u64>,
    /// Start time for rate calculation.
    started_at: chrono::DateTime<chrono::Utc>,
}

impl Bus {
    pub fn new() -> Arc<Self> {
        let bus = Arc::new(Self {
            senders: DashMap::new(),
            subscriber_counts: DashMap::new(),
            total_published: std::sync::atomic::AtomicU64::new(0),
            per_channel: DashMap::new(),
            started_at: chrono::Utc::now(),
        });

        // Pre-create all known channels
        for ch in Channel::ALL {
            let (tx, _rx) = broadcast::channel::<Arc<CnsMessage>>(CHANNEL_CAPACITY);
            bus.senders.insert(*ch, tx);
            bus.subscriber_counts.insert(*ch, 0);
            bus.per_channel.insert(*ch, 0);
        }

        bus
    }

    /// Subscribe to a channel. Returns a receiver that gets future messages.
    pub fn subscribe(&self, channel: Channel) -> broadcast::Receiver<Arc<CnsMessage>> {
        let entry = self.senders.entry(channel).or_insert_with(|| {
            let (tx, _rx) = broadcast::channel::<Arc<CnsMessage>>(CHANNEL_CAPACITY);
            tx
        });

        let count = entry.receiver_count();
        drop(entry);
        self.subscriber_counts.insert(channel, count + 1);

        self.senders
            .get(&channel)
            .expect("channel must exist")
            .subscribe()
    }

    /// Unsubscribe (decrement count). Called when a subscriber drops.
    pub fn unsubscribe(&self, channel: &Channel) {
        if let Some(mut count) = self.subscriber_counts.get_mut(channel) {
            if *count > 0 {
                *count -= 1;
            }
        }
    }

    /// Publish a message to all subscribers on the channel.
    /// Returns the number of subscribers that received it.
    pub fn publish(&self, msg: Arc<CnsMessage>) -> usize {
        let channel = msg.channel;

        // Update stats
        self.total_published
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.per_channel
            .entry(channel)
            .and_modify(|c| *c += 1)
            .or_insert(1);

        let delivered = if let Some(sender) = self.senders.get(&channel) {
            match sender.send(msg) {
                Ok(n) => {
                    trace!(channel = %channel, delivered = n, "message published");
                    n
                }
                Err(_) => {
                    debug!(channel = %channel, "no subscribers for message");
                    0
                }
            }
        } else {
            debug!(channel = %channel, "unknown channel, creating on-the-fly");
            let (tx, _rx) = broadcast::channel::<Arc<CnsMessage>>(CHANNEL_CAPACITY);
            self.senders.insert(channel, tx.clone());
            self.subscriber_counts.insert(channel, 0);
            self.per_channel.insert(channel, 1);
            match tx.send(msg) {
                Ok(n) => n,
                Err(_) => 0,
            }
        };

        delivered
    }

    /// Get current subscriber count for a channel.
    pub fn subscriber_count(&self, channel: &Channel) -> usize {
        self.subscriber_counts.get(channel).map(|c| *c).unwrap_or(0)
    }

    /// Get all channel info for the status endpoint.
    pub fn channel_info(&self) -> Vec<ChannelInfo> {
        Channel::ALL
            .iter()
            .map(|ch| {
                let subscribers = self.subscriber_count(ch);
                let total: u64 = self.per_channel.get(ch).map(|c| *c).unwrap_or(0);
                ChannelInfo {
                    channel: ch.as_str().to_string(),
                    subscribers,
                    messages_published: total,
                }
            })
            .collect()
    }

    /// Total messages since start.
    pub fn total_published(&self) -> u64 {
        self.total_published.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Uptime in seconds.
    pub fn uptime_secs(&self) -> f64 {
        (chrono::Utc::now() - self.started_at).num_seconds() as f64
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ChannelInfo {
    pub channel: String,
    pub subscribers: usize,
    pub messages_published: u64,
}
