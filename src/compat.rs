//! Backwards compatibility layer — USCP packet reading and JSONL spool writing.
//!
//! - Watches `~/.hermes/cns_outbox/` for new USCP JSON files from Hermes,
//!   converts them to typed messages, and publishes to the bus.
//! - Writes JSONL spool files to `~/.hermes/cns_inbox/` for Hermes to consume.
//! - The old Python monitor can run alongside without conflict.

use crate::bus::Bus;
use crate::store::Store;
use crate::types::CnsMessage;
use anyhow::{Context, Result};
use chrono::Utc;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Compat manager — reads USCP from Hermes outbox, writes JSONL to Hermes inbox.
pub struct Compat {
    bus: Arc<Bus>,
    store: Arc<Store>,
    inbox_dir: PathBuf,
    outbox_dir: PathBuf,
}

impl Compat {
    pub fn new(bus: Arc<Bus>, store: Arc<Store>, hermes_dir: &Path) -> Self {
        let inbox_dir = hermes_dir.join("cns_inbox");
        let outbox_dir = hermes_dir.join("cns_outbox");

        // Ensure directories exist
        let _ = std::fs::create_dir_all(&inbox_dir);
        let _ = std::fs::create_dir_all(&outbox_dir);

        Self {
            bus,
            store,
            inbox_dir,
            outbox_dir,
        }
    }

    /// Write a message as a JSONL file to the Hermes inbox.
    /// This lets the old Python CNS pick it up.
    pub async fn write_to_inbox(&self, msg: &CnsMessage) -> Result<PathBuf> {
        let uscp = msg.to_uscp();
        let ts = Utc::now().format("%Y%m%dT%H%M%S%3f");
        let filename = format!("cns_v3_{}_{}_{}.json", msg.origin, ts, &msg.id.to_string()[..8]);
        let path = self.inbox_dir.join(filename);

        let content = serde_json::to_string_pretty(&uscp)?;
        tokio::fs::write(&path, content).await
            .with_context(|| format!("writing to inbox: {}", path.display()))?;

        debug!(path = %path.display(), "wrote message to Hermes inbox");
        Ok(path)
    }

    /// Read a USCP packet file from the Hermes outbox, convert to CnsMessage,
    /// publish to bus and store.
    pub async fn process_outbox_file(&self, path: &Path) -> Result<()> {
        let content = tokio::fs::read_to_string(path).await
            .with_context(|| format!("reading outbox file: {}", path.display()))?;

        let raw: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("parsing JSON from: {}", path.display()))?;

        if let Some(msg) = CnsMessage::from_uscp(&raw) {
            debug!(channel = %msg.channel, origin = %msg.origin, "imported USCP message");

            let msg_arc = Arc::new(msg);
            self.store.store(&msg_arc).await.ok();
            self.bus.publish(msg_arc);

            // Archive the processed file
            let archive_dir = self.outbox_dir.join("processed");
            let _ = std::fs::create_dir_all(&archive_dir);
            let dest = archive_dir.join(path.file_name().unwrap_or_default());
            if let Err(e) = tokio::fs::rename(path, &dest).await {
                // If rename fails (cross-device), try copy+delete
                let _ = tokio::fs::copy(path, &dest).await;
                let _ = tokio::fs::remove_file(path).await;
                warn!(error = %e, "rename failed, used copy+delete instead");
            }
        } else {
            warn!(path = %path.display(), "could not convert USCP packet to CnsMessage");
        }

        Ok(())
    }

    /// Scan the outbox for existing files (on startup).
    pub async fn scan_outbox(&self) {
        let mut entries = match tokio::fs::read_dir(&self.outbox_dir).await {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut count = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Err(e) = self.process_outbox_file(&path).await {
                    warn!(path = %path.display(), error = %e, "failed to process outbox file during scan");
                } else {
                    count += 1;
                }
            }
        }

        if count > 0 {
            info!(count, "imported existing USCP packets from outbox");
        }
    }

    /// Start a background task watching the Hermes outbox for new files.
    pub fn spawn_watcher(self: Arc<Self>) {
        let outbox = self.outbox_dir.clone();
        let compat = self.clone();

        tokio::spawn(async move {
            let (tx, mut rx) = mpsc::channel::<Vec<PathBuf>>(64);

            // Create the watcher
            let mut watcher = match RecommendedWatcher::new(
                move |res: notify::Result<notify::Event>| {
                    if let Ok(event) = res {
                        let paths: Vec<PathBuf> = event
                            .paths
                            .into_iter()
                            .filter(|p| p.extension().is_some_and(|e| e == "json"))
                            .collect();
                        if !paths.is_empty() {
                            let _ = tx.blocking_send(paths);
                        }
                    }
                },
                notify::Config::default(),
            ) {
                Ok(w) => w,
                Err(e) => {
                    error!(error = %e, "failed to create filesystem watcher for Hermes outbox");
                    return;
                }
            };

            // Watch the outbox directory
            if let Err(e) = watcher.watch(&outbox, RecursiveMode::NonRecursive) {
                error!(error = %e, dir = %outbox.display(), "failed to watch Hermes outbox");
                return;
            }

            info!(dir = %outbox.display(), "watching Hermes outbox for USCP packets");

            // Keep the watcher alive
            loop {
                match rx.recv().await {
                    Some(paths) => {
                        for path in paths {
                            // Small delay to let the file be fully written
                            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                            if let Err(e) = compat.process_outbox_file(&path).await {
                                warn!(path = %path.display(), error = %e, "failed to process watched file");
                            }
                        }
                    }
                    None => {
                        info!("Hermes outbox watcher channel closed, exiting");
                        break;
                    }
                }
            }

            // Keep watcher alive
            drop(watcher);
        });
    }

    /// Start periodic cleanup of old messages.
    pub fn spawn_cleanup(store: Arc<Store>, retention_days: i64) {
        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_secs(3600); // Run hourly
            loop {
                tokio::time::sleep(interval).await;
                let cutoff = Utc::now() - chrono::Duration::days(retention_days);
                match store.cleanup_old(cutoff).await {
                    Ok(deleted) if deleted > 0 => {
                        info!(deleted, retention_days, "cleaned up old messages");
                    }
                    Ok(_) => {
                        debug!("cleanup ran, nothing to delete");
                    }
                    Err(e) => {
                        error!(error = %e, "failed to cleanup old messages");
                    }
                }
            }
        });
    }
}
