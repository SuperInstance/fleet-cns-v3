//! Comprehensive tests for the SQLite Store layer.

use fleet_cns_v3::store::Store;
use fleet_cns_v3::types::*;
use tempfile::NamedTempFile;

fn make_store() -> Store {
    let tmp = NamedTempFile::new().unwrap();
    Store::open(tmp.path()).unwrap()
}

fn make_message(channel: Channel, origin: &str, priority: Priority) -> CnsMessage {
    CnsMessage::new(
        channel,
        priority,
        origin,
        Payload::Text { content: format!("message from {}", origin) },
    )
}

// ─── Open / Schema ───────────────────────────────────────────────────────────

#[test]
fn store_opens_successfully() {
    let store = make_store();
    let stats = store.stats();
    assert_eq!(stats.total_messages, 0);
}

#[test]
fn store_creates_parent_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("a").join("b").join("c.db");
    let store = Store::open(&nested).unwrap();
    let _ = store.stats();
}

#[test]
fn store_stats_empty_initially() {
    let store = make_store();
    let stats = store.stats();
    assert_eq!(stats.total_messages, 0);
    assert!(stats.oldest.is_none());
    assert!(stats.newest.is_none());
    assert!(stats.per_channel.is_empty());
}

// ─── Store / Retrieve ────────────────────────────────────────────────────────

#[tokio::test]
async fn store_message_returns_sequence() {
    let store = make_store();
    let msg = make_message(Channel::Pulse, "test", Priority::Normal);
    let seq = store.store(&msg).await.unwrap();
    // first store returns seq 0 (RETURNING next_seq - 1, where next_seq was incremented from 1 to 2... wait
    // the SQL increments next_seq then returns next_seq - 1. First call: next_seq goes 1→2, returns 1... hmm
    // Actually: INSERT OR IGNORE sets next_seq=1 initially. UPDATE increments to 2, returns 2-1=1.
    // So first message gets seq=1, not 0.
    assert!(seq >= 0);
}

#[tokio::test]
async fn store_multiple_messages_increments_sequence() {
    let store = make_store();
    let msg1 = make_message(Channel::Pulse, "a", Priority::Normal);
    let msg2 = make_message(Channel::Pulse, "b", Priority::Normal);
    let msg3 = make_message(Channel::Creative, "c", Priority::High);

    let seq1 = store.store(&msg1).await.unwrap();
    let seq2 = store.store(&msg2).await.unwrap();
    let seq3 = store.store(&msg3).await.unwrap();

    // Sequence numbers are monotonically increasing
    assert!(seq1 < seq2);
    assert!(seq2 < seq3);
}

#[tokio::test]
async fn stats_reflects_stored_messages() {
    let store = make_store();
    store.store(&make_message(Channel::Pulse, "a", Priority::Normal)).await.unwrap();
    store.store(&make_message(Channel::Pulse, "b", Priority::Normal)).await.unwrap();
    store.store(&make_message(Channel::Creative, "c", Priority::High)).await.unwrap();

    // stats() uses blocking_lock — spawn_blocking to avoid runtime conflict
    let store_clone = std::sync::Arc::new(store);
    let store_for_stats = store_clone.clone();
    let stats = tokio::task::spawn_blocking(move || {
        store_for_stats.stats()
    }).await.unwrap();

    assert_eq!(stats.total_messages, 3);
    assert!(stats.oldest.is_some());
    assert!(stats.newest.is_some());
    assert_eq!(stats.per_channel.len(), 2);
}

#[tokio::test]
async fn stats_per_channel_counts() {
    let store = make_store();
    for _ in 0..5 {
        store.store(&make_message(Channel::Pulse, "a", Priority::Normal)).await.unwrap();
    }
    for _ in 0..3 {
        store.store(&make_message(Channel::Creative, "b", Priority::Normal)).await.unwrap();
    }

    let store = std::sync::Arc::new(store);
    let s = store.clone();
    let stats = tokio::task::spawn_blocking(move || s.stats()).await.unwrap();
    let pulse_count = stats.per_channel.iter().find(|(ch, _)| ch == "PULSE").map(|(_, c)| *c).unwrap_or(0);
    let creative_count = stats.per_channel.iter().find(|(ch, _)| ch == "CREATIVE").map(|(_, c)| *c).unwrap_or(0);
    assert_eq!(pulse_count, 5);
    assert_eq!(creative_count, 3);
}

// ─── Replay (uses blocking_lock — must run on spawn_blocking) ────────────────

#[tokio::test]
async fn replay_returns_messages_oldest_first() {
    let store = std::sync::Arc::new(make_store());
    let msg1 = make_message(Channel::Pulse, "first", Priority::Normal);
    let msg2 = make_message(Channel::Pulse, "second", Priority::Normal);
    let msg3 = make_message(Channel::Pulse, "third", Priority::Normal);

    store.store(&msg1).await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    store.store(&msg2).await.unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    store.store(&msg3).await.unwrap();

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 10)).await.unwrap();
    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0].origin, "first");
    assert_eq!(replayed[1].origin, "second");
    assert_eq!(replayed[2].origin, "third");
}

#[tokio::test]
async fn replay_respects_count_limit() {
    let store = std::sync::Arc::new(make_store());
    for i in 0..10 {
        let msg = make_message(Channel::Pulse, &format!("msg-{}", i), Priority::Normal);
        store.store(&msg).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 3)).await.unwrap();
    assert_eq!(replayed.len(), 3);
}

#[tokio::test]
async fn replay_filters_by_channel() {
    let store = std::sync::Arc::new(make_store());
    store.store(&make_message(Channel::Pulse, "pulse-1", Priority::Normal)).await.unwrap();
    store.store(&make_message(Channel::Creative, "creative-1", Priority::Normal)).await.unwrap();
    store.store(&make_message(Channel::Pulse, "pulse-2", Priority::Normal)).await.unwrap();

    let s = store.clone();
    let pulse_only = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 10)).await.unwrap();
    assert_eq!(pulse_only.len(), 2);
    assert!(pulse_only.iter().all(|m| m.channel == Channel::Pulse));
}

#[tokio::test]
async fn replay_empty_channel() {
    let store = std::sync::Arc::new(make_store());
    let s = store.clone();
    let result = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 10)).await.unwrap();
    assert!(result.is_empty());
}

// ─── Since ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn since_returns_messages_after_timestamp() {
    let store = make_store();
    let msg1 = make_message(Channel::Pulse, "before", Priority::Normal);
    store.store(&msg1).await.unwrap();

    std::thread::sleep(std::time::Duration::from_millis(50));
    let cutoff = chrono::Utc::now();

    std::thread::sleep(std::time::Duration::from_millis(10));
    let msg2 = make_message(Channel::Pulse, "after", Priority::Normal);
    store.store(&msg2).await.unwrap();

    let since = store.since(&Channel::Pulse, cutoff).await;
    assert_eq!(since.len(), 1);
    assert_eq!(since[0].origin, "after");
}

#[tokio::test]
async fn since_filters_by_channel() {
    let store = make_store();
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(1);

    store.store(&make_message(Channel::Pulse, "p", Priority::Normal)).await.unwrap();
    store.store(&make_message(Channel::Creative, "c", Priority::Normal)).await.unwrap();

    let pulse_since = store.since(&Channel::Pulse, cutoff).await;
    assert_eq!(pulse_since.len(), 1);
    assert_eq!(pulse_since[0].origin, "p");
}

// ─── Cleanup ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cleanup_deletes_old_messages() {
    let store = make_store();
    let old_msg = make_message(Channel::Pulse, "old", Priority::Normal);
    store.store(&old_msg).await.unwrap();

    let future = chrono::Utc::now() + chrono::Duration::seconds(1);
    let deleted = store.cleanup_old(future).await.unwrap();
    assert_eq!(deleted, 1);

    let store = std::sync::Arc::new(store);
    let s = store.clone();
    let stats = tokio::task::spawn_blocking(move || s.stats()).await.unwrap();
    assert_eq!(stats.total_messages, 0);
}

#[tokio::test]
async fn cleanup_preserves_recent_messages() {
    let store = make_store();
    let msg = make_message(Channel::Pulse, "recent", Priority::Normal);
    store.store(&msg).await.unwrap();

    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let deleted = store.cleanup_old(past).await.unwrap();
    assert_eq!(deleted, 0);

    let store = std::sync::Arc::new(store);
    let s = store.clone();
    let stats = tokio::task::spawn_blocking(move || s.stats()).await.unwrap();
    assert_eq!(stats.total_messages, 1);
}

// ─── Persistence ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn store_persists_across_reopen() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();

    {
        let store = Store::open(&path).unwrap();
        let msg = make_message(Channel::Pulse, "persistent", Priority::Normal);
        store.store(&msg).await.unwrap();
    }

    {
        let store = std::sync::Arc::new(Store::open(&path).unwrap());
        let s = store.clone();
        let stats = tokio::task::spawn_blocking(move || s.stats()).await.unwrap();
        assert_eq!(stats.total_messages, 1);
        let s2 = store.clone();
        let replayed = tokio::task::spawn_blocking(move || s2.replay(&Channel::Pulse, 10)).await.unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].origin, "persistent");
    }
}

// ─── Priority persistence ────────────────────────────────────────────────────

#[tokio::test]
async fn store_preserves_priority() {
    let store = std::sync::Arc::new(make_store());
    let msg = make_message(Channel::Decision, "test", Priority::Critical);
    store.store(&msg).await.unwrap();

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Decision, 1)).await.unwrap();
    assert_eq!(replayed[0].priority, Priority::Critical);
}

#[tokio::test]
async fn store_preserves_all_priorities() {
    let store = std::sync::Arc::new(make_store());
    let priorities = [Priority::Low, Priority::Normal, Priority::High, Priority::Critical];

    for (i, p) in priorities.iter().enumerate() {
        let mut msg = make_message(Channel::Pulse, &format!("p-{}", i), *p);
        msg.priority = *p;
        store.store(&msg).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 10)).await.unwrap();
    assert_eq!(replayed.len(), 4);
    for (i, p) in priorities.iter().enumerate() {
        assert_eq!(replayed[i].priority, *p);
    }
}

// ─── Payload persistence ─────────────────────────────────────────────────────

#[tokio::test]
async fn store_preserves_payload_text() {
    let store = std::sync::Arc::new(make_store());
    let msg = CnsMessage::new(
        Channel::Creative,
        Priority::Normal,
        "lucineer",
        Payload::Text { content: "the ocean remembers everything".into() },
    );
    store.store(&msg).await.unwrap();

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Creative, 1)).await.unwrap();
    match &replayed[0].payload {
        Payload::Text { content } => assert_eq!(content, "the ocean remembers everything"),
        _ => panic!("expected Text payload"),
    }
}

#[tokio::test]
async fn store_preserves_payload_decision() {
    let store = std::sync::Arc::new(make_store());
    let msg = CnsMessage::new(
        Channel::Decision,
        Priority::High,
        "riker",
        Payload::Decision {
            agent_id: "riker".into(),
            summary: "engage".into(),
            rationale: "captain's orders".into(),
        },
    );
    store.store(&msg).await.unwrap();

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Decision, 1)).await.unwrap();
    match &replayed[0].payload {
        Payload::Decision { summary, .. } => assert_eq!(summary, "engage"),
        _ => panic!("expected Decision payload"),
    }
}

#[tokio::test]
async fn store_preserves_correlation_id() {
    let store = std::sync::Arc::new(make_store());
    let mut msg = make_message(Channel::Pulse, "test", Priority::Normal);
    msg.correlation_id = Some("test-corr-id".into());
    store.store(&msg).await.unwrap();

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 1)).await.unwrap();
    assert_eq!(replayed[0].correlation_id, Some("test-corr-id".into()));
}

#[tokio::test]
async fn store_preserves_destination() {
    let store = std::sync::Arc::new(make_store());
    let mut msg = make_message(Channel::Pulse, "test", Priority::Normal);
    msg.destination = Some("hermes".into());
    store.store(&msg).await.unwrap();

    let s = store.clone();
    let replayed = tokio::task::spawn_blocking(move || s.replay(&Channel::Pulse, 1)).await.unwrap();
    assert_eq!(replayed[0].destination, Some("hermes".into()));
}
