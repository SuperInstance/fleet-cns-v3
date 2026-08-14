#[cfg(test)]
mod tests {
    use fleet_cns_v3::*;
    use std::str::FromStr;

    // ---- Channel tests ----

    #[test]
    fn channel_roundtrip() {
        for ch in types::Channel::ALL {
            let s = ch.as_str();
            let back: types::Channel = s.parse().unwrap();
            assert_eq!(*ch, back);
        }
    }

    #[test]
    fn channel_case_insensitive() {
        assert_eq!(
            types::Channel::from_str("pulse").unwrap(),
            types::Channel::Pulse
        );
        assert_eq!(
            types::Channel::from_str("Feel_Tilt").unwrap(),
            types::Channel::FeelTilt
        );
    }

    #[test]
    fn channel_unknown_fails() {
        assert!(types::Channel::from_str("GARBAGE").is_err());
    }

    // ---- Priority tests ----

    #[test]
    fn priority_ordering() {
        assert!(types::Priority::Critical > types::Priority::High);
        assert!(types::Priority::High > types::Priority::Normal);
        assert!(types::Priority::Normal > types::Priority::Low);
    }

    #[test]
    fn priority_roundtrip() {
        for p in [
            types::Priority::Critical,
            types::Priority::High,
            types::Priority::Normal,
            types::Priority::Low,
        ] {
            let s = p.to_string();
            let back: types::Priority = s.parse().unwrap();
            assert_eq!(p, back);
        }
    }

    // ---- Message tests ----

    #[test]
    fn message_creation() {
        let msg = types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::High,
            "test-agent",
            types::Payload::Pulse {
                agent_id: "test-agent".into(),
                status: "alive".into(),
            },
        );
        assert_eq!(msg.channel, types::Channel::Pulse);
        assert_eq!(msg.priority, types::Priority::High);
        assert_eq!(msg.origin, "test-agent");
        assert!(msg.destination.is_none());
    }

    // ---- USCP conversion tests ----

    #[test]
    fn uscp_to_cns_pulse() {
        let raw = serde_json::json!({
            "header": {
                "origin_id": "hermes-cns",
                "timestamp": "2026-08-08T05:55:00+00:00",
                "priority": "NORMAL",
                "destination_id": "lucineer-riker",
                "correlation_id": "corr-123"
            },
            "body": {
                "intent": "PULSE",
                "payload": { "status": "alive" }
            }
        });

        let msg = types::CnsMessage::from_uscp(&raw).expect("should parse");
        assert_eq!(msg.channel, types::Channel::Pulse);
        assert_eq!(msg.origin, "hermes-cns");
        assert_eq!(msg.priority, types::Priority::Normal);
        assert_eq!(msg.correlation_id.as_deref(), Some("corr-123"));
    }

    #[test]
    fn uscp_to_cns_creative() {
        let raw = serde_json::json!({
            "header": {
                "origin_id": "lucineer",
                "timestamp": "2026-08-08T05:55:00+00:00",
                "priority": "HIGH",
                "destination_id": null,
            },
            "body": {
                "intent": "CREATIVE",
                "payload": { "content": "A story about the stars" }
            }
        });

        let msg = types::CnsMessage::from_uscp(&raw).expect("should parse");
        assert_eq!(msg.channel, types::Channel::Creative);
        assert_eq!(msg.priority, types::Priority::High);
    }

    #[test]
    fn uscp_to_cns_decision() {
        let raw = serde_json::json!({
            "header": {
                "origin_id": "riker",
                "timestamp": "2026-08-08T05:55:00+00:00",
                "priority": "CRITICAL",
            },
            "body": {
                "intent": "DECISION",
                "payload": {
                    "summary": "Deploy update",
                    "rationale": "Tests pass"
                }
            }
        });

        let msg = types::CnsMessage::from_uscp(&raw).expect("should parse");
        assert_eq!(msg.channel, types::Channel::Decision);
        assert_eq!(msg.priority, types::Priority::Critical);
    }

    #[test]
    fn uscp_unknown_intent_goes_to_status() {
        let raw = serde_json::json!({
            "header": {
                "origin_id": "test",
                "timestamp": "2026-08-08T05:55:00+00:00",
                "priority": "LOW",
            },
            "body": {
                "intent": "WAT",
                "payload": { "foo": "bar" }
            }
        });

        let msg = types::CnsMessage::from_uscp(&raw).expect("should parse");
        assert_eq!(msg.channel, types::Channel::Status);
        assert_eq!(msg.priority, types::Priority::Low);
    }

    #[test]
    fn uscp_roundtrip() {
        let original = types::CnsMessage::new(
            types::Channel::IntentBroadcast,
            types::Priority::High,
            "lucineer",
            types::Payload::Intent {
                agent_id: "lucineer".into(),
                action: "build_start".into(),
                target: Some("vibe-world".into()),
            },
        );

        let uscp = original.to_uscp();
        let back = types::CnsMessage::from_uscp(&uscp).expect("roundtrip should work");
        assert_eq!(back.channel, original.channel);
        assert_eq!(back.priority, original.priority);
        assert_eq!(back.origin, original.origin);
    }

    // ---- Bus tests ----

    #[test]
    fn bus_publish_no_subscribers() {
        let bus = bus::Bus::new();
        let msg = std::sync::Arc::new(types::CnsMessage::new(
            types::Channel::Pulse,
            types::Priority::Normal,
            "test",
            types::Payload::Text { content: "hi".into() },
        ));
        let delivered = bus.publish(msg);
        assert_eq!(delivered, 0);
        assert_eq!(bus.total_published(), 1);
    }

    // ---- SQLite store tests ----

    #[test]
    fn store_open_and_query() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = store::Store::open(tmp.path()).expect("open store");
        let stats = store.stats();
        assert_eq!(stats.total_messages, 0);
    }

    #[test]
    fn store_and_replay() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = store::Store::open(tmp.path()).expect("open store");

        // Store is async, but tests are sync — use a minimal runtime
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for i in 0..5 {
                let msg = types::CnsMessage::new(
                    types::Channel::Pulse,
                    types::Priority::Normal,
                    format!("agent-{i}"),
                    types::Payload::Pulse {
                        agent_id: format!("agent-{i}"),
                        status: "alive".into(),
                    },
                );
                store.store(&msg).await.unwrap();
            }
        });

        let replayed = store.replay(&types::Channel::Pulse, 10);
        assert_eq!(replayed.len(), 5);

        let stats = store.stats();
        assert_eq!(stats.total_messages, 5);
        assert_eq!(stats.per_channel.len(), 1);
    }

    #[test]
    fn store_cleanup() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = store::Store::open(tmp.path()).expect("open store");

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let msg = types::CnsMessage::new(
                types::Channel::Pulse,
                types::Priority::Normal,
                "old-agent",
                types::Payload::Text { content: "old".into() },
            );
            store.store(&msg).await.unwrap();

            // Cleanup everything older than now+1s (should delete the message)
            let future = chrono::Utc::now() + chrono::Duration::seconds(1);
            let deleted = store.cleanup_old(future).await.unwrap();
            assert_eq!(deleted, 1);
        });
    }
}
