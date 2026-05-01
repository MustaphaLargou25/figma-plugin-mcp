# Figma MCP Rust Relay

Drop-in Rust replacement for `server/socket.js`.

## Run

```powershell
cd rust-relay
cargo run --release
```

Or double-click `start-rust-relay.bat` from the repository root.

The relay listens on `127.0.0.1:3055` by default and keeps the existing WebSocket protocol unchanged. It uses `figma-auto` as the default channel, so the patched MCP CLI server and current Figma plugin can connect without a copied channel ID.

## Endpoints

```text
GET /healthz
GET /status
GET /          WebSocket upgrade
GET /*         WebSocket upgrade
```

## Environment

```text
FIGMA_MCP_BIND=127.0.0.1:3055
FIGMA_MCP_BIND_HOST=127.0.0.1
FIGMA_MCP_SOCKET_PORT=3055
FIGMA_MCP_DEFAULT_CHANNEL=figma-auto
FIGMA_MCP_COMMAND_TIMEOUT_MS=120000
FIGMA_MCP_MAX_QUEUE_SIZE=100
FIGMA_MCP_MAX_MESSAGE_BYTES=26214400
FIGMA_MCP_CLIENT_BUFFER_SIZE=256
RUST_LOG=info
```

`FIGMA_MCP_BIND` wins over `FIGMA_MCP_BIND_HOST` and `FIGMA_MCP_SOCKET_PORT`.

## Compatibility

The Rust relay preserves these envelope shapes:

- `join`
- `message` with `message.command`
- `message` with `message.result` or `message.error`
- `progress_update`
- `queue_position`
- `system`
- `error`

The canonical schema remains `../contracts/figma-mcp-bridge.schema.json`.
