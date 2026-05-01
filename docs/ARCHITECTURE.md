# Figma MCP Bridge Architecture

## Current Shape

```text
AI model / MCP client
  |
  | stdio MCP tools (claude-talk-to-figma-mcp-server)
  v
MCP server process
  |
  | ws://localhost:3055, channel-scoped JSON messages
  v
Local relay: server/socket.js
  |
  | serialized command queue per channel
  v
Figma plugin UI iframe: ui.html
  |
  | parent.postMessage({ pluginMessage })
  v
Figma plugin main thread: code.js
  |
  | Figma Plugin API
  v
Open Figma document
```

The MCP server exposes tools to the AI model. The relay does not call Figma directly; it routes commands to the plugin and waits for a matching response ID. The plugin main thread is the only layer that touches the Figma document.

## Issues Found

- The workspace contained the Figma plugin bundle and a batch file pointing to a globally installed relay, so the critical WebSocket bridge was not versioned with the project.
- Long operations emitted progress, but the installed relay could still time out the active queue item while the plugin continued mutating the document.
- Progress events duplicated large arrays such as `textNodes`, `results`, and `chunkResults`, increasing WebSocket I/O and memory pressure.
- `get_nodes_info` exported every requested node in unbounded parallelism, which can spike CPU/memory on large selections.
- Image export accepted arbitrary scale values and encoded base64 through one growing string, a common high-allocation path for large raster exports.
- The UI WebSocket client lacked stable session IDs, connect timeout handling, reconnect backoff, and centralized send/error handling.

## Changes Made

- Added a local relay at `server/socket.js` and pointed `start-socket.bat` to it.
- Added `/healthz` and `/status` endpoints for monitoring.
- Added per-channel command queues with timeout reset on progress updates.
- Added default `figma-auto` channel support so the plugin and MCP CLI server can connect without copying a channel ID.
- Added reconnect/session handling in `ui.html` so relay restarts do not leave stale clients behind.
- Added plugin-side command metrics and a `health_check` command in `code.js`.
- Serialized mutating plugin commands while allowing read-only commands to run immediately.
- Compacted progress payloads while preserving full final command results.
- Bounded batch export concurrency for `get_nodes_info`.
- Added scale, format, and pixel-count guards for image export.
- Reworked base64 encoding to flush bounded string segments instead of growing one large string.
- Added the bridge protocol schema at `contracts/figma-mcp-bridge.schema.json`.

## Relay Endpoints

`GET http://localhost:3055/healthz`

Returns lightweight liveness:

```json
{
  "status": "ok",
  "uptimeMs": 12345,
  "pluginCount": 1,
  "agentCount": 1
}
```

`GET http://localhost:3055/status`

Returns queue depth, active channels, pending request count, client counts, and cumulative counters.

## WebSocket Contract

Join a channel. This is now automatic for the default workflow; both plugin and MCP CLI server use `figma-auto` unless overridden:

```json
{
  "type": "join",
  "channel": "figma-auto",
  "sessionId": "figma_7s9k2..."
}
```

Send a command from MCP server to Figma:

```json
{
  "id": "request-1",
  "type": "message",
  "channel": "figma-auto",
  "message": {
    "id": "request-1",
    "command": "create_frame",
    "params": {
      "parentId": "0:1",
      "x": 0,
      "y": 0,
      "width": 1440,
      "height": 900
    }
  }
}
```

Return a plugin result:

```json
{
  "id": "request-1",
  "type": "message",
  "channel": "figma-auto",
  "message": {
    "id": "request-1",
    "result": {
      "id": "12:34",
      "name": "Frame"
    }
  }
}
```

Stream progress:

```json
{
  "id": "request-1",
  "type": "progress_update",
  "channel": "figma-auto",
  "message": {
    "id": "request-1",
    "type": "progress_update",
    "data": {
      "status": "in_progress",
      "progress": 45,
      "message": "Processing chunk 3/8"
    }
  }
}
```

## Performance Notes

- Figma document mutation should remain single-writer. Even if multiple MCP clients connect, the relay and plugin queue keep state-changing commands in order.
- Bulk reads should be capped. The plugin currently uses `MAX_BATCH_EXPORT_CONCURRENCY = 4` for JSON exports.
- Avoid progress payloads that mirror final payloads. Progress should be status-oriented; final results carry the full data.
- Raster export size should be bounded before `exportAsync`, because base64 expands payloads by about one third and then JSON stringification adds another copy.

## Rust Layer

The best Rust target is the relay, not the Figma plugin. A Rust relay can provide lower and more predictable memory use for queues, JSON validation, rate limiting, and metrics while leaving Figma API access in JavaScript. A drop-in implementation now lives in `rust-relay/`; see `docs/RUST_RELAY.md` for run and integration details.

Recommended shape:

```text
MCP server (Node/Bun stdio)
  |
  | WebSocket or gRPC
  v
Rust relay (tokio + axum)
  |
  | WebSocket
  v
Figma plugin
```

Minimal Rust module sketch:

```rust
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, VecDeque}, time::Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeCommand {
    pub id: Uuid,
    pub channel: String,
    pub command: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug)]
pub struct QueuedCommand {
    pub command: BridgeCommand,
    pub enqueued_at: Instant,
}

#[derive(Default)]
pub struct ChannelQueue {
    active: Option<Uuid>,
    pending: VecDeque<QueuedCommand>,
}

#[derive(Default)]
pub struct RelayState {
    queues: HashMap<String, ChannelQueue>,
}

impl RelayState {
    pub fn enqueue(&mut self, command: BridgeCommand, max_depth: usize) -> Result<usize, String> {
        let queue = self.queues.entry(command.channel.clone()).or_default();
        if queue.pending.len() >= max_depth {
            return Err("queue is full".to_string());
        }
        queue.pending.push_back(QueuedCommand {
            command,
            enqueued_at: Instant::now(),
        });
        Ok(queue.pending.len())
    }

    pub fn next_for_channel(&mut self, channel: &str) -> Option<BridgeCommand> {
        let queue = self.queues.get_mut(channel)?;
        if queue.active.is_some() {
            return None;
        }
        let next = queue.pending.pop_front()?.command;
        queue.active = Some(next.id);
        Some(next)
    }

    pub fn finish(&mut self, channel: &str, request_id: Uuid) {
        if let Some(queue) = self.queues.get_mut(channel) {
            if queue.active == Some(request_id) {
                queue.active = None;
            }
        }
    }
}
```

Use Rust once relay traffic grows beyond local single-user workflows, or when multiple AI agents share one Figma file and you need stronger fairness, rate limits, and structured observability.
