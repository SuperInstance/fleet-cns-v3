//! Comprehensive tests for CNS v3 types — Channel, Priority, Payload, CnsMessage, USCP round-trip.

use chrono::Utc;
use fleet_cns_v3::types::*;
use uuid::Uuid;

// ─── Channel tests ───────────────────────────────────────────────────────────

#[test]
fn channel_all_has_six_channels() {
    assert_eq!(Channel::ALL.len(), 6);
}

#[test]
fn channel_as_str_roundtrip() {
    for ch in Channel::ALL {
        let s = ch.as_str();
        let parsed: Channel = s.parse().unwrap();
        assert_eq!(*ch, parsed, "roundtrip failed for {}", s);
    }
}

#[test]
fn channel_display_matches_as_str() {
    for ch in Channel::ALL {
        assert_eq!(ch.to_string(), ch.as_str());
    }
}

#[test]
fn channel_from_str_case_insensitive() {
    assert_eq!("pulse".parse::<Channel>().unwrap(), Channel::Pulse);
    assert_eq!("Pulse".parse::<Channel>().unwrap(), Channel::Pulse);
    assert_eq!("CREATIVE".parse::<Channel>().unwrap(), Channel::Creative);
    assert_eq!("feel_tilt".parse::<Channel>().unwrap(), Channel::FeelTilt);
    assert_eq!("FEELTILT".parse::<Channel>().unwrap(), Channel::FeelTilt);
    assert_eq!("intent_broadcast".parse::<Channel>().unwrap(), Channel::IntentBroadcast);
    assert_eq!("INTENTBROADCAST".parse::<Channel>().unwrap(), Channel::IntentBroadcast);
}

#[test]
fn channel_from_str_rejects_unknown() {
    assert!("GARBAGE".parse::<Channel>().is_err());
    assert!("".parse::<Channel>().is_err());
    assert!("pulse ".parse::<Channel>().is_err());
}

#[test]
fn channel_serde_roundtrip() {
    let json = serde_json::to_string(&Channel::Creative).unwrap();
    assert_eq!(json, "\"CREATIVE\"");
    let back: Channel = serde_json::from_str(&json).unwrap();
    assert_eq!(back, Channel::Creative);
}

#[test]
fn channel_all_contains_expected() {
    assert!(Channel::ALL.contains(&Channel::Pulse));
    assert!(Channel::ALL.contains(&Channel::Status));
    assert!(Channel::ALL.contains(&Channel::Creative));
    assert!(Channel::ALL.contains(&Channel::Decision));
    assert!(Channel::ALL.contains(&Channel::FeelTilt));
    assert!(Channel::ALL.contains(&Channel::IntentBroadcast));
}

// ─── Priority tests ──────────────────────────────────────────────────────────

#[test]
fn priority_ordering() {
    assert!(Priority::Critical > Priority::High);
    assert!(Priority::High > Priority::Normal);
    assert!(Priority::Normal > Priority::Low);
}

#[test]
fn priority_default_is_normal() {
    assert_eq!(Priority::default(), Priority::Normal);
}

#[test]
fn priority_display() {
    assert_eq!(Priority::Critical.to_string(), "CRITICAL");
    assert_eq!(Priority::High.to_string(), "HIGH");
    assert_eq!(Priority::Normal.to_string(), "NORMAL");
    assert_eq!(Priority::Low.to_string(), "LOW");
}

#[test]
fn priority_from_str_roundtrip() {
    for p in [Priority::Low, Priority::Normal, Priority::High, Priority::Critical] {
        let s = p.to_string();
        let parsed: Priority = s.parse().unwrap();
        assert_eq!(p, parsed);
    }
}

#[test]
fn priority_from_str_case_insensitive() {
    assert_eq!("low".parse::<Priority>().unwrap(), Priority::Low);
    assert_eq!("critical".parse::<Priority>().unwrap(), Priority::Critical);
}

#[test]
fn priority_from_str_rejects_unknown() {
    assert!("URGENT".parse::<Priority>().is_err());
    assert!("".parse::<Priority>().is_err());
}

#[test]
fn priority_serde_screaming_snake() {
    let json = serde_json::to_string(&Priority::High).unwrap();
    assert_eq!(json, "\"HIGH\"");
}

// ─── Payload tests ───────────────────────────────────────────────────────────

#[test]
fn payload_pulse_serializes() {
    let p = Payload::Pulse {
        agent_id: "wesley".into(),
        status: "alive".into(),
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"kind\":\"pulse\""));
    assert!(json.contains("wesley"));
}

#[test]
fn payload_text_serializes() {
    let p = Payload::Text {
        content: "hello world".into(),
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"kind\":\"text\""));
    assert!(json.contains("hello world"));
}

#[test]
fn payload_decision_serializes() {
    let p = Payload::Decision {
        agent_id: "riker".into(),
        summary: "chose option B".into(),
        rationale: "lower risk".into(),
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"kind\":\"decision\""));
    assert!(json.contains("riker"));
    assert!(json.contains("chose option B"));
}

#[test]
fn payload_feel_tilt_serializes() {
    let p = Payload::FeelTilt {
        agent_id: "hermes".into(),
        mood: "contemplative".into(),
        intensity: 0.73,
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"kind\":\"feel_tilt\""));
    assert!(json.contains("0.73"));
}

#[test]
fn payload_intent_serializes() {
    let p = Payload::Intent {
        agent_id: "lucineer".into(),
        action: "starting_creative_loop".into(),
        target: Some("ai-writings".into()),
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"kind\":\"intent\""));
    assert!(json.contains("starting_creative_loop"));
}

#[test]
fn payload_intent_with_null_target() {
    let p = Payload::Intent {
        agent_id: "lucineer".into(),
        action: "idle".into(),
        target: None,
    };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("null"));
}

#[test]
fn payload_data_serializes() {
    let mut fields = serde_json::Map::new();
    fields.insert("key".into(), serde_json::Value::String("value".into()));
    let p = Payload::Data { fields };
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.contains("\"kind\":\"data\""));
    assert!(json.contains("key"));
}

// ─── CnsMessage tests ────────────────────────────────────────────────────────

#[test]
fn cns_message_new_assigns_fields() {
    let msg = CnsMessage::new(
        Channel::Pulse,
        Priority::Normal,
        "wesley",
        Payload::Pulse {
            agent_id: "wesley".into(),
            status: "alive".into(),
        },
    );
    assert_eq!(msg.channel, Channel::Pulse);
    assert_eq!(msg.priority, Priority::Normal);
    assert_eq!(msg.origin, "wesley");
    assert!(msg.destination.is_none());
    assert!(msg.correlation_id.is_none());
}

#[test]
fn cns_message_new_generates_unique_ids() {
    let msg1 = CnsMessage::new(
        Channel::Pulse, Priority::Normal, "a",
        Payload::Text { content: "1".into() },
    );
    let msg2 = CnsMessage::new(
        Channel::Pulse, Priority::Normal, "a",
        Payload::Text { content: "2".into() },
    );
    assert_ne!(msg1.id, msg2.id);
}

#[test]
fn cns_message_serde_roundtrip() {
    let msg = CnsMessage::new(
        Channel::Creative,
        Priority::High,
        "lucineer",
        Payload::Text { content: "the hermit crab finds a shell".into() },
    );
    let json = serde_json::to_string(&msg).unwrap();
    let back: CnsMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(back.channel, Channel::Creative);
    assert_eq!(back.priority, Priority::High);
    assert_eq!(back.origin, "lucineer");
}

// ─── USCP round-trip tests ───────────────────────────────────────────────────

#[test]
fn uscp_roundtrip_pulse() {
    let msg = CnsMessage::new(
        Channel::Pulse,
        Priority::Normal,
        "wesley",
        Payload::Pulse { agent_id: "wesley".into(), status: "alive".into() },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).expect("from_uscp failed");
    assert_eq!(back.channel, Channel::Pulse);
    assert_eq!(back.origin, "wesley");
}

#[test]
fn uscp_roundtrip_creative() {
    let msg = CnsMessage::new(
        Channel::Creative,
        Priority::Low,
        "lucineer",
        Payload::Text { content: "the ocean remembers".into() },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.channel, Channel::Creative);
}

#[test]
fn uscp_roundtrip_decision() {
    let msg = CnsMessage::new(
        Channel::Decision,
        Priority::Critical,
        "riker",
        Payload::Decision {
            agent_id: "riker".into(),
            summary: "divert power to engines".into(),
            rationale: "incoming storm".into(),
        },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.channel, Channel::Decision);
    assert_eq!(back.priority, Priority::Critical);
}

#[test]
fn uscp_roundtrip_feel_tilt() {
    let msg = CnsMessage::new(
        Channel::FeelTilt,
        Priority::Normal,
        "hermes",
        Payload::FeelTilt {
            agent_id: "hermes".into(),
            mood: "curious".into(),
            intensity: 0.88,
        },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.channel, Channel::FeelTilt);
}

#[test]
fn uscp_roundtrip_intent() {
    let msg = CnsMessage::new(
        Channel::IntentBroadcast,
        Priority::High,
        "kimi",
        Payload::Intent {
            agent_id: "kimi".into(),
            action: "spatial_analysis".into(),
            target: Some("roblox-build".into()),
        },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.channel, Channel::IntentBroadcast);
}

#[test]
fn uscp_roundtrip_status() {
    let msg = CnsMessage::new(
        Channel::Status,
        Priority::Normal,
        "wesley",
        Payload::Status {
            agent_id: "wesley".into(),
            state: "reading".into(),
            metrics: Some(serde_json::json!({"cpu": 45.2})),
        },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.channel, Channel::Status);
}

#[test]
fn uscp_preserves_priority() {
    let msg = CnsMessage::new(
        Channel::Pulse,
        Priority::Critical,
        "test",
        Payload::Text { content: "test".into() },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.priority, Priority::Critical);
}

#[test]
fn uscp_preserves_origin() {
    let msg = CnsMessage::new(
        Channel::Pulse,
        Priority::Normal,
        "deepseek-v4",
        Payload::Text { content: "test".into() },
    );
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.origin, "deepseek-v4");
}

#[test]
fn uscp_preserves_correlation_id() {
    let mut msg = CnsMessage::new(
        Channel::Pulse,
        Priority::Normal,
        "test",
        Payload::Text { content: "test".into() },
    );
    msg.correlation_id = Some("corr-123".into());
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.correlation_id, Some("corr-123".into()));
}

#[test]
fn uscp_preserves_destination() {
    let mut msg = CnsMessage::new(
        Channel::Pulse,
        Priority::Normal,
        "riker",
        Payload::Text { content: "test".into() },
    );
    msg.destination = Some("hermes".into());
    let uscp = msg.to_uscp();
    let back = CnsMessage::from_uscp(&uscp).unwrap();
    assert_eq!(back.destination, Some("hermes".into()));
}

#[test]
fn uscp_unknown_intent_goes_to_status_data() {
    let raw = serde_json::json!({
        "header": {
            "origin_id": "unknown-agent",
            "timestamp": "2026-08-14T07:00:00Z",
            "priority": "NORMAL",
        },
        "body": {
            "intent": "QUANTUM_FLUX",
            "payload": { "field1": "value1" }
        }
    });
    let msg = CnsMessage::from_uscp(&raw).unwrap();
    assert_eq!(msg.channel, Channel::Status);
    match &msg.payload {
        Payload::Data { fields } => {
            assert!(fields.contains_key("intent"));
            assert!(fields.contains_key("field1"));
        }
        _ => panic!("expected Data payload for unknown intent"),
    }
}

#[test]
fn uscp_missing_origin_returns_none() {
    let raw = serde_json::json!({
        "header": {},
        "body": { "intent": "PULSE", "payload": {} }
    });
    assert!(CnsMessage::from_uscp(&raw).is_none());
}

#[test]
fn uscp_missing_body_returns_none() {
    let raw = serde_json::json!({
        "header": { "origin_id": "test" }
    });
    assert!(CnsMessage::from_uscp(&raw).is_none());
}

#[test]
fn uscp_heartbeat_maps_to_pulse() {
    let raw = serde_json::json!({
        "header": {
            "origin_id": "wesley",
            "timestamp": "2026-08-14T07:00:00Z",
        },
        "body": {
            "intent": "HEARTBEAT",
            "payload": { "status": "alive" }
        }
    });
    let msg = CnsMessage::from_uscp(&raw).unwrap();
    assert_eq!(msg.channel, Channel::Pulse);
    match &msg.payload {
        Payload::Pulse { agent_id, status } => {
            assert_eq!(agent_id, "wesley");
            assert_eq!(status, "alive");
        }
        _ => panic!("expected Pulse payload"),
    }
}

#[test]
fn uscp_missing_priority_defaults_to_normal() {
    let raw = serde_json::json!({
        "header": { "origin_id": "test" },
        "body": { "intent": "PULSE", "payload": {} }
    });
    let msg = CnsMessage::from_uscp(&raw).unwrap();
    assert_eq!(msg.priority, Priority::Normal);
}

#[test]
fn uscp_missing_timestamp_uses_now() {
    let raw = serde_json::json!({
        "header": { "origin_id": "test" },
        "body": { "intent": "PULSE", "payload": {} }
    });
    let before = Utc::now();
    let msg = CnsMessage::from_uscp(&raw).unwrap();
    let after = Utc::now();
    assert!(msg.timestamp >= before);
    assert!(msg.timestamp <= after);
}

#[test]
fn uscp_signature_includes_version() {
    let msg = CnsMessage::new(
        Channel::Pulse, Priority::Normal, "test",
        Payload::Text { content: "x".into() },
    );
    let uscp = msg.to_uscp();
    let sig = uscp.get("signature").unwrap();
    assert_eq!(sig["version"], "3.0");
    assert_eq!(sig["type"], "USCP-v3");
}

// ─── PublishRequest / RelayRequest deserialization ───────────────────────────

#[test]
fn publish_request_deserializes() {
    let json = serde_json::json!({
        "channel": "PULSE",
        "priority": "HIGH",
        "origin": "wesley",
        "payload": { "content": "hello" }
    });
    let req: PublishRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.channel, "PULSE");
    assert_eq!(req.priority, "HIGH");
    assert_eq!(req.origin, "wesley");
    assert!(req.destination.is_none());
    assert!(req.correlation_id.is_none());
}

#[test]
fn publish_request_default_priority() {
    let json = serde_json::json!({
        "channel": "CREATIVE",
        "origin": "lucineer",
        "payload": { "content": "story" }
    });
    let req: PublishRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.priority, ""); // default is empty string from serde
}

#[test]
fn relay_request_deserializes() {
    let json = serde_json::json!({
        "packet": { "header": {}, "body": {} }
    });
    let req: RelayRequest = serde_json::from_value(json).unwrap();
    assert!(req.packet.is_object());
}
