//! HTTP API — Axum endpoints for the CNS v3 bus.
//!
//! - POST /publish — publish a message
//! - GET /subscribe/:channel — SSE stream
//! - GET /channels — list channels and subscriber counts
//! - GET /health — bus health, message rates, queue depths
//! - POST /relay — relay to Hermes inbox (backwards compat)

use crate::bus::{Bus, ChannelInfo};
use crate::compat::Compat;
use crate::store::Store;
use crate::types::{Channel, CnsMessage, Payload, Priority, PublishRequest, RelayRequest};
use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Json, sse::{Event, KeepAlive, Sse}},
    routing::{get, post},
    Router,
};
use futures::Stream;
use tokio::sync::broadcast;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

#[derive(Clone)]
pub struct AppState {
    pub bus: Arc<Bus>,
    pub store: Arc<Store>,
    pub compat: Arc<Compat>,
}

pub async fn run_server(
    bind: &str,
    port: u16,
    db_path: &PathBuf,
    hermes_dir: &PathBuf,
) -> Result<()> {
    let store = Arc::new(Store::open(db_path)?);
    let bus = Bus::new();
    let compat = Arc::new(Compat::new(bus.clone(), store.clone(), hermes_dir));

    // Initial scan of existing outbox files
    compat.scan_outbox().await;

    // Start filesystem watcher for Hermes outbox
    compat.clone().spawn_watcher();

    // Start retention cleanup (7 days)
    Compat::spawn_cleanup(store.clone(), 7);

    let state = AppState {
        bus,
        store,
        compat,
    };

    let app = Router::new()
        .route("/publish", post(publish))
        .route("/subscribe/:channel", get(subscribe))
        .route("/channels", get(channels))
        .route("/health", get(health))
        .route("/relay", post(relay))
        .route("/", get(root))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{bind}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(addr = %addr, "CNS v3 bus listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received");
}

// --- Handlers ---

async fn root() -> &'static str {
    "CNS v3 — Inter-agent communication bus\n\nEndpoints:\n  POST /publish\n  GET  /subscribe/:channel\n  GET  /channels\n  GET  /health\n  POST /relay\n"
}

async fn publish(
    State(state): State<AppState>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let channel: Channel = req.channel.parse().map_err(|e: _| {
        (StatusCode::BAD_REQUEST, format!("unknown channel: {e}"))
    })?;

    let priority: Priority = if req.priority.is_empty() {
        Priority::Normal
    } else {
        req.priority.parse().map_err(|e: _| {
            (StatusCode::BAD_REQUEST, format!("unknown priority: {e}"))
        })?
    };

    let payload = parse_payload_from_json(&channel, &req.origin, &req.payload);

    let mut msg = CnsMessage::new(channel, priority, req.origin, payload);
    msg.destination = req.destination;
    msg.correlation_id = req.correlation_id;

    // Store to SQLite
    if let Err(e) = state.store.store(&msg).await {
        error!(error = %e, "failed to store message");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("store error: {e}")));
    }

    // Write to Hermes inbox for backwards compat
    if let Err(e) = state.compat.write_to_inbox(&msg).await {
        error!(error = %e, "failed to write to Hermes inbox");
        // Non-fatal — the message is already in SQLite
    }

    // Publish to in-memory bus
    let delivered = state.bus.publish(Arc::new(msg.clone()));

    Ok(Json(serde_json::json!({
        "status": "published",
        "id": msg.id,
        "channel": msg.channel.as_str(),
        "priority": msg.priority.to_string(),
        "delivered_to": delivered,
    })))
}

async fn subscribe(
    State(state): State<AppState>,
    Path(channel_str): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let channel: Channel = channel_str.parse().map_err(|e: _| {
        (StatusCode::BAD_REQUEST, format!("unknown channel: {e}"))
    })?;

    let mut rx = state.bus.subscribe(channel);
    let ch = channel;

    let stream = async_stream::stream! {
        // Send a welcome event with replay of last 5 messages
        let history = state.store.replay(&ch, 5);
        for msg in &history {
            let json = serde_json::to_string(&msg).unwrap_or_default();
            yield Ok(Event::default().event("history").data(json));
        }

        yield Ok(Event::default()
            .event("ready")
            .data(format!("{{\"channel\":\"{}\",\"replayed\":{}}}", ch, history.len())));

        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let json = serde_json::to_string(&*msg).unwrap_or_default();
                    yield Ok(Event::default().event("message").data(json));
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    yield Ok(Event::default()
                        .event("lagged")
                        .data(format!("{{\"missed\":{n}}}")));
                }
                Err(broadcast::error::RecvError::Closed) => {
                    yield Ok(Event::default().event("closed").data("{}"));
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

async fn channels(State(state): State<AppState>) -> Json<Vec<ChannelInfo>> {
    Json(state.bus.channel_info())
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let stats = state.store.stats();
    let uptime = state.bus.uptime_secs();
    let total = state.bus.total_published();
    let rate = if uptime > 0.0 { total as f64 / uptime } else { 0.0 };

    Json(serde_json::json!({
        "status": "healthy",
        "uptime_secs": uptime,
        "total_messages_published": total,
        "messages_per_second": rate,
        "total_in_db": stats.total_messages,
        "oldest_in_db": stats.oldest.map(|t| t.to_rfc3339()),
        "newest_in_db": stats.newest.map(|t| t.to_rfc3339()),
        "channels": state.bus.channel_info(),
        "retention_days": 7,
    }))
}

async fn relay(
    State(state): State<AppState>,
    Json(req): Json<RelayRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Convert USCP packet to CnsMessage, publish, and write to inbox
    let msg = CnsMessage::from_uscp(&req.packet).ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "could not parse USCP packet".to_string())
    })?;

    // Store and publish
    if let Err(e) = state.store.store(&msg).await {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("store error: {e}")));
    }

    state.compat.write_to_inbox(&msg).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("inbox write error: {e}"))
    })?;

    let delivered = state.bus.publish(Arc::new(msg.clone()));

    Ok(Json(serde_json::json!({
        "status": "relayed",
        "id": msg.id,
        "channel": msg.channel.as_str(),
        "delivered_to": delivered,
    })))
}

/// Parse an arbitrary JSON payload into a typed Payload based on the channel.
fn parse_payload_from_json(channel: &Channel, origin: &str, raw: &serde_json::Value) -> Payload {
    match channel {
        Channel::Pulse => Payload::Pulse {
            agent_id: raw.get("agent_id").and_then(|v| v.as_str()).unwrap_or(origin).to_string(),
            status: raw.get("status").and_then(|v| v.as_str()).unwrap_or("alive").to_string(),
        },
        Channel::Status => Payload::Status {
            agent_id: raw.get("agent_id").and_then(|v| v.as_str()).unwrap_or(origin).to_string(),
            state: raw.get("state").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            metrics: raw.get("metrics").cloned(),
        },
        Channel::Creative => {
            if let Some(content) = raw.get("content").and_then(|v| v.as_str()) {
                Payload::Text { content: content.to_string() }
            } else {
                Payload::Text { content: raw.to_string() }
            }
        }
        Channel::Decision => Payload::Decision {
            agent_id: raw.get("agent_id").and_then(|v| v.as_str()).unwrap_or(origin).to_string(),
            summary: raw.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            rationale: raw.get("rationale").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        },
        Channel::FeelTilt => Payload::FeelTilt {
            agent_id: raw.get("agent_id").and_then(|v| v.as_str()).unwrap_or(origin).to_string(),
            mood: raw.get("mood").and_then(|v| v.as_str()).unwrap_or("neutral").to_string(),
            intensity: raw.get("intensity").and_then(|v| v.as_f64()).unwrap_or(0.5),
        },
        Channel::IntentBroadcast => Payload::Intent {
            agent_id: raw.get("agent_id").and_then(|v| v.as_str()).unwrap_or(origin).to_string(),
            action: raw.get("action").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            target: raw.get("target").and_then(|v| v.as_str()).map(String::from),
        },
    }
}
