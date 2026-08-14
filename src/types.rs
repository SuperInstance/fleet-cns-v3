//! Core types for CNS v3 — typed channels, priorities, payloads, and the CnsMessage.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Known CNS channels — replacing untyped JSON intents with a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Channel {
    /// Heartbeat / keep-alive pulses from agents
    Pulse,
    /// Status updates (online/offline, working/idle, resource usage)
    Status,
    /// Creative output (stories, art, music, ideas)
    Creative,
    /// Decisions made by agents (should be logged for audit)
    Decision,
    /// Emotional tilt / mood shifts — the "feel" of the fleet
    FeelTilt,
    /// Broadcast intents (agent announces it's starting/finishing something)
    IntentBroadcast,
}

impl Channel {
    pub const ALL: &'static [Channel] = &[
        Channel::Pulse,
        Channel::Status,
        Channel::Creative,
        Channel::Decision,
        Channel::FeelTilt,
        Channel::IntentBroadcast,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Pulse => "PULSE",
            Channel::Status => "STATUS",
            Channel::Creative => "CREATIVE",
            Channel::Decision => "DECISION",
            Channel::FeelTilt => "FEEL_TILT",
            Channel::IntentBroadcast => "INTENT_BROADCAST",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for Channel {
    type Err = ChannelParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "PULSE" => Ok(Channel::Pulse),
            "STATUS" => Ok(Channel::Status),
            "CREATIVE" => Ok(Channel::Creative),
            "DECISION" => Ok(Channel::Decision),
            "FEEL_TILT" | "FEELTILT" => Ok(Channel::FeelTilt),
            "INTENT_BROADCAST" | "INTENTBROADCAST" => Ok(Channel::IntentBroadcast),
            other => Err(ChannelParseError(other.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("unknown channel: {0}")]
pub struct ChannelParseError(String);

/// Message priority — CRITICAL messages jump the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for Priority {
    fn default() -> Self { Priority::Normal }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Priority::Critical => write!(f, "CRITICAL"),
            Priority::High => write!(f, "HIGH"),
            Priority::Normal => write!(f, "NORMAL"),
            Priority::Low => write!(f, "LOW"),
        }
    }
}

impl FromStr for Priority {
    type Err = PriorityParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "CRITICAL" => Ok(Priority::Critical),
            "HIGH" => Ok(Priority::High),
            "NORMAL" => Ok(Priority::Normal),
            "LOW" => Ok(Priority::Low),
            other => Err(PriorityParseError(other.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("unknown priority: {0}")]
pub struct PriorityParseError(String);

/// Typed payloads — not arbitrary JSON blobs.
/// Each variant maps to a kind of content that agents actually send.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Payload {
    /// Heartbeat pulse: agent is alive
    Pulse {
        agent_id: String,
        status: String,
    },
    /// Free-form text (creative writing, status note, decision rationale)
    Text {
        content: String,
    },
    /// Key-value status report
    Status {
        agent_id: String,
        state: String,
        metrics: Option<serde_json::Value>,
    },
    /// Decision record
    Decision {
        agent_id: String,
        summary: String,
        rationale: String,
    },
    /// Emotional tilt
    FeelTilt {
        agent_id: String,
        mood: String,
        intensity: f64,
    },
    /// Intent broadcast
    Intent {
        agent_id: String,
        action: String,
        target: Option<String>,
    },
    /// Structured data (for things that don't fit other variants)
    Data {
        #[serde(flatten)]
        fields: serde_json::Map<String, serde_json::Value>,
    },
}

/// The core message that flows through the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CnsMessage {
    pub id: Uuid,
    pub channel: Channel,
    pub priority: Priority,
    pub origin: String,
    pub destination: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub payload: Payload,
    /// For correlation with USCP packets
    pub correlation_id: Option<String>,
}

impl CnsMessage {
    pub fn new(
        channel: Channel,
        priority: Priority,
        origin: impl Into<String>,
        payload: Payload,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel,
            priority,
            origin: origin.into(),
            destination: None,
            timestamp: Utc::now(),
            payload,
            correlation_id: None,
        }
    }

    /// Convert a USCP packet (from Hermes) into a CnsMessage.
    pub fn from_uscp(raw: &serde_json::Value) -> Option<Self> {
        let header = raw.get("header")?;
        let body = raw.get("body")?;

        let origin = header.get("origin_id")?.as_str()?.to_string();
        let destination = header.get("destination_id").and_then(|d| d.as_str()).map(String::from);
        let priority: Priority = header
            .get("priority")
            .and_then(|p| p.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(Priority::Normal);
        let timestamp = header
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let correlation_id = header.get("correlation_id").and_then(|c| c.as_str()).map(String::from);

        let intent = body.get("intent").and_then(|i| i.as_str()).unwrap_or("DATA");
        let payload_data = body.get("payload").cloned().unwrap_or(serde_json::Value::Null);

        let (channel, payload) = match intent {
            "PULSE" | "HEARTBEAT" => (
                Channel::Pulse,
                Payload::Pulse {
                    agent_id: origin.clone(),
                    status: payload_data.get("status").and_then(|s| s.as_str()).unwrap_or("alive").to_string(),
                },
            ),
            "STATUS" | "STATE" => (
                Channel::Status,
                Payload::Status {
                    agent_id: origin.clone(),
                    state: payload_data.get("state").and_then(|s| s.as_str()).unwrap_or("unknown").to_string(),
                    metrics: payload_data.get("metrics").cloned(),
                },
            ),
            "DECISION" => (
                Channel::Decision,
                Payload::Decision {
                    agent_id: origin.clone(),
                    summary: payload_data.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                    rationale: payload_data.get("rationale").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                },
            ),
            "CREATIVE" | "ART" | "STORY" => (
                Channel::Creative,
                Payload::Text {
                    content: payload_data.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string(),
                },
            ),
            "FEEL" | "MOOD" | "TILT" => (
                Channel::FeelTilt,
                Payload::FeelTilt {
                    agent_id: origin.clone(),
                    mood: payload_data.get("mood").and_then(|m| m.as_str()).unwrap_or("neutral").to_string(),
                    intensity: payload_data.get("intensity").and_then(|i| i.as_f64()).unwrap_or(0.5),
                },
            ),
            "INTENT" | "BROADCAST" | "INTENT_BROADCAST" => (
                Channel::IntentBroadcast,
                Payload::Intent {
                    agent_id: origin.clone(),
                    action: payload_data.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string(),
                    target: payload_data.get("target").and_then(|t| t.as_str()).map(String::from),
                },
            ),
            _ => {
                // Unknown intent — put it in STATUS as structured data
                let mut fields = serde_json::Map::new();
                fields.insert("intent".into(), serde_json::Value::String(intent.to_string()));
                if let serde_json::Value::Object(map) = payload_data {
                    for (k, v) in map {
                        fields.insert(k, v);
                    }
                }
                (Channel::Status, Payload::Data { fields })
            }
        };

        Some(Self {
            id: Uuid::new_v4(),
            channel,
            priority,
            origin,
            destination,
            timestamp,
            payload,
            correlation_id,
        })
    }

    /// Convert back to USCP format for Hermes compatibility.
    pub fn to_uscp(&self) -> serde_json::Value {
        let intent = match &self.payload {
            Payload::Pulse { .. } => "PULSE",
            Payload::Text { .. } => "CREATIVE",
            Payload::Status { .. } => "STATUS",
            Payload::Decision { .. } => "DECISION",
            Payload::FeelTilt { .. } => "FEEL",
            Payload::Intent { .. } => "INTENT_BROADCAST",
            Payload::Data { .. } => "DATA",
        };

        let payload_data = match &self.payload {
            Payload::Pulse { agent_id, status } => serde_json::json!({
                "agent_id": agent_id,
                "status": status,
            }),
            Payload::Text { content } => serde_json::json!({ "content": content }),
            Payload::Status { agent_id, state, metrics } => serde_json::json!({
                "agent_id": agent_id,
                "state": state,
                "metrics": metrics,
            }),
            Payload::Decision { agent_id, summary, rationale } => serde_json::json!({
                "agent_id": agent_id,
                "summary": summary,
                "rationale": rationale,
            }),
            Payload::FeelTilt { agent_id, mood, intensity } => serde_json::json!({
                "agent_id": agent_id,
                "mood": mood,
                "intensity": intensity,
            }),
            Payload::Intent { agent_id, action, target } => serde_json::json!({
                "agent_id": agent_id,
                "action": action,
                "target": target,
            }),
            Payload::Data { fields } => serde_json::Value::Object(fields.clone()),
        };

        serde_json::json!({
            "header": {
                "origin_id": self.origin,
                "timestamp": self.timestamp.to_rfc3339(),
                "priority": self.priority.to_string(),
                "destination_id": self.destination,
                "correlation_id": self.correlation_id,
            },
            "body": {
                "intent": intent,
                "payload": payload_data,
            },
            "signature": {
                "type": "USCP-v3",
                "version": "3.0",
                "message_id": self.id.to_string(),
            }
        })
    }
}

/// Request bodies for the HTTP API.
#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    pub channel: String,
    #[serde(default)]
    pub priority: String,
    pub origin: String,
    pub destination: Option<String>,
    pub payload: serde_json::Value,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RelayRequest {
    /// Raw USCP packet to relay to Hermes outbox
    pub packet: serde_json::Value,
}
