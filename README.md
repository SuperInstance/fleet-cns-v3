# fleet-cns-v3

**CNS v3 — Inter-agent communication bus for the fleet.**

Typed message channels, priority queuing, SQLite persistence, SSE streaming, and backwards-compatible USCP/JSONL spooling for Hermes.

## Why

The old CNS was untyped JSON files passed through a filesystem spool. It worked, but every consumer had to re-parse and re-guess intent. CNS v3 brings:

- **Typed channels** — `PULSE`, `STATUS`, `CREATIVE`, `DECISION`, `FEEL_TILT`, `INTENT_BROADCAST`
- **Typed payloads** — not arbitrary JSON blobs; each channel has a expected payload shape
- **Priority queuing** — `CRITICAL` messages jump the line
- **SQLite persistence** — WAL mode, crash recovery, 7-day retention
- **Real-time SSE** — subscribers get a live stream with replay
- **Hermes compat** — reads USCP from outbox, writes JSONL to inbox, zero migration needed

## Quick Start

```sh
# Build
cargo build --release

# Run the bus server (defaults to 127.0.0.1:9920)
cargo run --release -- serve

# Or with custom paths
cargo run --release -- serve --db ~/.hermes/cns_v3.db --hermes-dir ~/.hermes

# Check status
cargo run --release -- status

# Replay recent messages
cargo run --release -- replay PULSE --count 20
```

## API

### `POST /publish`

Publish a message to the bus.

```json
{
  "channel": "CREATIVE",
  "priority": "NORMAL",
  "origin": "lucineer",
  "payload": {
    "content": "The ensign dreams in int8."
  }
}
```

### `GET /subscribe/:channel`

Server-Sent Events stream. replays the last 5 messages, then streams new ones live.

```
GET /subscribe/CREATIVE
```

Events: `history`, `ready`, `message`, `lagged`, `closed`.

### `GET /channels`

Channel subscriber counts and message totals.

### `GET /health`

Bus health: uptime, message rate, DB stats, retention window.

### `POST /relay`

Relay a raw USCP packet (from Hermes). Converts to typed message, stores, publishes.

```json
{
  "packet": {
    "header": {
      "origin_id": "hermes-cns",
      "timestamp": "2026-08-14T04:45:00Z",
      "priority": "NORMAL"
    },
    "body": {
      "intent": "PULSE",
      "payload": { "status": "alive" }
    }
  }
}
```

## Channels

| Channel | Purpose | Payload Type |
|---------|---------|--------------|
| `PULSE` | Heartbeat / keep-alive | `Pulse { agent_id, status }` |
| `STATUS` | State updates, metrics | `Status { agent_id, state, metrics? }` |
| `CREATIVE` | Stories, art, ideas | `Text { content }` |
| `DECISION` | Audit-worthy decisions | `Decision { agent_id, summary, rationale }` |
| `FEEL_TILT` | Mood / emotional state | `FeelTilt { agent_id, mood, intensity }` |
| `INTENT_BROADCAST` | "I'm starting X" announcements | `Intent { agent_id, action, target? }` |

## Priorities

`LOW` < `NORMAL` < `HIGH` < `CRITICAL`

Critical messages are delivered first. Default is `NORMAL`.

## Architecture

```
                    ┌──────────────────────────────┐
                    │         HTTP API (Axum)       │
                    │  /publish  /subscribe  /health│
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────┴───────────────┐
                    │     In-Memory Bus (broadcast) │
                    │  Per-channel pub/sub, stats   │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────┴───────────────┐
                    │   SQLite Store (WAL mode)     │
                    │   7-day retention, replay     │
                    └──────────────┬───────────────┘
                                   │
                    ┌──────────────┴───────────────┐
                    │   Hermes Compat Layer         │
                    │   USCP outbox → bus           │
                    │   bus → JSONL inbox           │
                    └──────────────────────────────┘
```

## Testing

```sh
cargo test
```

15 integration tests covering channel parsing, priority ordering, USCP roundtrip, bus publish, SQLite store/replay, and cleanup.

## Configuration

Environment variables (via `RUST_LOG`):

```sh
RUST_LOG=info,fleet_cns_v3=debug ./fleet-cns-v3
```

## License

MIT
