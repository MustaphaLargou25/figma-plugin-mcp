# Rust Relay Enhancement

## Current Node Relay Limitations

The Bun/Node relay is already serviceable for local use, but it has scaling pressure points:

- WebSocket throughput is pinned to one JavaScript event loop. A large JSON message or burst of queue work can delay every channel.
- All state lives in mutable JS maps. That is simple, but it serializes routing, cleanup, queue inspection, and timeout work through one thread.
- Large payloads, especially base64 image responses, are parsed into JS objects and then stringified into wrapped relay messages. That creates temporary copies and GC pressure.
- Timeout handling uses per-command timers in the same event loop. Timer callbacks can be delayed by CPU-heavy JSON serialization or many synchronous map scans.
- Backpressure is mostly implicit. If a client is slow, `send()` can buffer more memory inside the runtime before the relay notices.
- Cleanup scans maps and queues synchronously. At many channels/sessions this increases tail latency for unrelated clients.

## Rust Relay Architecture

```text
MCP server / AI agent
  |
  | same WebSocket JSON protocol
  v
Rust relay (tokio + axum)
  |
  | bounded per-client writer channel
  | DashMap session registry
  | async per-channel queue mutex
  v
Figma plugin UI iframe
  |
  v
Figma plugin main thread
```

The Rust implementation lives in `rust-relay/`:

```text
rust-relay/
  Cargo.toml
  src/
    main.rs       runtime startup, tracing, graceful shutdown
    config.rs     environment configuration
    protocol.rs   compatible JSON envelope helpers
    state.rs      session registry, queues, timeouts, routing
    web.rs        axum HTTP and WebSocket handlers
```

### State Model

- `DashMap` stores clients, channels, sessions, pending requests, and role sets.
- Each client has a bounded `tokio::sync::mpsc` writer queue. Slow clients create backpressure instead of unbounded memory growth.
- Each Figma channel has one `tokio::sync::Mutex<ChannelQueue>`. Only commands within the same channel serialize; unrelated channels continue concurrently.
- Command timeouts are `tokio` tasks stored on the active channel queue and aborted/reset when progress arrives.
- Progress updates are routed using the raw original text frame after one parse for routing, keeping streaming latency low.

## Protocol Compatibility

The Rust relay keeps the same protocol described in `contracts/figma-mcp-bridge.schema.json` and also accepts omitted `channel` values by falling back to `figma-auto`. The Figma plugin and patched MCP CLI server now use that shared channel automatically:

- `join`
- `message` with `message.command`
- `message` with `message.result` or `message.error`
- `progress_update`
- `queue_position`
- `system`
- `error`

## Health Endpoints

`GET /healthz`

```json
{
  "status": "ok",
  "uptimeMs": 1234,
  "pluginCount": 1,
  "agentCount": 1
}
```

`GET /status`

Returns counters, active plugin/agent counts, queue depth per channel, current request age, and oldest queued age.

## Performance Improvements

- Higher concurrency: tokio runs WebSocket I/O, timers, and queue work across the async runtime instead of one JS event loop.
- Lower tail latency: channel-local queue locks mean one busy Figma file does not block other channels.
- Bounded memory: per-client writer buffers cap slow-client memory growth.
- Better timeout accuracy: timeout tasks are scheduled by tokio and reset/aborted on progress.
- Less serialization churn for progress: progress frames are parsed once and forwarded as raw text to the target client.
- Cheaper status inspection: counters are atomics and channel state is read without stopping unrelated routing work.

## Run

From the repo root:

```powershell
cd rust-relay
cargo run --release
```

Or use:

```powershell
.\start-rust-relay.bat
```

Default bind:

```text
127.0.0.1:3055
```

## Replace the Node Relay

1. Stop any running `bun.exe` relay on port `3055`.
2. Start `start-rust-relay.bat`.
3. Run the Figma plugin and click Connect.
4. Keep using the same MCP server. No `join_channel` call or copied channel ID is needed for the default workflow.

No `manifest.json`, `ui.html`, `code.js`, or MCP server tool changes are required.

## Configuration

```text
FIGMA_MCP_BIND=127.0.0.1:3055
FIGMA_MCP_SOCKET_PORT=3055
FIGMA_MCP_DEFAULT_CHANNEL=figma-auto
FIGMA_MCP_AUTO_CHANNEL=figma-auto
FIGMA_MCP_COMMAND_TIMEOUT_MS=120000
FIGMA_MCP_MAX_QUEUE_SIZE=100
FIGMA_MCP_MAX_MESSAGE_BYTES=26214400
FIGMA_MCP_CLIENT_BUFFER_SIZE=256
RUST_LOG=info
```

Use `FIGMA_MCP_BIND=0.0.0.0:3055` only if you explicitly need remote access; local loopback is safer for the default Figma/MCP workflow.
