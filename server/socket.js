// Local Claude/Figma WebSocket relay.
// Runs with Bun and keeps MCP-agent commands serialized per Figma channel.

const PORT = Number(Bun.env.FIGMA_MCP_SOCKET_PORT || "3055");
const DEFAULT_CHANNEL = Bun.env.FIGMA_MCP_DEFAULT_CHANNEL || "figma-auto";
const COMMAND_TIMEOUT_MS = Number(Bun.env.FIGMA_MCP_COMMAND_TIMEOUT_MS || "120000");
const MAX_QUEUE_SIZE = Number(Bun.env.FIGMA_MCP_MAX_QUEUE_SIZE || "100");
const MAX_INCOMING_MESSAGE_BYTES = Number(Bun.env.FIGMA_MCP_MAX_MESSAGE_BYTES || String(25 * 1024 * 1024));
const QUEUE_WARN_DEPTH = 50;

const channels = new Map();
const channelQueues = new Map();
const requestToClient = new Map();
const pluginClients = new Set();
const agentClients = new Set();
const sessionToClient = new Map();

const CREATION_COMMANDS = new Set([
  "create_rectangle",
  "create_frame",
  "create_text",
  "create_ellipse",
  "create_polygon",
  "create_star",
  "create_vector",
  "create_line",
  "create_component_instance",
  "create_component_set",
  "set_svg",
  "clone_node",
  "create_component_from_node",
  "create_section",
  "create_sticky",
  "create_shape_with_text",
  "create_connector",
]);

const BLOCKED_COMMANDS = new Set(["set_current_page"]);

const stats = {
  startedAt: Date.now(),
  totalConnections: 0,
  activeConnections: 0,
  messagesReceived: 0,
  messagesSent: 0,
  errors: 0,
  queuedCommands: 0,
  queueDepthMax: 0,
  queueRejections: 0,
  blockedCommands: 0,
  unicastResponses: 0,
  discardedResponses: 0,
  progressUpdates: 0,
  timeoutErrors: 0,
};

const logger = {
  info: (message, ...args) => console.log(`[INFO] ${message}`, ...args),
  debug: (message, ...args) => {
    if (Bun.env.FIGMA_MCP_DEBUG === "1") console.log(`[DEBUG] ${message}`, ...args);
  },
  warn: (message, ...args) => console.warn(`[WARN] ${message}`, ...args),
  error: (message, ...args) => console.error(`[ERROR] ${message}`, ...args),
};

function clientId(ws) {
  return (ws.data && ws.data.clientId) || "unknown";
}

function safeSend(ws, envelope) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;

  try {
    ws.send(JSON.stringify(envelope));
    stats.messagesSent++;
    return true;
  } catch (error) {
    stats.errors++;
    logger.error(`Failed to send to ${clientId(ws)}:`, error);
    return false;
  }
}

function normalizeChannelName(channelName) {
  if (typeof channelName !== "string") return null;
  const normalized = channelName.trim();
  if (!/^[a-zA-Z0-9_-]{3,64}$/.test(normalized)) return null;
  return normalized;
}

function getChannelClients(channelName) {
  if (!channels.has(channelName)) channels.set(channelName, new Set());
  return channels.get(channelName);
}

function getPluginClient(channelName) {
  const clients = channels.get(channelName);
  if (!clients) return null;

  for (const client of clients) {
    if (pluginClients.has(client) && client.readyState === WebSocket.OPEN) {
      return client;
    }
  }

  return null;
}

function ensureQueueState(channelName) {
  if (!channelQueues.has(channelName)) {
    channelQueues.set(channelName, {
      queue: [],
      isProcessing: false,
      currentRequestId: null,
      currentTimeout: null,
      currentStartedAt: null,
    });
  }

  return channelQueues.get(channelName);
}

function classifyClient(ws, data) {
  if (pluginClients.has(ws) || agentClients.has(ws)) return;

  if (data.type === "progress_update" || data.message?.result !== undefined || data.message?.error !== undefined) {
    pluginClients.add(ws);
    logger.info(`Client ${clientId(ws)} classified as Figma plugin`);
    return;
  }

  if (data.message?.command) {
    agentClients.add(ws);
    logger.info(`Client ${clientId(ws)} classified as MCP agent`);
  }
}

function validateCommand(data) {
  const command = data.message && data.message.command;
  const params = (data.message && data.message.params) || {};

  if (BLOCKED_COMMANDS.has(command)) {
    return `"${command}" is stateful and blocked by the relay. Use parentId on creation commands instead.`;
  }

  if (CREATION_COMMANDS.has(command) && !params.parentId) {
    return `"${command}" requires parentId so writes land on the intended page or frame.`;
  }

  return null;
}

function resetCurrentCommandTimeout(channelName, requestId) {
  const queueState = channelQueues.get(channelName);
  if (!queueState || queueState.currentRequestId !== requestId) return;

  if (queueState.currentTimeout) clearTimeout(queueState.currentTimeout);
  queueState.currentTimeout = setTimeout(() => {
    if (queueState.currentRequestId !== requestId) return;
    stats.timeoutErrors++;
    logger.warn(`Command ${requestId} timed out in channel ${channelName}`);

    const entry = requestToClient.get(requestId);
    if (entry) {
      safeSend(entry.ws, {
        type: "broadcast",
        message: { id: requestId, error: "Command timed out waiting for Figma plugin response" },
        sender: "User",
        channel: channelName,
      });
      requestToClient.delete(requestId);
    }

    queueState.isProcessing = false;
    queueState.currentRequestId = null;
    queueState.currentStartedAt = null;
    queueState.currentTimeout = null;
    processQueue(channelName);
  }, COMMAND_TIMEOUT_MS);
}

function enqueueCommand(data, ws, channelName) {
  const requestId = data.message && data.message.id;
  if (!requestId) {
    safeSend(ws, { type: "error", message: "Command message.id is required" });
    return;
  }

  const queueState = ensureQueueState(channelName);
  if (queueState.queue.length >= MAX_QUEUE_SIZE) {
    stats.queueRejections++;
    safeSend(ws, {
      type: "broadcast",
      message: { id: requestId, error: `Command queue is full (${MAX_QUEUE_SIZE} pending commands)` },
      sender: "You",
      channel: channelName,
    });
    return;
  }

  requestToClient.set(requestId, { ws, timestamp: Date.now(), channelName });
  queueState.queue.push({ data, senderWs: ws, requestId, enqueuedAt: Date.now() });
  stats.queuedCommands++;
  stats.queueDepthMax = Math.max(stats.queueDepthMax, queueState.queue.length);

  if (queueState.queue.length > QUEUE_WARN_DEPTH) {
    logger.warn(`Queue depth ${queueState.queue.length} in channel ${channelName}`);
  }

  processQueue(channelName);
}

function processQueue(channelName) {
  const queueState = channelQueues.get(channelName);
  if (!queueState || queueState.isProcessing || queueState.queue.length === 0) return;

  const item = queueState.queue.shift();
  const payload = {
    type: "broadcast",
    message: item.data.message,
    sender: "User",
    channel: channelName,
  };

  let forwarded = false;
  const pluginClient = getPluginClient(channelName);
  if (pluginClient) {
    forwarded = safeSend(pluginClient, payload);
  } else {
    const clients = channels.get(channelName) || new Set();
    for (const client of clients) {
      if (client !== item.senderWs && !agentClients.has(client) && client.readyState === WebSocket.OPEN) {
        forwarded = safeSend(client, payload) || forwarded;
      }
    }
  }

  if (!forwarded) {
    safeSend(item.senderWs, {
      type: "broadcast",
      message: { id: item.requestId, error: "No Figma plugin connected to this channel" },
      sender: "You",
      channel: channelName,
    });
    requestToClient.delete(item.requestId);
    setTimeout(() => processQueue(channelName), 0);
    return;
  }

  queueState.isProcessing = true;
  queueState.currentRequestId = item.requestId;
  queueState.currentStartedAt = Date.now();
  resetCurrentCommandTimeout(channelName, item.requestId);

  safeSend(item.senderWs, {
    type: "broadcast",
    message: item.data.message,
    sender: "You",
    channel: channelName,
  });

  queueState.queue.forEach((waiting, index) => {
    safeSend(waiting.senderWs, {
      type: "queue_position",
      id: waiting.requestId,
      position: index + 1,
      queueSize: queueState.queue.length,
      message: {
        data: {
          status: "queued",
          progress: 0,
          message: `Queued at position ${index + 1} of ${queueState.queue.length}`,
        },
      },
    });
  });
}

function finishCurrentCommand(channelName, requestId) {
  const queueState = channelQueues.get(channelName);
  if (!queueState || queueState.currentRequestId !== requestId) return;

  if (queueState.currentTimeout) clearTimeout(queueState.currentTimeout);
  queueState.isProcessing = false;
  queueState.currentRequestId = null;
  queueState.currentStartedAt = null;
  queueState.currentTimeout = null;
  processQueue(channelName);
}

function handleResponseFromPlugin(data, channelName) {
  const responseId = data.message && data.message.id;
  const entry = responseId ? requestToClient.get(responseId) : null;

  if (entry && entry.ws.readyState === WebSocket.OPEN) {
    safeSend(entry.ws, {
      type: "broadcast",
      message: data.message,
      sender: "User",
      channel: channelName,
    });
    stats.unicastResponses++;
    requestToClient.delete(responseId);
  } else {
    stats.discardedResponses++;
    if (responseId) requestToClient.delete(responseId);
    logger.debug(`Discarded orphaned response ${responseId || "<missing id>"}`);
  }

  if (responseId) finishCurrentCommand(channelName, responseId);
}

function handleProgressUpdate(data, ws) {
  const channelName = normalizeChannelName(data.channel || DEFAULT_CHANNEL);
  if (!channelName) return;

  classifyClient(ws, data);
  stats.progressUpdates++;
  const requestId = data.id || (data.message && data.message.id);
  if (requestId) resetCurrentCommandTimeout(channelName, requestId);

  const entry = requestId ? requestToClient.get(requestId) : null;
  if (entry && entry.ws.readyState === WebSocket.OPEN) {
    safeSend(entry.ws, data);
    return;
  }

  const clients = channels.get(channelName);
  if (!clients) return;
  for (const client of clients) {
    if (client !== ws && client.readyState === WebSocket.OPEN) safeSend(client, data);
  }
}

function removeClientFromChannels(ws) {
  const clientChannels = [];
  for (const [channelName, clients] of channels.entries()) {
    if (clients.delete(ws)) {
      clientChannels.push(channelName);
      for (const client of clients) {
        safeSend(client, {
          type: "system",
          message: "A client has left the channel",
          channel: channelName,
        });
      }
      if (clients.size === 0) {
        channels.delete(channelName);
        channelQueues.delete(channelName);
      }
    }
  }
  return clientChannels;
}

function cleanupClient(ws, clientChannels = []) {
  const wasPlugin = pluginClients.has(ws);

  if (wasPlugin) {
    const channelsToCheck = clientChannels.length > 0 ? clientChannels : Array.from(channelQueues.keys());
    for (const channelName of channelsToCheck) {
      const queueState = channelQueues.get(channelName);
      if (!queueState || !queueState.currentRequestId) continue;

      const requestId = queueState.currentRequestId;
      const entry = requestToClient.get(requestId);
      if (entry) {
        safeSend(entry.ws, {
          type: "broadcast",
          message: { id: requestId, error: "Figma plugin disconnected while processing command" },
          sender: "User",
          channel: channelName,
        });
        requestToClient.delete(requestId);
      }

      if (queueState.currentTimeout) clearTimeout(queueState.currentTimeout);
      queueState.isProcessing = false;
      queueState.currentRequestId = null;
      queueState.currentStartedAt = null;
      queueState.currentTimeout = null;
      setTimeout(() => processQueue(channelName), 0);
    }
  }

  for (const [channelName, queueState] of channelQueues.entries()) {
    queueState.queue = queueState.queue.filter((item) => {
      if (item.senderWs === ws) {
        requestToClient.delete(item.requestId);
        return false;
      }
      return true;
    });
  }

  for (const [requestId, entry] of requestToClient.entries()) {
    if (entry.ws === ws) requestToClient.delete(requestId);
  }

  if (ws.data && ws.data.sessionId && sessionToClient.get(ws.data.sessionId) === ws) {
    sessionToClient.delete(ws.data.sessionId);
  }

  pluginClients.delete(ws);
  agentClients.delete(ws);
}

function handleJoin(ws, data) {
  const channelName = normalizeChannelName(data.channel || DEFAULT_CHANNEL);
  if (!channelName) {
    safeSend(ws, { type: "error", message: "Valid channel is required (3-64 chars: letters, numbers, _, -)" });
    return;
  }

  if (typeof data.sessionId === "string" && data.sessionId.length > 0) {
    const stale = sessionToClient.get(data.sessionId);
    if (stale && stale !== ws) {
      const staleChannels = removeClientFromChannels(stale);
      cleanupClient(stale, staleChannels);
      try {
        stale.close(1000, "Replaced by reconnecting session");
      } catch (_) {
      }
    }
    sessionToClient.set(data.sessionId, ws);
    ws.data.sessionId = data.sessionId;
  }

  getChannelClients(channelName).add(ws);
  safeSend(ws, { type: "system", message: `Joined channel: ${channelName}`, channel: channelName });
  safeSend(ws, {
    type: "system",
    message: {
      id: data.id,
      result: `Connected to channel: ${channelName}`,
    },
    channel: channelName,
  });

  logger.info(`Client ${clientId(ws)} joined channel ${channelName}`);
}

function handleMessage(ws, data) {
  const channelName = normalizeChannelName(data.channel || DEFAULT_CHANNEL);
  if (!channelName) {
    safeSend(ws, { type: "error", message: "Channel name is required" });
    return;
  }

  const clients = channels.get(channelName);
  if (!clients || !clients.has(ws)) {
    safeSend(ws, { type: "error", message: "You must join the channel first" });
    return;
  }

  classifyClient(ws, data);
  const isResponse = data.message && (data.message.result !== undefined || data.message.error !== undefined);
  const isCommand = data.message && data.message.command;

  if (isResponse) {
    handleResponseFromPlugin(data, channelName);
    return;
  }

  if (isCommand) {
    const validationError = validateCommand(data);
    if (validationError) {
      stats.blockedCommands++;
      safeSend(ws, {
        type: "broadcast",
        message: { id: data.message.id, error: validationError },
        sender: "You",
        channel: channelName,
      });
      return;
    }

    enqueueCommand(data, ws, channelName);
    return;
  }

  for (const client of clients) {
    safeSend(client, {
      type: "broadcast",
      message: data.message,
      sender: client === ws ? "You" : "User",
      channel: channelName,
    });
  }
}

function parseSocketPayload(message) {
  const raw = typeof message === "string" ? message : new TextDecoder().decode(message);
  if (raw.length > MAX_INCOMING_MESSAGE_BYTES) {
    throw new Error(`Incoming message exceeds ${MAX_INCOMING_MESSAGE_BYTES} bytes`);
  }
  return JSON.parse(raw);
}

function statusPayload() {
  return {
    status: "running",
    uptimeMs: Date.now() - stats.startedAt,
    defaultChannel: DEFAULT_CHANNEL,
    stats,
    queue: {
      pendingRequests: requestToClient.size,
      agentCount: agentClients.size,
      pluginCount: pluginClients.size,
      channels: Array.from(channelQueues.entries()).map(([channel, state]) => ({
        channel,
        queueDepth: state.queue.length,
        isProcessing: state.isProcessing,
        currentRequestId: state.currentRequestId,
        currentAgeMs: state.currentStartedAt ? Date.now() - state.currentStartedAt : 0,
      })),
    },
  };
}

const server = Bun.serve({
  port: PORT,
  fetch(req, serverApi) {
    const url = new URL(req.url);
    const headers = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "GET, OPTIONS",
      "Access-Control-Allow-Headers": "Content-Type, Authorization",
    };

    if (req.method === "OPTIONS") return new Response(null, { headers });

    if (url.pathname === "/healthz") {
      return Response.json({
        status: "ok",
        uptimeMs: Date.now() - stats.startedAt,
        defaultChannel: DEFAULT_CHANNEL,
        pluginCount: pluginClients.size,
        agentCount: agentClients.size,
      }, { headers });
    }

    if (url.pathname === "/status") {
      return Response.json(statusPayload(), { headers });
    }

    if (serverApi.upgrade(req, { headers })) return;

    return new Response("Claude/Figma WebSocket relay is running. Use /healthz or /status for diagnostics.", {
      headers: { ...headers, "Content-Type": "text/plain" },
    });
  },
  websocket: {
    open(ws) {
      stats.totalConnections++;
      stats.activeConnections++;
      ws.data = {
        clientId: `client_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
      };
      logger.info(`Client connected: ${clientId(ws)}`);
      safeSend(ws, {
        type: "system",
        message: "Please join a channel to start communicating with Figma",
      });
    },
    message(ws, message) {
      try {
        stats.messagesReceived++;
        const data = parseSocketPayload(message);

        if (data.type === "join") {
          handleJoin(ws, data);
          return;
        }

        if (data.type === "message") {
          handleMessage(ws, data);
          return;
        }

        if (data.type === "progress_update") {
          handleProgressUpdate(data, ws);
          return;
        }

        safeSend(ws, { type: "error", message: `Unsupported message type: ${data.type}` });
      } catch (error) {
        stats.errors++;
        safeSend(ws, {
          type: "error",
          message: `Error processing message: ${error instanceof Error ? error.message : String(error)}`,
        });
      }
    },
    close(ws, code, reason) {
      logger.info(`Client closed: ${clientId(ws)} code=${code} reason=${reason || "none"}`);
      const clientChannels = removeClientFromChannels(ws);
      cleanupClient(ws, clientChannels);
      stats.activeConnections = Math.max(0, stats.activeConnections - 1);
    },
    drain(ws) {
      logger.debug(`Backpressure relieved for ${clientId(ws)}`);
    },
  },
});

logger.info(`Claude/Figma WebSocket relay running on port ${server.port}`);
logger.info(`Health: http://localhost:${server.port}/healthz`);
logger.info(`Status: http://localhost:${server.port}/status`);
