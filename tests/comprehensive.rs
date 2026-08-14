//! Comprehensive tests for CNS v3 — types, bus pub/sub, store, USCP compat.

#[cfg(test)]
mod tests {
    use fleet_cns_v3::*;
    use std::str::FromStr;
    use std::sync::Arc;

    // ============================================================
    // TYPE SERIALIZATION
    // ============================================================

    #[test]
    fn channel_all_has_six_variants() {
        assert_eq!(types::Channel::ALL.len(), 6);
    }

    #[test]
    fn channel_display_matches_as_str() {
        for ch in types::Channel::ALL {
            assert_eq!(ch.to_string(), ch.as_str());
        }
    }

    #[test]
    fn channel_feel_tilt_accepts_concatenated_form() {
        assert_eq!(
            types::Channel::from_str("FEELTILT").unwrap(),
            types::Channel::FeelTilt
        );
    }

    #[test]
    fn channel_intent_broadcast_accepts_concatenated_form() {
        assert_eq!(
            types::Channel::from_str("INTENTBROADCAST").unwrap(),
            types::Channel::IntentBroadcast
        );
    }

    #[test]
    fn channel_serde_roundtrip_all_variants() {
        for ch in types::Channel::ALL {
            let json = serde_json::to_string(ch).unwrap();
            let back: types::Channel = serde_json::from_str(&json).unwrap();
            assert_eq!(*ch, back);
        }
    }

    #[test]
    fn channel_serde_uses_screaming_snake_case() {
        let json = serde_json::to_string(&types::Channel::FeelTilt).unwrap();
        assert_eq!(json, "\"FEEL_TILT\"");

        let json = serde_json::to_string(&types::Channel::IntentBroadcast).unwrap();
        assert_eq!(json, "\"INTENT_BROADCAST\"");
    }

    #[test]
    fn priority_default_is_normal() {
        assert_eq!(types::Priority::default(), types::Priority::Normal);
    }

    #[test]
    fn priority_serde_uses_screaming_snake_case() {
        let json = serde_json::to_string(&types::Priority::Critical).unwrap();
        assert_eq!(json, "\"CRITICAL\"");
    }

    #[test]
    fn priority_parse_invalid_fails() {
        assert!(types::Priority::from_str("URGENT").is_err());
        assert!(types::Priority::from_str("").is_err());
    }

    #[test]
    fn priority_ordering_total() {
        use types::Priority::*;
        assert!(Critical > High);
        assert!(High > Normal);
        assert!(Normal > Low);
        assert!(Critical > Low);
    }

    // ============================================================
    // PAYLOAD SERIALIZATION
    // ============================================================

    #[test]
    fn payload_pulse_serializes_with_kind_tag() {
        let p = types::Payload::Pulse {
            agent_id: "wesley".into(),
            status: "learning".into(),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "pulse");
        assert_eq!(json["data"]["agent_id"], "wesley");
        assert_eq!(json["data"]["status"], "learning");
    }

    #[test]
    fn payload_text_serializes() {
        let p = types::Payload::Text { content: "hello".into() };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(json["data"]["content"], "hello");
    }

    #[test]
    fn payload_decision_serializes() {
        let p = types::Payload::Decision {
            agent_id: "riker".into(),
            summary: "Deploy hotfix".into(),
            rationale: "Critical bug fixed".into(),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "decision");
        assert_eq!(json["data"]["summary"], "Deploy hotfix");
    }

    #[test]
    fn payload_feel_tilt_serializes() {
        let p = types::Payload::FeelTilt {
            agent_id: "engine".into(),
            mood: "contemplative".into(),
            intensity: 0.73,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "feel_tilt");
        assert_eq!(json["data"]["intensity"], 0.73);
    }

    #[test]
    fn payload_data_accepts_arbitrary_fields() {
        let mut fields = serde_json::Map::new();
        fields.insert("custom".into(), serde_json::json!(42));
        fields.insert("nested".into(), serde_json::json!({"a": 1}));
        let p = types::Payload::Data { fields };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "data");
        assert_eq!(json["data"]["custom"], 42);
    }

    #[test]
    fn payload_status_with_metrics() {
        let metrics = serde_json::json!({"cpu": 45.2, "mem_mb": 1024});
        let p = types::Payload::Status {
            agent_id: "monitor".into(),
            state: "working".into(),
            metrics: Some(metrics.clone()),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["data"]["metrics"]["cpu"], 45.2);
        assert_eq!(json["data"]["metrics"]["mem_mb"], 1024);
    }

    #[test]
    fn payload_status_without_metrics() {
        let p = types::Payload::Status {
            agent_id: "monitor".into(),
            state: "idle".into(),
            metrics: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["data"]["metrics"], serde_json::Value::Null);
    }

    #[test]
    fn payload_intent_with_target() {
        let p = types::Payload::Intent {
            agent_id: "lucineer".into(),
            action: "deploy".into(),
            target: Some("fleet-cns-v3".into()),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["data"]["target"], "fleet-cns-v3");
    }

    #[test]
    fn payload_intent_without_target() {
        let p = types::Payload::Intent {
            agent_id: "lucineer".into(),
            action: "scan".into(),
            target: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert!(json["data"]["target"].is_null());
    }

    // ============================================================
    // MESSAGE CREATION
    // ============================================================

    #[test]
    fn message_new_generates_unique_ids() {
        let msg1 = types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Normal,
            "a",
            types::Payload::Text { content: "1".into() },
        );
        let msg2 = types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Normal,
            "a",
            types::Payload::Text { content: "2".into() },
        );
        assert_ne!(msg1.id, msg2.id);
    }

    #[test]
    fn message_destination_defaults_none() {
        let msg = types::CnsMessage::new(
            types::Channel::Status,
            types::Priority::Normal,
            "x",
            types::Payload::Text { content: "y".into() },
        );
        assert!(msg.destination.is_none());
        assert!(msg.correlation_id.is_none());
    }

    #[test]
    fn message_timestamp_is_recent() {
        let before = chrono::Utc::now();
        let msg = types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Low,
            "t",
            types::Payload::Text { content: "t".into() },
        );
        let after = chrono::Utc::now();
        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    // ============================================================
    // BUS PUB/SUB
    // ============================================================

    #[test]
    fn bus_subscribe_and_receive() {
        let bus = bus::Bus::new();
        let mut rx = bus.subscribe(types::Channel::Pulse);

        let msg = Arc::new(types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Normal,
            "test",
            types::Payload::Pulse {
                agent_id: "test".into(),
                status: "ok".into(),
            },
        ));

        let delivered = bus.publish(msg.clone());
        assert_eq!(delivered, 1);

        let received = rx.try_recv().unwrap();
        assert_eq!(received.origin, "test");
        assert_eq!(received.channel, types::Channel::Pulse);
    }

    #[test]
    fn bus_multiple_subscribers_all_receive() {
        let bus = bus::Bus::new();
        let mut rx1 = bus.subscribe(types::Channel::Creative);
        let mut rx2 = bus.subscribe(types::Channel::Creative);
        let mut rx3 = bus.subscribe(types::Channel::Creative);

        let msg = Arc::new(types::CnsMessage::new(
            types::Channel::Creative,
            types::Priority::Normal,
            "writer",
            types::Payload::Text { content: "story".into() },
        ));

        let delivered = bus.publish(msg);
        assert_eq!(delivered, 3);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
        assert!(rx3.try_recv().is_ok());
    }

    #[test]
    fn bus_subscriber_only_gets_its_channel() {
        let bus = bus::Bus::new();
        let mut rx_pulse = bus.subscribe(types::Channel::Pulse);

        // Publish to a different channel
        let msg = Arc::new(types::CnsMessage::new(
            types::Channel::Creative,
            types::Priority::Normal,
            "writer",
            types::Payload::Text { content: "nope".into() },
        ));
        bus.publish(msg);

        // Pulse subscriber should not receive it
        assert!(rx_pulse.try_recv().is_err());
    }

    #[test]
    fn bus_unsubscribe_decrements_count() {
        let bus = bus::Bus::new();
        let _rx = bus.subscribe(types::Channel::Pulse);
        assert_eq!(bus.subscriber_count(&types::Channel::Pulse), 1);
        bus.unsubscribe(&types::Channel::Pulse);
        assert_eq!(bus.subscriber_count(&types::Channel::Pulse), 0);
    }

    #[test]
    fn bus_unsubscribe_does_not_go_negative() {
        let bus = bus::Bus::new();
        bus.unsubscribe(&types::Channel::Pulse);
        bus.unsubscribe(&types::Channel::Pulse);
        assert_eq!(bus.subscriber_count(&types::Channel::Pulse), 0);
    }

    #[test]
    fn bus_channel_info_reports_correctly() {
        let bus = bus::Bus::new();
        let _rx1 = bus.subscribe(types::Channel::Pulse);
        let _rx2 = bus.subscribe(types::Channel::Pulse);
        let _rx3 = bus.subscribe(types::Channel::Creative);

        let info = bus.channel_info();
        let pulse_info = info.iter().find(|c| c.channel == "PULSE").unwrap();
        assert_eq!(pulse_info.subscribers, 2);

        let creative_info = info.iter().find(|c| c.channel == "CREATIVE").unwrap();
        assert_eq!(creative_info.subscribers, 1);
    }

    #[test]
    fn bus_total_published_tracks_across_channels() {
        let bus = bus::Bus::new();

        for ch in types::Channel::ALL {
            let msg = Arc::new(types::CnsMessage::new(
                *ch,
                types::Priority::Normal,
                "test",
                types::Payload::Text { content: "x".into() },
            ));
            bus.publish(msg);
        }

        assert_eq!(bus.total_published(), 6);
    }

    #[test]
    fn bus_uptime_is_positive() {
        let bus = bus::Bus::new();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(bus.uptime_secs() >= 1.0);
    }

    // ============================================================
    // STORE OPERATIONS
    // ============================================================

    fn setup_store() -> (store::Store, tempfile::NamedTempFile) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let s = store::Store::open(tmp.path()).expect("open store");
        (s, tmp)
    }

    #[test]
    fn store_multiple_channels_stats() {
        let (store, _tmp) = setup_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for ch in [types::Channel::Pulse, types::Channel::Creative, types::Channel::Decision] {
                for i in 0..3 {
                    let msg = types::CnsMessage::new(
                        ch,
                        types::Priority::Normal,
                        format!("agent-{i}"),
                        types::Payload::Text { content: "x".into() },
                    );
                    store.store(&msg).await.unwrap();
                }
            }
        });

        let stats = store.stats();
        assert_eq!(stats.total_messages, 9);
        assert_eq!(stats.per_channel.len(), 3);

        // Each channel should have 3
        for (_, count) in &stats.per_channel {
            assert_eq!(*count, 3);
        }
    }

    #[test]
    fn store_replay_respects_count() {
        let (store, _tmp) = setup_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for i in 0..10 {
                let msg = types::CnsMessage::new(
                    types::Channel::Status,
                    types::Priority::Normal,
                    format!("agent-{i}"),
                    types::Payload::Status {
                        agent_id: format!("agent-{i}"),
                        state: "running".into(),
                        metrics: None,
                    },
                );
                store.store(&msg).await.unwrap();
            }
        });

        let replayed = store.replay(&types::Channel::Status, 3);
        assert_eq!(replayed.len(), 3);
    }

    #[test]
    fn store_replay_empty_channel() {
        let (store, _tmp) = setup_store();
        let replayed = store.replay(&types::Channel::FeelTilt, 10);
        assert!(replayed.is_empty());
    }

    #[test]
    fn store_replay_preserves_payload_types() {
        let (store, _tmp) = setup_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let msg = types::CnsMessage::new(
                types::Channel::Decision,
                types::Priority::High,
                "riker",
                types::Payload::Decision {
                    agent_id: "riker".into(),
                    summary: "Deploy".into(),
                    rationale: "Tests green".into(),
                },
            );
            store.store(&msg).await.unwrap();
        });

        let replayed = store.replay(&types::Channel::Decision, 1);
        assert_eq!(replayed.len(), 1);
        // The payload should deserialize back as Decision
        match &replayed[0].payload {
            types::Payload::Decision { summary, .. } => {
                assert_eq!(summary, "Deploy");
            }
            other => panic!("expected Decision, got {:?}", other),
        }
    }

    #[test]
    fn store_cleanup_removes_old_messages() {
        let (store, _tmp) = setup_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Store 5 messages
            for i in 0..5 {
                let msg = types::CnsMessage::new(
                    types::Channel::Pulse,
                    types::Priority::Normal,
                    format!("agent-{i}"),
                    types::Payload::Text { content: "x".into() },
                );
                store.store(&msg).await.unwrap();
            }

            // Cleanup all (future timestamp)
            let future = chrono::Utc::now() + chrono::Duration::hours(1);
            let deleted = store.cleanup_old(future).await.unwrap();
            assert_eq!(deleted, 5);
        });

        // Verify store is empty (stats uses blocking_lock, call outside runtime)
        let stats = store.stats();
        assert_eq!(stats.total_messages, 0);
    }

    #[test]
    fn store_cleanup_preserves_recent() {
        let (store, _tmp) = setup_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let msg = types::CnsMessage::new(
                types::Channel::Pulse,
                types::Priority::Normal,
                "recent",
                types::Payload::Text { content: "recent".into() },
            );
            store.store(&msg).await.unwrap();

            // Cleanup messages older than 1 year ago (should keep everything)
            let long_ago = chrono::Utc::now() - chrono::Duration::days(365);
            let deleted = store.cleanup_old(long_ago).await.unwrap();
            assert_eq!(deleted, 0);
        });

        // stats uses blocking_lock, call outside runtime
        let stats = store.stats();
        assert_eq!(stats.total_messages, 1);
    }

    #[test]
    fn store_seq_increments() {
        let (store, _tmp) = setup_store();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let msg1 = types::CnsMessage::new(
                types::Channel::Pulse,
                types::Priority::Normal,
                "a",
                types::Payload::Text { content: "1".into() },
            );
            let seq1 = store.store(&msg1).await.unwrap();

            let msg2 = types::CnsMessage::new(
                types::Channel::Pulse,
                types::Priority::Normal,
                "b",
                types::Payload::Text { content: "2".into() },
            );
            let seq2 = store.store(&msg2).await.unwrap();

            assert_eq!(seq1, 1);
            assert_eq!(seq2, 2);
        });
    }

    // ============================================================
    // USCP ROUNDTRIPS — ALL PAYLOAD TYPES
    // ============================================================

    #[test]
    fn uscp_roundtrip_status() {
        let original = types::CnsMessage::new(
            types::Channel::Status,
            types::Priority::Normal,
            "monitor",
            types::Payload::Status {
                agent_id: "monitor".into(),
                state: "online".into(),
                metrics: Some(serde_json::json!({"cpu": 50})),
            },
        );

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.channel, types::Channel::Status);
        assert_eq!(back.origin, "monitor");
    }

    #[test]
    fn uscp_roundtrip_creative() {
        let original = types::CnsMessage::new(
            types::Channel::Creative,
            types::Priority::Low,
            "deepseek",
            types::Payload::Text { content: "A poem about the sea".into() },
        );

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.channel, types::Channel::Creative);
        assert_eq!(back.priority, types::Priority::Low);
    }

    #[test]
    fn uscp_roundtrip_feel_tilt() {
        let original = types::CnsMessage::new(
            types::Channel::FeelTilt,
            types::Priority::Normal,
            "engine",
            types::Payload::FeelTilt {
                agent_id: "engine".into(),
                mood: "melancholic".into(),
                intensity: 0.82,
            },
        );

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.channel, types::Channel::FeelTilt);
    }

    #[test]
    fn uscp_roundtrip_decision() {
        let original = types::CnsMessage::new(
            types::Channel::Decision,
            types::Priority::Critical,
            "riker",
            types::Payload::Decision {
                agent_id: "riker".into(),
                summary: "All hands".into(),
                rationale: "Breach detected".into(),
            },
        );

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.channel, types::Channel::Decision);
        assert_eq!(back.priority, types::Priority::Critical);
    }

    #[test]
    fn uscp_roundtrip_intent() {
        let original = types::CnsMessage::new(
            types::Channel::IntentBroadcast,
            types::Priority::Normal,
            "kimi",
            types::Payload::Intent {
                agent_id: "kimi".into(),
                action: "build_start".into(),
                target: Some("vibe-world".into()),
            },
        );

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.channel, types::Channel::IntentBroadcast);
    }

    #[test]
    fn uscp_roundtrip_preserves_destination() {
        let mut original = types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Normal,
            "wesley",
            types::Payload::Pulse {
                agent_id: "wesley".into(),
                status: "alive".into(),
            },
        );
        original.destination = Some("riker".into());

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.destination.as_deref(), Some("riker"));
    }

    #[test]
    fn uscp_roundtrip_preserves_correlation_id() {
        let mut original = types::CnsMessage::new(
            types::Channel::Decision,
            types::Priority::High,
            "claude",
            types::Payload::Text { content: "test".into() },
        );
        original.correlation_id = Some("req-42".into());

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip");
        assert_eq!(back.correlation_id.as_deref(), Some("req-42"));
    }

    #[test]
    fn uscp_from_invalid_json_returns_none() {
        let cases = vec![
            serde_json::json!({}),  // empty
            serde_json::json!({"header": {}}),  // no origin
            serde_json::json!({"body": {}}),  // no header
            serde_json::json!("not an object"),  // wrong type
        ];

        for case in cases {
            assert!(
                types::CnsMessage::from_uscp(&case).is_none(),
                "should return None for: {case}"
            );
        }
    }

    #[test]
    fn uscp_to_uscp_includes_version_signature() {
        let msg = types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Normal,
            "test",
            types::Payload::Text { content: "x".into() },
        );

        let uscp = msg.to_uscp();
        assert_eq!(uscp["signature"]["type"], "USCP-v3");
        assert_eq!(uscp["signature"]["version"], "3.0");
        assert_eq!(uscp["signature"]["message_id"], msg.id.to_string());
    }

    #[test]
    fn uscp_intent_aliases() {
        // "HEARTBEAT" should map to PULSE channel
        let hb = serde_json::json!({
            "header": {"origin_id": "a", "timestamp": "2026-01-01T00:00:00Z", "priority": "NORMAL"},
            "body": {"intent": "HEARTBEAT", "payload": {"status": "alive"}}
        });
        let msg = types::CnsMessage::from_uscp(&hb).unwrap();
        assert_eq!(msg.channel, types::Channel::Pulse);

        // "MOOD" should map to FEEL_TILT channel
        let mood = serde_json::json!({
            "header": {"origin_id": "a", "timestamp": "2026-01-01T00:00:00Z", "priority": "NORMAL"},
            "body": {"intent": "MOOD", "payload": {"mood": "calm", "intensity": 0.3}}
        });
        let msg = types::CnsMessage::from_uscp(&mood).unwrap();
        assert_eq!(msg.channel, types::Channel::FeelTilt);

        // "STORY" should map to CREATIVE channel
        let story = serde_json::json!({
            "header": {"origin_id": "a", "timestamp": "2026-01-01T00:00:00Z", "priority": "NORMAL"},
            "body": {"intent": "STORY", "payload": {"content": "Once upon a time"}}
        });
        let msg = types::CnsMessage::from_uscp(&story).unwrap();
        assert_eq!(msg.channel, types::Channel::Creative);

        // "BROADCAST" should map to INTENT_BROADCAST
        let bcast = serde_json::json!({
            "header": {"origin_id": "a", "timestamp": "2026-01-01T00:00:00Z", "priority": "NORMAL"},
            "body": {"intent": "BROADCAST", "payload": {"action": "deploy"}}
        });
        let msg = types::CnsMessage::from_uscp(&bcast).unwrap();
        assert_eq!(msg.channel, types::Channel::IntentBroadcast);
    }

    #[test]
    fn uscp_missing_priority_defaults_normal() {
        let raw = serde_json::json!({
            "header": {"origin_id": "a", "timestamp": "2026-01-01T00:00:00Z"},
            "body": {"intent": "PULSE", "payload": {"status": "alive"}}
        });
        let msg = types::CnsMessage::from_uscp(&raw).unwrap();
        assert_eq!(msg.priority, types::Priority::Normal);
    }

    #[test]
    fn uscp_missing_timestamp_defaults_now() {
        let before = chrono::Utc::now();
        let raw = serde_json::json!({
            "header": {"origin_id": "a", "priority": "NORMAL"},
            "body": {"intent": "PULSE", "payload": {"status": "alive"}}
        });
        let msg = types::CnsMessage::from_uscp(&raw).unwrap();
        let after = chrono::Utc::now();
        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    #[test]
    fn uscp_invalid_priority_defaults_normal() {
        let raw = serde_json::json!({
            "header": {"origin_id": "a", "timestamp": "2026-01-01T00:00:00Z", "priority": "WHATEVER"},
            "body": {"intent": "PULSE", "payload": {"status": "alive"}}
        });
        let msg = types::CnsMessage::from_uscp(&raw).unwrap();
        assert_eq!(msg.priority, types::Priority::Normal);
    }
}
