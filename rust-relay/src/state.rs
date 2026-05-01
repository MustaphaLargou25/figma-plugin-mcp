use crate::{
    config::RelayConfig,
    protocol::{
        broadcast_message, command_error_broadcast, error_text, queue_position, system_join_result,
        system_joined, system_text, BridgeEnvelope, Outbound,
    },
};
use dashmap::{DashMap, DashSet};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

type ClientId = String;

const CREATION_COMMANDS: &[&str] = &[
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
];

const BLOCKED_COMMANDS: &[&str] = &["set_current_page"];

#[derive(Debug, Clone)]
pub struct ClientHandle {
    tx: mpsc::Sender<Outbound>,
}

#[derive(Debug)]
struct PendingRequest {
    client_id: ClientId,
    channel: String,
    timestamp: Instant,
}

#[derive(Debug)]
struct QueuedCommand {
    message: Value,
    sender_id: ClientId,
    request_id: String,
    enqueued_at: Instant,
}

#[derive(Debug, Default)]
struct ChannelQueue {
    queue: VecDeque<QueuedCommand>,
    is_processing: bool,
    current_request_id: Option<String>,
    current_started_at: Option<Instant>,
    current_timeout: Option<JoinHandle<()>>,
}

#[derive(Debug, Default)]
pub struct RelayStats {
    total_connections: AtomicU64,
    active_connections: AtomicU64,
    messages_received: AtomicU64,
    messages_sent: AtomicU64,
    errors: AtomicU64,
    queued_commands: AtomicU64,
    queue_depth_max: AtomicU64,
    queue_rejections: AtomicU64,
    blocked_commands: AtomicU64,
    unicast_responses: AtomicU64,
    discarded_responses: AtomicU64,
    progress_updates: AtomicU64,
    timeout_errors: AtomicU64,
    backpressure_drops: AtomicU64,
    cleaned_stale_requests: AtomicU64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    pub total_connections: u64,
    pub active_connections: u64,
    pub messages_received: u64,
    pub messages_sent: u64,
    pub errors: u64,
    pub queued_commands: u64,
    pub queue_depth_max: u64,
    pub queue_rejections: u64,
    pub blocked_commands: u64,
    pub unicast_responses: u64,
    pub discarded_responses: u64,
    pub progress_updates: u64,
    pub timeout_errors: u64,
    pub backpressure_drops: u64,
    pub cleaned_stale_requests: u64,
}

impl RelayStats {
    fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            total_connections: self.total_connections.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            queued_commands: self.queued_commands.load(Ordering::Relaxed),
            queue_depth_max: self.queue_depth_max.load(Ordering::Relaxed),
            queue_rejections: self.queue_rejections.load(Ordering::Relaxed),
            blocked_commands: self.blocked_commands.load(Ordering::Relaxed),
            unicast_responses: self.unicast_responses.load(Ordering::Relaxed),
            discarded_responses: self.discarded_responses.load(Ordering::Relaxed),
            progress_updates: self.progress_updates.load(Ordering::Relaxed),
            timeout_errors: self.timeout_errors.load(Ordering::Relaxed),
            backpressure_drops: self.backpressure_drops.load(Ordering::Relaxed),
            cleaned_stale_requests: self.cleaned_stale_requests.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthSnapshot {
    pub status: &'static str,
    pub uptime_ms: u128,
    pub plugin_count: usize,
    pub agent_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueChannelSnapshot {
    pub channel: String,
    pub queue_depth: usize,
    pub is_processing: bool,
    pub current_request_id: Option<String>,
    pub current_age_ms: u128,
    pub oldest_queued_age_ms: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub pending_requests: usize,
    pub agent_count: usize,
    pub plugin_count: usize,
    pub channels: Vec<QueueChannelSnapshot>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub status: &'static str,
    pub uptime_ms: u128,
    pub stats: StatsSnapshot,
    pub queue: QueueSnapshot,
}

#[derive(Debug)]
pub struct AppState {
    pub config: RelayConfig,
    started_at: Instant,
    stats: RelayStats,
    clients: DashMap<ClientId, ClientHandle>,
    channels: DashMap<String, DashSet<ClientId>>,
    queues: DashMap<String, Arc<Mutex<ChannelQueue>>>,
    request_to_client: DashMap<String, PendingRequest>,
    plugin_clients: DashSet<ClientId>,
    agent_clients: DashSet<ClientId>,
    session_to_client: DashMap<String, ClientId>,
    client_sessions: DashMap<ClientId, String>,
    next_client_id: AtomicU64,
}

impl AppState {
    pub fn new(config: RelayConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            started_at: Instant::now(),
            stats: RelayStats::default(),
            clients: DashMap::new(),
            channels: DashMap::new(),
            queues: DashMap::new(),
            request_to_client: DashMap::new(),
            plugin_clients: DashSet::new(),
            agent_clients: DashSet::new(),
            session_to_client: DashMap::new(),
            client_sessions: DashMap::new(),
            next_client_id: AtomicU64::new(1),
        })
    }

    pub fn register_client(self: &Arc<Self>, tx: mpsc::Sender<Outbound>) -> ClientId {
        let ordinal = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        let client_id = format!("client_{}_{}", ordinal, Uuid::new_v4().simple());
        self.clients.insert(client_id.clone(), ClientHandle { tx });
        self.stats.total_connections.fetch_add(1, Ordering::Relaxed);
        self.stats
            .active_connections
            .fetch_add(1, Ordering::Relaxed);
        info!(%client_id, "client connected");
        self.send_value(
            &client_id,
            system_text("Please join a channel to start communicating with Figma"),
        );
        client_id
    }

    pub async fn disconnect_client(self: &Arc<Self>, client_id: &str) {
        let client_channels = self.remove_client_from_channels(client_id);
        self.cleanup_client(client_id, client_channels).await;
        self.clients.remove(client_id);
        self.stats
            .active_connections
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_sub(1)
            })
            .ok();
        info!(%client_id, "client disconnected");
    }

    pub async fn handle_incoming(self: &Arc<Self>, client_id: &str, raw: String) {
        if raw.len() > self.config.max_message_bytes {
            self.stats.errors.fetch_add(1, Ordering::Relaxed);
            self.send_value(
                client_id,
                error_text(format!(
                    "Incoming message exceeds {} bytes",
                    self.config.max_message_bytes
                )),
            );
            return;
        }

        self.stats.messages_received.fetch_add(1, Ordering::Relaxed);
        let envelope = match serde_json::from_str::<BridgeEnvelope>(&raw) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.stats.errors.fetch_add(1, Ordering::Relaxed);
                self.send_value(
                    client_id,
                    error_text(format!("Invalid JSON message: {error}")),
                );
                return;
            }
        };
        let raw: Arc<str> = Arc::from(raw);

        match envelope.kind.as_str() {
            "join" => self.handle_join(client_id, envelope).await,
            "message" => self.handle_message(client_id, envelope).await,
            "progress_update" => self.handle_progress_update(client_id, envelope, raw).await,
            other => {
                self.stats.errors.fetch_add(1, Ordering::Relaxed);
                self.send_value(
                    client_id,
                    error_text(format!("Unsupported message type: {other}")),
                );
            }
        }
    }

    pub fn health_snapshot(&self) -> HealthSnapshot {
        HealthSnapshot {
            status: "ok",
            uptime_ms: self.started_at.elapsed().as_millis(),
            plugin_count: self.plugin_clients.len(),
            agent_count: self.agent_clients.len(),
        }
    }

    pub async fn status_snapshot(&self) -> StatusSnapshot {
        let now = Instant::now();
        let mut channels = Vec::new();

        for entry in self.queues.iter() {
            let channel = entry.key().clone();
            let queue = entry.value().lock().await;
            channels.push(QueueChannelSnapshot {
                channel,
                queue_depth: queue.queue.len(),
                is_processing: queue.is_processing,
                current_request_id: queue.current_request_id.clone(),
                current_age_ms: queue
                    .current_started_at
                    .map(|started_at| now.saturating_duration_since(started_at).as_millis())
                    .unwrap_or(0),
                oldest_queued_age_ms: queue
                    .queue
                    .front()
                    .map(|item| now.saturating_duration_since(item.enqueued_at).as_millis())
                    .unwrap_or(0),
            });
        }

        channels.sort_by(|left, right| left.channel.cmp(&right.channel));

        StatusSnapshot {
            status: "running",
            uptime_ms: self.started_at.elapsed().as_millis(),
            stats: self.stats.snapshot(),
            queue: QueueSnapshot {
                pending_requests: self.request_to_client.len(),
                agent_count: self.agent_clients.len(),
                plugin_count: self.plugin_clients.len(),
                channels,
            },
        }
    }

    async fn handle_join(self: &Arc<Self>, client_id: &str, envelope: BridgeEnvelope) {
        let Some(channel) = envelope.channel.as_deref().and_then(normalize_channel_name) else {
            self.send_value(
                client_id,
                error_text("Valid channel is required (3-64 chars: letters, numbers, _, -)"),
            );
            return;
        };

        if let Some(session_id) = envelope
            .session_id
            .as_deref()
            .filter(|session_id| !session_id.is_empty())
        {
            if let Some(stale_client_id) = self
                .session_to_client
                .insert(session_id.to_string(), client_id.to_string())
            {
                if stale_client_id != client_id {
                    let stale_channels = self.remove_client_from_channels(&stale_client_id);
                    self.cleanup_client(&stale_client_id, stale_channels).await;
                    self.close_client(&stale_client_id, 1000, "Replaced by reconnecting session");
                }
            }
            self.client_sessions
                .insert(client_id.to_string(), session_id.to_string());
        }

        self.channels
            .entry(channel.clone())
            .or_insert_with(DashSet::new)
            .insert(client_id.to_string());
        self.ensure_queue(&channel);

        self.send_value(client_id, system_joined(&channel));
        self.send_value(
            client_id,
            system_join_result(envelope.id.as_deref(), &channel),
        );
        info!(%client_id, %channel, "client joined channel");
    }

    async fn handle_message(self: &Arc<Self>, client_id: &str, envelope: BridgeEnvelope) {
        let Some(channel) = envelope.channel.as_deref().and_then(normalize_channel_name) else {
            self.send_value(client_id, error_text("Channel name is required"));
            return;
        };

        if !self.client_is_in_channel(client_id, &channel) {
            self.send_value(client_id, error_text("You must join the channel first"));
            return;
        }

        self.classify_client(client_id, &envelope);

        if envelope.is_response() {
            self.handle_response_from_plugin(&channel, envelope).await;
            return;
        }

        if envelope.command().is_some() {
            if let Some(validation_error) = validate_command(&envelope) {
                self.stats.blocked_commands.fetch_add(1, Ordering::Relaxed);
                let request_id = envelope.message_id().unwrap_or("");
                self.send_value(
                    client_id,
                    command_error_broadcast(request_id, &channel, validation_error),
                );
                return;
            }

            self.enqueue_command(&channel, client_id, envelope).await;
            return;
        }

        let message = envelope.message.unwrap_or(Value::Null);
        let you = serialize_value(broadcast_message(message.clone(), &channel, "You"));
        let user = serialize_value(broadcast_message(message, &channel, "User"));

        for target_id in self.channel_client_ids(&channel) {
            if target_id == client_id {
                self.send_raw(&target_id, you.clone());
            } else {
                self.send_raw(&target_id, user.clone());
            }
        }
    }

    async fn handle_progress_update(
        self: &Arc<Self>,
        client_id: &str,
        envelope: BridgeEnvelope,
        raw: Arc<str>,
    ) {
        let Some(channel) = envelope.channel.as_deref().and_then(normalize_channel_name) else {
            return;
        };

        self.classify_client(client_id, &envelope);
        self.stats.progress_updates.fetch_add(1, Ordering::Relaxed);

        if let Some(request_id) = envelope.message_id() {
            self.reset_current_command_timeout(&channel, request_id)
                .await;

            if let Some(entry) = self.request_to_client.get(request_id) {
                let target_id = entry.client_id.clone();
                drop(entry);
                self.send_raw(&target_id, raw);
                return;
            }
        }

        for target_id in self.channel_client_ids(&channel) {
            if target_id != client_id {
                self.send_raw(&target_id, raw.clone());
            }
        }
    }

    async fn enqueue_command(
        self: &Arc<Self>,
        channel: &str,
        client_id: &str,
        envelope: BridgeEnvelope,
    ) {
        let Some(request_id) = envelope.message_id().map(ToOwned::to_owned) else {
            self.send_value(client_id, error_text("Command message.id is required"));
            return;
        };
        let Some(message) = envelope.message else {
            self.send_value(client_id, error_text("Command message payload is required"));
            return;
        };

        let queue = self.ensure_queue(channel);
        {
            let mut queue = queue.lock().await;
            if queue.queue.len() >= self.config.max_queue_size {
                self.stats.queue_rejections.fetch_add(1, Ordering::Relaxed);
                self.send_value(
                    client_id,
                    command_error_broadcast(
                        &request_id,
                        channel,
                        format!(
                            "Command queue is full ({} pending commands)",
                            self.config.max_queue_size
                        ),
                    ),
                );
                return;
            }

            queue.queue.push_back(QueuedCommand {
                message,
                sender_id: client_id.to_string(),
                request_id: request_id.clone(),
                enqueued_at: Instant::now(),
            });

            self.stats.queued_commands.fetch_add(1, Ordering::Relaxed);
            update_max_atomic(&self.stats.queue_depth_max, queue.queue.len() as u64);
        }

        self.request_to_client.insert(
            request_id,
            PendingRequest {
                client_id: client_id.to_string(),
                channel: channel.to_string(),
                timestamp: Instant::now(),
            },
        );

        self.process_queue(channel.to_string()).await;
    }

    async fn process_queue(self: &Arc<Self>, channel: String) {
        loop {
            let queue = match self.queues.get(&channel) {
                Some(queue) => queue.clone(),
                None => return,
            };

            let (item, queued_positions) = {
                let mut queue = queue.lock().await;
                if queue.is_processing {
                    return;
                }
                let Some(item) = queue.queue.pop_front() else {
                    return;
                };

                queue.is_processing = true;
                queue.current_request_id = Some(item.request_id.clone());
                queue.current_started_at = Some(Instant::now());
                queue.current_timeout =
                    Some(self.spawn_timeout(channel.clone(), item.request_id.clone()));

                let queued_positions = queue
                    .queue
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        (item.sender_id.clone(), item.request_id.clone(), index + 1)
                    })
                    .collect::<Vec<_>>();

                (item, queued_positions)
            };

            let payload =
                serialize_value(broadcast_message(item.message.clone(), &channel, "User"));
            let mut forwarded = false;

            if let Some(plugin_id) = self.plugin_client_for_channel(&channel) {
                forwarded = self.send_raw(&plugin_id, payload.clone());
            } else {
                for target_id in self.channel_client_ids(&channel) {
                    if target_id != item.sender_id && !self.agent_clients.contains(&target_id) {
                        forwarded = self.send_raw(&target_id, payload.clone()) || forwarded;
                    }
                }
            }

            if !forwarded {
                self.send_value(
                    &item.sender_id,
                    command_error_broadcast(
                        &item.request_id,
                        &channel,
                        "No Figma plugin connected to this channel",
                    ),
                );
                self.request_to_client.remove(&item.request_id);
                self.clear_current_command(&channel, &item.request_id).await;
                continue;
            }

            self.send_value(
                &item.sender_id,
                broadcast_message(item.message, &channel, "You"),
            );

            let queue_size = queued_positions.len();
            for (sender_id, request_id, position) in queued_positions {
                self.send_value(
                    &sender_id,
                    queue_position(&request_id, position, queue_size),
                );
            }

            return;
        }
    }

    async fn handle_response_from_plugin(
        self: &Arc<Self>,
        channel: &str,
        envelope: BridgeEnvelope,
    ) {
        let Some(response_id) = envelope.message_id().map(ToOwned::to_owned) else {
            self.stats
                .discarded_responses
                .fetch_add(1, Ordering::Relaxed);
            return;
        };
        let message = envelope.message.unwrap_or(Value::Null);

        if let Some((_, pending)) = self.request_to_client.remove(&response_id) {
            debug!(
                request_id = %response_id,
                pending_channel = %pending.channel,
                pending_age_ms = pending.timestamp.elapsed().as_millis(),
                "routing plugin response"
            );
            self.send_value(
                &pending.client_id,
                broadcast_message(message, channel, "User"),
            );
            self.stats.unicast_responses.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats
                .discarded_responses
                .fetch_add(1, Ordering::Relaxed);
            debug!(request_id = %response_id, "discarded orphaned response");
        }

        self.finish_current_command(channel, &response_id).await;
    }

    async fn reset_current_command_timeout(self: &Arc<Self>, channel: &str, request_id: &str) {
        let Some(queue) = self.queues.get(channel).map(|queue| queue.clone()) else {
            return;
        };

        let mut queue = queue.lock().await;
        if queue.current_request_id.as_deref() != Some(request_id) {
            return;
        }

        if let Some(timeout) = queue.current_timeout.take() {
            timeout.abort();
        }
        queue.current_timeout =
            Some(self.spawn_timeout(channel.to_string(), request_id.to_string()));
    }

    async fn finish_current_command(self: &Arc<Self>, channel: &str, request_id: &str) {
        self.clear_current_command(channel, request_id).await;
        self.process_queue(channel.to_string()).await;
    }

    async fn clear_current_command(&self, channel: &str, request_id: &str) {
        let Some(queue) = self.queues.get(channel).map(|queue| queue.clone()) else {
            return;
        };

        let mut queue = queue.lock().await;
        if queue.current_request_id.as_deref() != Some(request_id) {
            return;
        }

        if let Some(timeout) = queue.current_timeout.take() {
            timeout.abort();
        }
        queue.is_processing = false;
        queue.current_request_id = None;
        queue.current_started_at = None;
    }

    fn spawn_timeout(self: &Arc<Self>, channel: String, request_id: String) -> JoinHandle<()> {
        let state = Arc::clone(self);
        let timeout = self.config.command_timeout;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            state.handle_command_timeout(channel, request_id).await;
        })
    }

    async fn handle_command_timeout(self: Arc<Self>, channel: String, request_id: String) {
        let Some(queue) = self.queues.get(&channel).map(|queue| queue.clone()) else {
            return;
        };

        {
            let mut queue = queue.lock().await;
            if queue.current_request_id.as_deref() != Some(request_id.as_str()) {
                return;
            }

            queue.is_processing = false;
            queue.current_request_id = None;
            queue.current_started_at = None;
            queue.current_timeout = None;
        }

        self.stats.timeout_errors.fetch_add(1, Ordering::Relaxed);
        warn!(%channel, %request_id, "command timed out");

        if let Some((_, pending)) = self.request_to_client.remove(&request_id) {
            self.send_value(
                &pending.client_id,
                command_error_broadcast(
                    &request_id,
                    &channel,
                    "Command timed out waiting for Figma plugin response",
                ),
            );
        }

        self.process_queue(channel).await;
    }

    async fn cleanup_client(self: &Arc<Self>, client_id: &str, client_channels: Vec<String>) {
        let was_plugin = self.plugin_clients.contains(client_id);

        if was_plugin {
            let channels_to_check = if client_channels.is_empty() {
                self.queues
                    .iter()
                    .map(|entry| entry.key().clone())
                    .collect::<Vec<_>>()
            } else {
                client_channels.clone()
            };

            for channel in channels_to_check {
                let Some(queue) = self.queues.get(&channel).map(|queue| queue.clone()) else {
                    continue;
                };
                let request_id = {
                    let mut queue = queue.lock().await;
                    let request_id = queue.current_request_id.clone();
                    if request_id.is_some() {
                        if let Some(timeout) = queue.current_timeout.take() {
                            timeout.abort();
                        }
                        queue.is_processing = false;
                        queue.current_request_id = None;
                        queue.current_started_at = None;
                    }
                    request_id
                };

                if let Some(request_id) = request_id {
                    if let Some((_, pending)) = self.request_to_client.remove(&request_id) {
                        self.send_value(
                            &pending.client_id,
                            command_error_broadcast(
                                &request_id,
                                &channel,
                                "Figma plugin disconnected while processing command",
                            ),
                        );
                    }
                    let state = Arc::clone(self);
                    tokio::spawn(async move {
                        state.process_queue(channel).await;
                    });
                }
            }
        }

        let queue_channels = self
            .queues
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for channel in queue_channels {
            let Some(queue) = self.queues.get(&channel).map(|queue| queue.clone()) else {
                continue;
            };
            let mut removed_request_ids = Vec::new();
            {
                let mut queue = queue.lock().await;
                queue.queue.retain(|item| {
                    if item.sender_id == client_id {
                        removed_request_ids.push(item.request_id.clone());
                        false
                    } else {
                        true
                    }
                });
            }
            for request_id in removed_request_ids {
                self.request_to_client.remove(&request_id);
            }
        }

        self.request_to_client
            .retain(|_, pending| pending.client_id != client_id);

        if let Some((_, session_id)) = self.client_sessions.remove(client_id) {
            self.session_to_client.remove(&session_id);
        }

        self.plugin_clients.remove(client_id);
        self.agent_clients.remove(client_id);
    }

    fn classify_client(&self, client_id: &str, envelope: &BridgeEnvelope) {
        if self.plugin_clients.contains(client_id) || self.agent_clients.contains(client_id) {
            return;
        }

        if envelope.kind == "progress_update" || envelope.is_response() {
            self.plugin_clients.insert(client_id.to_string());
            info!(%client_id, "client classified as Figma plugin");
            return;
        }

        if envelope.command().is_some() {
            self.agent_clients.insert(client_id.to_string());
            info!(%client_id, "client classified as MCP agent");
        }
    }

    fn ensure_queue(&self, channel: &str) -> Arc<Mutex<ChannelQueue>> {
        self.queues
            .entry(channel.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(ChannelQueue::default())))
            .clone()
    }

    fn client_is_in_channel(&self, client_id: &str, channel: &str) -> bool {
        self.channels
            .get(channel)
            .is_some_and(|clients| clients.contains(client_id))
    }

    fn channel_client_ids(&self, channel: &str) -> Vec<ClientId> {
        self.channels
            .get(channel)
            .map(|clients| clients.iter().map(|client_id| client_id.clone()).collect())
            .unwrap_or_default()
    }

    fn plugin_client_for_channel(&self, channel: &str) -> Option<ClientId> {
        self.channel_client_ids(channel)
            .into_iter()
            .find(|client_id| {
                self.plugin_clients.contains(client_id) && self.clients.contains_key(client_id)
            })
    }

    fn remove_client_from_channels(&self, client_id: &str) -> Vec<String> {
        let channel_names = self
            .channels
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        let mut removed_from = Vec::new();

        for channel in channel_names {
            let mut should_remove_channel = false;
            let mut notify = Vec::new();

            if let Some(clients) = self.channels.get(&channel) {
                if clients.remove(client_id).is_some() {
                    removed_from.push(channel.clone());
                    notify = clients.iter().map(|client_id| client_id.clone()).collect();
                    should_remove_channel = clients.is_empty();
                }
            }

            for target_id in notify {
                self.send_value(
                    &target_id,
                    serde_json::json!({
                        "type": "system",
                        "message": "A client has left the channel",
                        "channel": channel
                    }),
                );
            }

            if should_remove_channel {
                self.channels.remove(&channel);
                self.queues.remove(&channel);
            }
        }

        removed_from
    }

    fn send_value(&self, client_id: &str, value: Value) -> bool {
        self.send_raw(client_id, serialize_value(value))
    }

    fn send_raw(&self, client_id: &str, raw: Arc<str>) -> bool {
        let Some(client) = self.clients.get(client_id) else {
            return false;
        };

        match client.tx.try_send(Outbound::Text(raw)) {
            Ok(()) => {
                self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.stats
                    .backpressure_drops
                    .fetch_add(1, Ordering::Relaxed);
                warn!(%client_id, "client send buffer full; dropped relay message");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn close_client(&self, client_id: &str, code: u16, reason: &str) {
        if let Some(client) = self.clients.get(client_id) {
            let _ = client.tx.try_send(Outbound::Close {
                code,
                reason: reason.to_string(),
            });
        }
    }
}

fn serialize_value(value: Value) -> Arc<str> {
    Arc::from(serde_json::to_string(&value).expect("relay messages must serialize"))
}

fn update_max_atomic(target: &AtomicU64, candidate: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while candidate > current {
        match target.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

fn normalize_channel_name(raw: &str) -> Option<String> {
    let channel = raw.trim();
    if !(3..=64).contains(&channel.len()) {
        return None;
    }

    if channel
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Some(channel.to_string())
    } else {
        None
    }
}

fn validate_command(envelope: &BridgeEnvelope) -> Option<String> {
    let command = envelope.command()?;
    if BLOCKED_COMMANDS.contains(&command) {
        return Some(format!(
            "\"{command}\" is stateful and blocked by the relay. Use parentId on creation commands instead."
        ));
    }

    if CREATION_COMMANDS.contains(&command) {
        let has_parent_id = envelope
            .message
            .as_ref()
            .and_then(|message| message.get("params"))
            .and_then(|params| params.get("parentId"))
            .is_some_and(|parent_id| !parent_id.is_null());

        if !has_parent_id {
            return Some(format!(
                "\"{command}\" requires parentId so writes land on the intended page or frame."
            ));
        }
    }

    None
}
