$ErrorActionPreference = "Stop"

$serverPath = "C:\Users\hp\AppData\Roaming\npm\node_modules\claude-talk-to-figma-mcp\dist\talk_to_figma_mcp\server.js"
$backupPath = "$serverPath.pre-auto-channel-backup"

if (!(Test-Path -LiteralPath $serverPath)) {
  throw "MCP server file not found: $serverPath"
}

if (!(Test-Path -LiteralPath $backupPath)) {
  Copy-Item -LiteralPath $serverPath -Destination $backupPath
}

$text = (Get-Content -Raw -LiteralPath $serverPath).Replace("`r`n", "`n")

if ($text.Contains("FIGMA_MCP_AUTO_CHANNEL")) {
  [System.IO.File]::WriteAllText($serverPath, $text, [System.Text.UTF8Encoding]::new($false))
  Write-Host "Auto-channel patch is already installed."
  exit 0
}

function Replace-Once {
  param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Old,
    [Parameter(Mandatory = $true)][string]$New,
    [Parameter(Mandatory = $true)][string]$Label
  )

  if (!$Source.Contains($Old)) {
    throw "Could not find patch target: $Label"
  }

  return $Source.Replace($Old, $New)
}

$oldConfig = @"
var reconnectArg = args.find((arg) => arg.startsWith("--reconnect-interval="));
var serverUrl = serverArg ? serverArg.split("=")[1] : "localhost";
var defaultPort = portArg ? parseInt(portArg.split("=")[1], 10) : 3055;
var reconnectInterval = reconnectArg ? parseInt(reconnectArg.split("=")[1], 10) : 2e3;
"@

$newConfig = @"
var reconnectArg = args.find((arg) => arg.startsWith("--reconnect-interval="));
var autoChannelArg = args.find((arg) => arg.startsWith("--auto-channel=") || arg.startsWith("--channel="));
var serverUrl = serverArg ? serverArg.split("=")[1] : "localhost";
var defaultPort = portArg ? parseInt(portArg.split("=")[1], 10) : 3055;
var reconnectInterval = reconnectArg ? parseInt(reconnectArg.split("=")[1], 10) : 2e3;
var autoChannel = autoChannelArg ? autoChannelArg.split("=")[1] : process.env.FIGMA_MCP_AUTO_CHANNEL || "figma-auto";
if (["", "0", "false", "none", "off"].includes(String(autoChannel).toLowerCase())) autoChannel = null;
"@

$oldPending = 'var pendingRequests = /* @__PURE__ */ new Map();'
$newPending = @"
var pendingRequests = /* @__PURE__ */ new Map();
var autoJoinPromise = null;
"@

$oldOpen = @"
    ws.on("open", () => {
      clearTimeout(connectionTimeout);
      logger.info("Connected to Figma socket server");
      currentChannel = null;
    });
"@

$newOpen = @'
    ws.on("open", () => {
      clearTimeout(connectionTimeout);
      logger.info("Connected to Figma socket server");
      currentChannel = null;
      if (autoChannel) {
        setTimeout(() => {
          ensureAutoChannel().catch((error) => {
            const message = error instanceof Error ? error.message : String(error);
            logger.warn(`Auto-channel join pending: ${message}`);
          });
        }, 0);
      }
    });
'@

$oldJoinAndSend = @"
async function joinChannel(channelName) {
  if (!ws || ws.readyState !== WebSocket.OPEN) {
    throw new Error("Not connected to Figma");
  }
  try {
    await sendCommandToFigma("join", { channel: channelName });
    currentChannel = channelName;
    try {
      await sendCommandToFigma("ping", {}, 12e3);
      logger.info(`Joined channel: ${channelName}`);
    } catch (verificationError) {
      currentChannel = null;
      const errorMsg = verificationError instanceof Error ? verificationError.message : String(verificationError);
      logger.error(`Failed to verify channel ${channelName}: ${errorMsg}`);
      throw new Error(`Failed to verify connection to channel "${channelName}". The Figma plugin may not be connected to this channel.`);
    }
  } catch (error) {
    logger.error(`Failed to join channel: ${error instanceof Error ? error.message : String(error)}`);
    throw error;
  }
}
function sendCommandToFigma(command, params = {}, timeoutMs = 3e5) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      connectToFigma();
      reject(new Error("Not connected to Figma. Attempting to connect..."));
      return;
    }
    const requiresChannel = command !== "join";
    if (requiresChannel && !currentChannel) {
      reject(new Error("Must join a channel before sending commands"));
      return;
    }
    const id = uuidv4();
    const request = {
      id,
      type: command === "join" ? "join" : "message",
      ...command === "join" ? { channel: params.channel, sessionId: SESSION_ID } : { channel: currentChannel },
      message: {
        id,
        command,
        params: {
          ...params,
          commandId: id
          // Include the command ID in params
        }
      }
    };
    const timeout = setTimeout(() => {
      if (pendingRequests.has(id)) {
        pendingRequests.delete(id);
        logger.error(`Request ${id} to Figma timed out after ${timeoutMs / 1e3} seconds`);
        reject(new Error("Request to Figma timed out"));
      }
    }, timeoutMs);
    pendingRequests.set(id, {
      resolve,
      reject,
      timeout,
      lastActivity: Date.now()
    });
    logger.info(`Sending command to Figma: ${command}`);
    logger.debug(`Request details: ${JSON.stringify(request)}`);
    ws.send(JSON.stringify(request));
  });
}
"@

$newJoinAndSend = @'
async function ensureAutoChannel() {
  if (!autoChannel) return false;
  if (currentChannel === autoChannel) return true;
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  if (!autoJoinPromise) {
    autoJoinPromise = joinChannel(autoChannel, { verify: false }).finally(() => {
      autoJoinPromise = null;
    });
  }
  await autoJoinPromise;
  return currentChannel === autoChannel;
}
async function joinChannel(channelName, options = {}) {
  if (!ws || ws.readyState !== WebSocket.OPEN) {
    throw new Error("Not connected to Figma");
  }
  try {
    await sendCommandToFigma("join", { channel: channelName });
    currentChannel = channelName;
    if (options.verify === false || channelName === autoChannel) {
      logger.info(`Joined channel: ${channelName}`);
      return;
    }
    try {
      await sendCommandToFigma("ping", {}, 12e3);
      logger.info(`Joined channel: ${channelName}`);
    } catch (verificationError) {
      currentChannel = null;
      const errorMsg = verificationError instanceof Error ? verificationError.message : String(verificationError);
      logger.error(`Failed to verify channel ${channelName}: ${errorMsg}`);
      throw new Error(`Failed to verify connection to channel "${channelName}". The Figma plugin may not be connected to this channel.`);
    }
  } catch (error) {
    logger.error(`Failed to join channel: ${error instanceof Error ? error.message : String(error)}`);
    throw error;
  }
}
function sendCommandToFigma(command, params = {}, timeoutMs = 3e5) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      connectToFigma();
      reject(new Error("Not connected to Figma. Attempting to connect..."));
      return;
    }

    const sendRequest = () => {
      const id = uuidv4();
      const request = {
        id,
        type: command === "join" ? "join" : "message",
        ...command === "join" ? { channel: params.channel || autoChannel, sessionId: SESSION_ID } : { channel: currentChannel },
        message: {
          id,
          command,
          params: {
            ...params,
            commandId: id
            // Include the command ID in params
          }
        }
      };
      const timeout = setTimeout(() => {
        if (pendingRequests.has(id)) {
          pendingRequests.delete(id);
          logger.error(`Request ${id} to Figma timed out after ${timeoutMs / 1e3} seconds`);
          reject(new Error("Request to Figma timed out"));
        }
      }, timeoutMs);
      pendingRequests.set(id, {
        resolve,
        reject,
        timeout,
        lastActivity: Date.now()
      });
      logger.info(`Sending command to Figma: ${command}`);
      logger.debug(`Request details: ${JSON.stringify(request)}`);
      ws.send(JSON.stringify(request));
    };

    const requiresChannel = command !== "join";
    if (requiresChannel && !currentChannel) {
      if (!autoChannel) {
        reject(new Error("Must join a channel before sending commands"));
        return;
      }
      ensureAutoChannel().then((joined) => {
        if (!joined || !currentChannel) {
          reject(new Error(`Could not auto-join Figma channel "${autoChannel}"`));
          return;
        }
        sendRequest();
      }).catch(reject);
      return;
    }

    sendRequest();
  });
}
'@

$text = Replace-Once -Source $text -Old $oldConfig -New $newConfig -Label "config args"
$text = Replace-Once -Source $text -Old $oldPending -New $newPending -Label "pending requests"
$text = Replace-Once -Source $text -Old $oldOpen -New $newOpen -Label "open auto-join"

$joinPattern = "(?s)async function joinChannel\(channelName\) \{.*?\n\}\nfunction sendCommandToFigma\(command, params = \{\}, timeoutMs = 3e5\) \{.*?\n\}\n\n// src/talk_to_figma_mcp/tools/document-tools.ts"
$joinReplacement = $newJoinAndSend + "`n// src/talk_to_figma_mcp/tools/document-tools.ts"
$patchedText = [regex]::Replace($text, $joinPattern, $joinReplacement, 1)
if ($patchedText -eq $text) {
  throw "Could not find patch target: join/send command block"
}
$text = $patchedText

[System.IO.File]::WriteAllText($serverPath, $text, [System.Text.UTF8Encoding]::new($false))
Write-Host "Patched MCP server for automatic Figma channel: figma-auto"
Write-Host "Backup: $backupPath"
