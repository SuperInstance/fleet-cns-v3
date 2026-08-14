//! CNS v3 — Inter-agent communication bus
//!
//! Typed message bus replacing the JSON-file CNS.
//! Channels: PULSE, STATUS, CREATIVE, DECISION, FEEL_TILT, INTENT_BROADCAST
//! Priority queuing, SQLite persistence, SSE streaming, backwards-compatible
//! USCP/JSONL spooling for Hermes.

mod types;
mod store;
mod bus;
mod compat;
mod api;

use std::path::PathBuf;
use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser)]
#[command(name = "fleet-cns-v3", version, about = "Inter-agent communication bus")]
struct Cli {
    /// Bind address for the HTTP API
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,

    /// Port for the HTTP API
    #[arg(long, default_value = "9920")]
    port: u16,

    /// SQLite database path
    #[arg(long)]
    db: Option<PathBuf>,

    /// Hermes spool directory (parent of cns_inbox/cns_outbox)
    #[arg(long)]
    hermes_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the bus server (default)
    Serve,
    /// Show bus status
    Status,
    /// Replay messages from the database
    Replay {
        /// Channel to replay
        channel: String,
        /// Number of messages to replay
        #[arg(short, long, default_value = "10")]
        count: usize,
    },
}

fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,fleet_cns_v3=debug"))
        )
        .with_target(true)
        .compact()
        .init();

    let cli = Cli::parse();

    let db_path = cli.db.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".hermes")
            .join("cns_v3.db")
    });

    let hermes_dir = cli.hermes_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".hermes")
    });

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match cli.command.unwrap_or(Commands::Serve) {
            Commands::Serve => {
                api::run_server(&cli.bind, cli.port, &db_path, &hermes_dir).await
            }
            Commands::Status => {
                let store = store::Store::open(&db_path).expect("failed to open db");
                let stats = store.stats();
                println!("CNS v3 Status");
                println!("  Total messages:    {}", stats.total_messages);
                println!("  Oldest:            {}", stats.oldest.map(|t| t.to_rfc3339()).unwrap_or_else(|| "none".into()));
                println!("  Newest:            {}", stats.newest.map(|t| t.to_rfc3339()).unwrap_or_else(|| "none".into()));
                println!("  DB path:           {}", db_path.display());
                println!("\nMessages per channel:");
                for (ch, count) in &stats.per_channel {
                    println!("  {:20} {}", ch, count);
                }
                Ok(())
            }
            Commands::Replay { channel, count } => {
                let store = store::Store::open(&db_path).expect("failed to open db");
                let ch: types::Channel = channel.parse()
                    .unwrap_or_else(|_| {
                        eprintln!("Unknown channel: {channel}");
                        std::process::exit(1);
                    });
                let msgs = store.replay(&ch, count);
                println!("Replaying {} messages from {}:", msgs.len(), ch);
                for msg in msgs {
                    println!("\n--- {} ---", msg.id);
                    println!("  Priority: {}", msg.priority);
                    println!("  Origin:   {}", msg.origin);
                    println!("  Time:     {}", msg.timestamp.to_rfc3339());
                    println!("  Payload:  {}", serde_json::to_string_pretty(&msg.payload).unwrap_or_default());
                }
                Ok(())
            }
        }
    })
}
