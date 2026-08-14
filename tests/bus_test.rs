//! Comprehensive tests for the in-memory pub/sub Bus.

use fleet_cns_v3::bus::Bus;
use fleet_cns_v3::types::*;
use std::sync::Arc;

fn make_msg(channel: Channel, origin: &str) -> Arc<CnsMessage> {
    Arc::new(CnsMessage::new(
        channel,
        Priority::Normal,
        origin,
        Payload::Text { content: "test".into() },
    ))
}

// ─── Construction ────────────────────────────────────────────────────────────

#[test]
fn bus_new_creates_all_channels() {
    let bus = Bus::new();
    for ch in Channel::ALL {
        let count = bus.subscriber_count(ch);
        assert_eq!(count, 0, "channel {:?} should start with 0 subscribers", ch);
    }
}

#[test]
fn bus_starts_with_zero_published() {
    let bus = Bus::new();
    assert_eq!(bus.total_published(), 0);
}

#[test]
fn bus_uptime_is_positive() {
    let bus = Bus::new();
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(bus.uptime_secs() >= 1.0);
}

// ─── Subscribe / Unsubscribe ─────────────────────────────────────────────────

#[test]
fn subscribe_increments_count() {
    let bus = Bus::new();
    let _rx1 = bus.subscribe(Channel::Pulse);
    assert_eq!(bus.subscriber_count(&Channel::Pulse), 1);
    let _rx2 = bus.subscribe(Channel::Pulse);
    assert_eq!(bus.subscriber_count(&Channel::Pulse), 2);
}

#[test]
fn unsubscribe_decrements_count() {
    let bus = Bus::new();
    let _rx = bus.subscribe(Channel::Pulse);
    assert_eq!(bus.subscriber_count(&Channel::Pulse), 1);
    bus.unsubscribe(&Channel::Pulse);
    assert_eq!(bus.subscriber_count(&Channel::Pulse), 0);
}

#[test]
fn unsubscribe_does_not_go_negative() {
    let bus = Bus::new();
    bus.unsubscribe(&Channel::Pulse);
    bus.unsubscribe(&Channel::Pulse);
    assert_eq!(bus.subscriber_count(&Channel::Pulse), 0);
}

#[test]
fn subscribe_to_different_channels_independent() {
    let bus = Bus::new();
    let _rx1 = bus.subscribe(Channel::Pulse);
    let _rx2 = bus.subscribe(Channel::Creative);
    assert_eq!(bus.subscriber_count(&Channel::Pulse), 1);
    assert_eq!(bus.subscriber_count(&Channel::Creative), 1);
}

// ─── Publish ─────────────────────────────────────────────────────────────────

#[test]
fn publish_with_no_subscribers_returns_zero() {
    let bus = Bus::new();
    let delivered = bus.publish(make_msg(Channel::Pulse, "test"));
    assert_eq!(delivered, 0);
    assert_eq!(bus.total_published(), 1);
}

#[test]
fn publish_delivers_to_subscriber() {
    let bus = Bus::new();
    let mut rx = bus.subscribe(Channel::Pulse);
    let msg = make_msg(Channel::Pulse, "wesley");
    let delivered = bus.publish(msg.clone());
    assert_eq!(delivered, 1);
    let received = rx.try_recv().unwrap();
    assert_eq!(received.origin, "wesley");
}

#[test]
fn publish_delivers_to_multiple_subscribers() {
    let bus = Bus::new();
    let mut rx1 = bus.subscribe(Channel::Pulse);
    let mut rx2 = bus.subscribe(Channel::Pulse);
    let mut rx3 = bus.subscribe(Channel::Pulse);

    let delivered = bus.publish(make_msg(Channel::Pulse, "test"));
    assert_eq!(delivered, 3);

    assert!(rx1.try_recv().is_ok());
    assert!(rx2.try_recv().is_ok());
    assert!(rx3.try_recv().is_ok());
}

#[test]
fn publish_to_wrong_channel_does_not_deliver() {
    let bus = Bus::new();
    let mut rx = bus.subscribe(Channel::Pulse);
    let delivered = bus.publish(make_msg(Channel::Creative, "test"));
    assert_eq!(delivered, 0);
    assert!(rx.try_recv().is_err());
}

#[test]
fn publish_increments_total_counter() {
    let bus = Bus::new();
    bus.publish(make_msg(Channel::Pulse, "a"));
    bus.publish(make_msg(Channel::Pulse, "b"));
    bus.publish(make_msg(Channel::Creative, "c"));
    assert_eq!(bus.total_published(), 3);
}

#[test]
fn publish_increments_per_channel() {
    let bus = Bus::new();
    bus.publish(make_msg(Channel::Pulse, "a"));
    bus.publish(make_msg(Channel::Pulse, "b"));
    bus.publish(make_msg(Channel::Creative, "c"));

    let info = bus.channel_info();
    let pulse = info.iter().find(|c| c.channel == "PULSE").unwrap();
    let creative = info.iter().find(|c| c.channel == "CREATIVE").unwrap();
    assert_eq!(pulse.messages_published, 2);
    assert_eq!(creative.messages_published, 1);
}

// ─── Channel Info ────────────────────────────────────────────────────────────

#[test]
fn channel_info_returns_all_six() {
    let bus = Bus::new();
    let info = bus.channel_info();
    assert_eq!(info.len(), 6);
}

#[test]
fn channel_info_includes_subscriber_counts() {
    let bus = Bus::new();
    let _rx = bus.subscribe(Channel::Decision);
    let info = bus.channel_info();
    let decision = info.iter().find(|c| c.channel == "DECISION").unwrap();
    assert_eq!(decision.subscribers, 1);
}

// ─── Message ordering ────────────────────────────────────────────────────────

#[test]
fn messages_delivered_in_order() {
    let bus = Bus::new();
    let mut rx = bus.subscribe(Channel::Pulse);

    for i in 0..10 {
        let mut msg = CnsMessage::new(
            Channel::Pulse,
            Priority::Normal,
            "test",
            Payload::Text { content: i.to_string() },
        );
        msg.origin = format!("agent-{}", i);
        bus.publish(Arc::new(msg));
    }

    for i in 0..10 {
        let received = rx.try_recv().unwrap();
        assert_eq!(received.origin, format!("agent-{}", i));
    }
}

// ─── Broadcast capacity ──────────────────────────────────────────────────────

#[test]
fn publish_more_than_capacity_does_not_panic() {
    let bus = Bus::new();
    // Don't subscribe — just publish a lot
    for _ in 0..500 {
        bus.publish(make_msg(Channel::Pulse, "test"));
    }
    assert_eq!(bus.total_published(), 500);
}
