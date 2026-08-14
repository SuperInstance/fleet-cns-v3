//! SQLite persistence layer — WAL mode, crash recovery, 7-day retention.
//! Messages are stored on publish and can be replayed by subscribers.

use crate::types::{Channel, CnsMessage, Priority};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug)]
pub struct StoreStats {
    pub total_messages: usize,
    pub oldest: Option<DateTime<Utc>>,
    pub newest: Option<DateTime<Utc>>,
    pub per_channel: Vec<(String, usize)>,
}

impl Store {
    /// Open (or create) the SQLite database with WAL mode and schema.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db parent dir: {}", parent.display()))?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("opening db: {}", db_path.display()))?;

        // Performance pragmas
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -8000;
             PRAGMA temp_store = MEMORY;
             PRAGMA mmap_size = 268435456;",
        )?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id          TEXT PRIMARY KEY,
                channel     TEXT NOT NULL,
                priority    INTEGER NOT NULL,
                origin      TEXT NOT NULL,
                destination TEXT,
                timestamp   TEXT NOT NULL,
                payload     TEXT NOT NULL,
                correlation_id TEXT,
                seq         INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_messages_channel
                ON messages(channel, timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_priority
                ON messages(priority DESC, timestamp);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp
                ON messages(timestamp);

            CREATE TABLE IF NOT EXISTS seq_counter (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                next_seq INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO seq_counter (id, next_seq) VALUES (1, 1);
            ",
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Persist a message and return its sequence number.
    pub async fn store(&self, msg: &CnsMessage) -> Result<i64> {
        let conn = self.conn.lock().await;
        let payload_json = serde_json::to_string(&msg.payload)?;
        let ts = msg.timestamp.to_rfc3339();

        let seq: i64 = conn
            .query_row(
                "UPDATE seq_counter SET next_seq = next_seq + 1 WHERE id = 1 RETURNING next_seq - 1",
                [],
                |row| row.get(0),
            )
            .context("getting sequence number")?;

        conn.execute(
            "INSERT INTO messages (id, channel, priority, origin, destination, timestamp, payload, correlation_id, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg.id.to_string(),
                msg.channel.as_str(),
                msg.priority as i64,
                msg.origin,
                msg.destination,
                ts,
                payload_json,
                msg.correlation_id,
                seq,
            ],
        )?;

        Ok(seq)
    }

    /// Replay the last `count` messages on a channel (oldest first).
    pub fn replay(&self, channel: &Channel, count: usize) -> Vec<CnsMessage> {
        let conn = self.conn.blocking_lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, channel, priority, origin, destination, timestamp, payload, correlation_id
                 FROM messages
                 WHERE channel = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .expect("prepare replay");

        let rows: Vec<CnsMessage> = stmt
            .query_map(params![channel.as_str(), count as i64], |row| {
                let id: String = row.get(0)?;
                let _channel: String = row.get(1)?;
                let priority_int: i64 = row.get(2)?;
                let origin: String = row.get(3)?;
                let destination: Option<String> = row.get(4)?;
                let ts: String = row.get(5)?;
                let payload_str: String = row.get(6)?;
                let correlation_id: Option<String> = row.get(7)?;

                let payload: crate::types::Payload =
                    serde_json::from_str(&payload_str).unwrap_or(crate::types::Payload::Text {
                        content: "(unparseable payload)".into(),
                    });

                let priority = match priority_int {
                    3 => Priority::Critical,
                    2 => Priority::High,
                    1 => Priority::Normal,
                    _ => Priority::Low,
                };

                Ok(CnsMessage {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                    channel: *channel,
                    priority,
                    origin,
                    destination,
                    timestamp: DateTime::parse_from_rfc3339(&ts)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    payload,
                    correlation_id,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        rows.into_iter().rev().collect()
    }

    /// Messages since a given timestamp on a channel.
    pub async fn since(&self, channel: &Channel, since: DateTime<Utc>) -> Vec<CnsMessage> {
        let conn = self.conn.lock().await;
        let mut stmt = match conn.prepare(
            "SELECT id, channel, priority, origin, destination, timestamp, payload, correlation_id
             FROM messages
             WHERE channel = ?1 AND timestamp > ?2
             ORDER BY timestamp ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let since_str = since.to_rfc3339();
        let ch_str = channel.as_str();

        let rows = stmt
            .query_map(params![ch_str, since_str], |row| {
                let id: String = row.get(0)?;
                let _channel: String = row.get(1)?;
                let priority_int: i64 = row.get(2)?;
                let origin: String = row.get(3)?;
                let destination: Option<String> = row.get(4)?;
                let ts: String = row.get(5)?;
                let payload_str: String = row.get(6)?;
                let correlation_id: Option<String> = row.get(7)?;

                let payload: crate::types::Payload =
                    serde_json::from_str(&payload_str).unwrap_or(crate::types::Payload::Text {
                        content: "(unparseable payload)".into(),
                    });

                let priority = match priority_int {
                    3 => Priority::Critical,
                    2 => Priority::High,
                    1 => Priority::Normal,
                    _ => Priority::Low,
                };

                Ok(CnsMessage {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                    channel: *channel,
                    priority,
                    origin,
                    destination,
                    timestamp: DateTime::parse_from_rfc3339(&ts)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    payload,
                    correlation_id,
                })
            });

        match rows {
            Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Delete messages older than the retention period. Returns count deleted.
    pub async fn cleanup_old(&self, before: DateTime<Utc>) -> Result<usize> {
        let conn = self.conn.lock().await;
        let before_str = before.to_rfc3339();
        let deleted = conn.execute(
            "DELETE FROM messages WHERE timestamp < ?1",
            params![before_str],
        )?;
        Ok(deleted)
    }

    pub fn stats(&self) -> StoreStats {
        let conn = self.conn.blocking_lock();
        let total: usize = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap_or(0);

        let oldest: Option<String> = conn
            .query_row("SELECT MIN(timestamp) FROM messages", [], |row| row.get(0))
            .ok()
            .flatten();
        let newest: Option<String> = conn
            .query_row("SELECT MAX(timestamp) FROM messages", [], |row| row.get(0))
            .ok()
            .flatten();

        let per_channel: Vec<(String, usize)> = conn
            .prepare("SELECT channel, COUNT(*) FROM messages GROUP BY channel ORDER BY COUNT(*) DESC")
            .and_then(|mut stmt| {
                let rows = stmt.query_map([], |row| {
                    let ch: String = row.get(0)?;
                    let count: usize = row.get(1)?;
                    Ok((ch, count))
                })?;
                Ok(rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        StoreStats {
            total_messages: total,
            oldest: oldest.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
            newest: newest.and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(|dt| dt.with_timezone(&Utc))),
            per_channel,
        }
    }
}
