# Write-capable Figma MCP — ready to run

Everything needed is already installed on this machine. You just have to:
1. Start a helper window
2. Load the plugin inside Figma Desktop
3. Restart Claude Code so it picks up the new MCP server

---

## What was installed (already done)

- **Bun 1.3.13** at `C:\Users\hp\.bun\bin\bun.exe` — required by the WebSocket relay.
- **claude-talk-to-figma-mcp 1.0.0** globally via npm, at `C:\Users\hp\AppData\Roaming\npm\node_modules\claude-talk-to-figma-mcp\`.
- **Figma plugin source** in this folder: `code.js`, `manifest.json`, `setcharacters.js`, `ui.html`.
- **MCP entry added** to `C:\Users\hp\.claude.json` under the `C:/Users/hp` project, named `figma-write`. The previous `figma` (read-only) entry is kept.

A backup of the prior config is at `C:\Users\hp\.claude.json.pre-figma-write-mcp-backup` in case anything goes wrong.

---

## Step 1 — Start the WebSocket relay (keep this window open)

Double-click `start-socket.bat` in this folder.

A terminal window will open showing something like:

```
Claude/Figma WebSocket relay running on port 3055
```

Leave it running. Closing this window kills the bridge between Claude and Figma.

Health checks are available while it is running:

```
http://localhost:3055/healthz
http://localhost:3055/status
```

---

## Step 2 — Load the plugin inside Figma Desktop

You need the **Figma desktop app** (not the web app — desktop Figma is required to load a local plugin).

1. Open Figma Desktop → open your file `marolet-x-claude-code`.
2. Menu bar → **Plugins → Development → Import plugin from manifest…**
3. Navigate to `C:\Users\hp\Desktop\figma_mcp_developement\figma-plugin-ready\` and select `manifest.json`.
4. The plugin **Claude Talk to Figma Plugin** now appears under Plugins → Development.
5. Run it: **Plugins → Development → Claude Talk to Figma Plugin**.
6. A small panel opens inside Figma. It auto-connects to the relay using the shared `figma-auto` channel. If the relay was not running yet, leave the panel open and start the relay; it will retry.

---

## Step 3 — Restart Claude Code

Close and reopen the Claude Code window (or the terminal it's running in). This makes it re-read `.claude.json` and load the new `figma-write` MCP server alongside the existing `figma` (read-only) one.

After reopening, verify by typing `/mcp` in Claude Code — you should see both `figma` and `figma-write` listed as connected.

---

## Step 4 — Use Claude normally

No channel ID is needed now. The patched MCP CLI server auto-joins the shared `figma-auto` channel, then uses write tools like `create_frame`, `create_text`, `set_fill_color`, `create_component`, etc.

---

## If something goes wrong

| Symptom | Fix |
|---|---|
| `/mcp` doesn't list `figma-write` | Config not reloaded — fully quit Claude Code and reopen. |
| Plugin says "Disconnected" and won't connect | The socket relay isn't running. Re-run `start-socket.bat`. |
| "Bun is not defined" error when starting the socket | Close and reopen the batch file — Bun PATH wasn't loaded in the old terminal. |
| Port 3055 already in use | Another socket process is running. In Task Manager, end any `bun.exe` or `node.exe` listening on 3055, then restart `start-socket.bat`. |
| You want to remove it later | Delete the `figma-write` key from the `mcpServers` block in `C:\Users\hp\.claude.json`, or restore the backup. |

## Developer notes

- The local relay source lives at `server/socket.js`.
- A high-performance Rust relay replacement lives at `rust-relay/`.
- To use it instead of Bun/Node, stop the JS relay and start `start-rust-relay.bat`.
- The installed MCP CLI server has been patched by `scripts/enable-auto-channel.ps1` to auto-join `figma-auto`.
- The bridge JSON contract lives at `contracts/figma-mcp-bridge.schema.json`.
- Architecture and performance notes live at `docs/ARCHITECTURE.md`.
- Rust relay notes live at `docs/RUST_RELAY.md`.
- You can override relay settings with environment variables before launching Bun:
  - `FIGMA_MCP_SOCKET_PORT`
  - `FIGMA_MCP_DEFAULT_CHANNEL`
  - `FIGMA_MCP_AUTO_CHANNEL`
  - `FIGMA_MCP_COMMAND_TIMEOUT_MS`
  - `FIGMA_MCP_MAX_QUEUE_SIZE`
  - `FIGMA_MCP_MAX_MESSAGE_BYTES`

---

## Security note

Your Figma personal access token is stored in `.claude.json` under the existing `figma` (read-only) MCP entry. The new `figma-write` entry does NOT use that token — it talks to Figma via the locally loaded plugin (no API token needed for write operations). If you want tighter isolation you can remove the old `figma` entry once you've confirmed `figma-write` covers your needs.
