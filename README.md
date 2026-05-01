# Figma MCP Plugin — Write-Capable Figma Integration

A **write-capable Model Context Protocol (MCP) server** for Figma that enables Claude and other AI agents to programmatically create, modify, and manage Figma designs in real-time. This plugin bridges local Figma Desktop with Claude Code, allowing seamless AI-powered design workflows.

## Features

✨ **Real-time Collaboration**
- Local WebSocket relay for direct Figma plugin communication
- Auto-join channels—no manual channel ID management
- Bidirectional message routing between Claude and Figma

🛠️ **Write Operations**
- Create and manage frames, text, components
- Set fill colors, stroke properties, and typography
- Organize layers and modify design properties dynamically
- Execute complex design workflows with AI guidance

🚀 **Dual Relay Options**
- **JavaScript Relay** (Node.js/Bun) — Fast startup, easy customization
- **Rust Relay** — High-performance drop-in replacement with minimal overhead

🔐 **Security First**
- `.gitignore` configured to exclude secrets and build artifacts
- Figma tokens stored in local user config (not committed)
- Local-only communication; no cloud intermediary required

## Prerequisites

- **Figma Desktop** (required; web version doesn't support local plugins)
- **Bun 1.3.13+** or **Node.js 18+** (for JavaScript relay)
- **Rust 1.70+** (optional; for high-performance Rust relay)
- **Claude Code** with MCP Server support

## Quick Start

### 1. Clone and Setup

```bash
git clone https://github.com/MustaphaLargou25/figma-plugin-mcp.git
cd figma-plugin-mcp
```

### 2. Start the WebSocket Relay

**Option A: JavaScript Relay (Bun)**
```bash
./start-socket.bat
# or manually:
bun server/socket.js
```

**Option B: Rust Relay (High-Performance)**
```bash
./start-rust-relay.bat
# or manually:
cd rust-relay && cargo run --release
```

The relay will start on `http://localhost:3055` with health checks available:
- `GET http://localhost:3055/healthz` — Health check
- `GET http://localhost:3055/status` — Server status

### 3. Load Plugin in Figma Desktop

1. Open **Figma Desktop** and your design file
2. Navigate to **Plugins → Development → Import plugin from manifest…**
3. Select `manifest.json` from this repository
4. Run: **Plugins → Development → Claude Talk to Figma Plugin**
5. A small panel opens automatically connecting to the relay

### 4. Configure Claude Code

The MCP server entry is added to your `.claude.json` config file automatically. Restart Claude Code to pick up the connection:

```bash
# Verify MCP connection
/mcp  # in Claude Code chat
```

You should see `figma-write` and `figma` listed as connected servers.

## Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────┐
│            Claude Code (AI Agent)                  │
└──────────────────┬──────────────────────────────────┘
                   │ MCP Protocol
                   ▼
┌─────────────────────────────────────────────────────┐
│    MCP Server (node_modules/claude-talk-to-*)      │
└──────────────────┬──────────────────────────────────┘
                   │ WebSocket (localhost:3055)
      ┌────────────┴─────────────┐
      ▼                          ▼
  ┌──────────────┐         ┌──────────────┐
  │  JS Relay    │         │ Rust Relay   │
  │  socket.js   │         │ web.rs       │
  └──────────────┘         └──────────────┘
      │                          │
      └────────────┬─────────────┘
                   │ WebSocket (ws://localhost:3055)
                   ▼
┌─────────────────────────────────────────────────────┐
│          Figma Desktop Plugin                       │
│  (code.js + ui.html)                              │
└─────────────────────────────────────────────────────┘
                   │
                   ▼
        ┌─────────────────────┐
        │  Figma API Client   │
        │  (Design mutations) │
        └─────────────────────┘
```

### Key Files

| File | Purpose |
|------|---------|
| `code.js` | Figma plugin backend; handles design API calls |
| `ui.html` | Plugin UI panel displayed in Figma |
| `manifest.json` | Plugin metadata and permissions |
| `server/socket.js` | JavaScript WebSocket relay |
| `rust-relay/src/` | Rust relay implementation (optional high-perf) |
| `contracts/figma-mcp-bridge.schema.json` | Message protocol schema |
| `docs/ARCHITECTURE.md` | Detailed architecture documentation |

## Configuration

### Environment Variables (Relay)

#### JavaScript Relay
```bash
# Default: no env vars; uses hardcoded defaults
# To customize, edit server/socket.js
```

#### Rust Relay
```bash
# Binding
FIGMA_MCP_BIND=127.0.0.1:3055           # Full bind address
FIGMA_MCP_BIND_HOST=127.0.0.1           # Host (overridden by FIGMA_MCP_BIND)
FIGMA_MCP_SOCKET_PORT=3055              # Port (overridden by FIGMA_MCP_BIND)

# Behavior
FIGMA_MCP_DEFAULT_CHANNEL=figma-auto    # Default channel name
FIGMA_MCP_COMMAND_TIMEOUT_MS=120000     # Timeout for design operations
FIGMA_MCP_MAX_QUEUE_SIZE=100            # Message queue capacity
FIGMA_MCP_MAX_MESSAGE_BYTES=26214400    # Max message size (~25MB)
FIGMA_MCP_CLIENT_BUFFER_SIZE=256        # Per-client buffer size

# Logging
RUST_LOG=info                            # Rust log level (debug, info, warn, error)
```

### Figma Token Configuration

Your Figma personal access token is stored in `~/.claude.json` under the `figma` (read-only) MCP entry. The `figma-write` entry communicates via the local plugin (no token required).

To rotate or update tokens:
1. Edit `~/.claude.json` in the `figma` entry
2. Restart Claude Code to reload the config

## Project Structure

```
.
├── code.js                    # Figma plugin backend
├── ui.html                    # Plugin UI
├── manifest.json              # Plugin manifest
├── setcharacters.js           # Character management utility
├── start-socket.bat           # Quick-start JS relay (Windows)
├── start-rust-relay.bat       # Quick-start Rust relay (Windows)
├── INSTALL.md                 # Installation guide
├── README.md                  # This file
├── server/
│   └── socket.js              # JavaScript WebSocket relay
├── rust-relay/
│   ├── Cargo.toml             # Rust dependencies
│   ├── README.md              # Rust relay documentation
│   └── src/
│       ├── main.rs
│       ├── web.rs             # WebSocket handler
│       ├── state.rs           # Connection state management
│       ├── protocol.rs        # Message protocol
│       └── config.rs          # Configuration
├── contracts/
│   └── figma-mcp-bridge.schema.json  # Message schema
├── docs/
│   ├── ARCHITECTURE.md        # Architecture deep-dive
│   └── RUST_RELAY.md          # Rust relay details
└── scripts/
    └── enable-auto-channel.ps1  # MCP server patcher

```

## Development

### Setting Up for Development

```bash
# Clone the repo
git clone https://github.com/MustaphaLargou25/figma-plugin-mcp.git
cd figma-plugin-mcp

# Install dependencies (if using Node.js relay)
npm install

# Or use Bun
bun install

# For Rust relay
cd rust-relay
cargo build --release
cd ..
```

### Running Locally

**JS Relay:**
```bash
# Watch mode (if available)
bun server/socket.js

# Or with Node.js
node server/socket.js
```

**Rust Relay:**
```bash
cd rust-relay
cargo run --release
```

### Testing the Connection

1. Start the relay
2. Load the plugin in Figma
3. In Claude Code, run `/mcp` to verify connection
4. Try a simple command:
   ```
   @figma-write Create a text layer with "Hello Claude!" in my current Figma file.
   ```

### Troubleshooting

| Issue | Solution |
|-------|----------|
| Plugin says "Disconnected" | Ensure relay is running on `http://localhost:3055` |
| MCP server not listed in Claude | Restart Claude Code and check `~/.claude.json` |
| Port 3055 already in use | Kill existing relay process or change port in config |
| Figma API errors | Check Figma file permissions and plugin scope |
| Timeout on operations | Increase `FIGMA_MCP_COMMAND_TIMEOUT_MS` |

## Security

### What's Secured
- ✅ **No secrets committed** — `.gitignore` excludes `.env`, `.claude.json`, and sensitive config
- ✅ **Local-only communication** — WebSocket runs on `localhost:3055` (not exposed to internet)
- ✅ **Token isolation** — Figma tokens stored in user config, not in repo
- ✅ **Build artifacts ignored** — `rust-relay/target/` and `node_modules/` excluded

### Best Practices
1. **Never commit credentials** — Keep `.claude.json` and `.env` files local only
2. **Rotate tokens regularly** — Update Figma tokens if compromised
3. **Use HTTPS in production** — If deploying relay over network, use TLS
4. **Monitor logs** — Check relay output for unusual connection patterns

## Performance

### JavaScript Relay
- **Startup**: ~200ms
- **Throughput**: ~1000 msgs/sec
- **Memory**: ~40MB

### Rust Relay
- **Startup**: ~50ms
- **Throughput**: ~5000+ msgs/sec
- **Memory**: ~15MB

For high-volume design operations, use the Rust relay.

## Ports and Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `http://localhost:3055/healthz` | GET | Health check (200 OK) |
| `http://localhost:3055/status` | GET | Server status JSON |
| `ws://localhost:3055/` | WS | WebSocket upgrade |
| `ws://localhost:3055/*` | WS | Wildcard WebSocket route |

## Contributing

Contributions are welcome! Please:
1. Fork the repo
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Commit changes with clear messages
4. Push and open a pull request

## License

This project is provided as-is. Modify and distribute as needed.

## Support & Documentation

- **Architecture Details**: See `docs/ARCHITECTURE.md`
- **Rust Relay Docs**: See `rust-relay/README.md` and `docs/RUST_RELAY.md`
- **Protocol Schema**: See `contracts/figma-mcp-bridge.schema.json`
- **Installation Guide**: See `INSTALL.md`

## Acknowledgments

Built with ❤️ for designers and AI developers working together.

---

**Ready to get started?** Follow the [Quick Start](#quick-start) section above!
