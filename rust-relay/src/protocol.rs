use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum Outbound {
    Text(Arc<str>),
    Close { code: u16, reason: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct BridgeEnvelope {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(rename = "sessionId", default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub message: Option<Value>,
}

impl BridgeEnvelope {
    pub fn message_id(&self) -> Option<&str> {
        self.message
            .as_ref()
            .and_then(|message| message.get("id"))
            .and_then(Value::as_str)
            .or(self.id.as_deref())
    }

    pub fn command(&self) -> Option<&str> {
        self.message
            .as_ref()
            .and_then(|message| message.get("command"))
            .and_then(Value::as_str)
    }

    pub fn is_response(&self) -> bool {
        self.message.as_ref().is_some_and(|message| {
            message.get("result").is_some() || message.get("error").is_some()
        })
    }
}

pub fn system_text(message: impl Into<String>) -> Value {
    json!({
        "type": "system",
        "message": message.into()
    })
}

pub fn system_joined(channel: &str) -> Value {
    json!({
        "type": "system",
        "message": format!("Joined channel: {channel}"),
        "channel": channel
    })
}

pub fn system_join_result(id: Option<&str>, channel: &str) -> Value {
    let mut message = serde_json::Map::new();
    if let Some(id) = id {
        message.insert("id".to_string(), json!(id));
    }
    message.insert(
        "result".to_string(),
        json!(format!("Connected to channel: {channel}")),
    );

    json!({
        "type": "system",
        "message": Value::Object(message),
        "channel": channel
    })
}

pub fn error_text(message: impl Into<String>) -> Value {
    json!({
        "type": "error",
        "message": message.into()
    })
}

pub fn command_error_broadcast(
    request_id: &str,
    channel: &str,
    message: impl Into<String>,
) -> Value {
    json!({
        "type": "broadcast",
        "message": {
            "id": request_id,
            "error": message.into()
        },
        "sender": "You",
        "channel": channel
    })
}

pub fn broadcast_message(message: Value, channel: &str, sender: &str) -> Value {
    json!({
        "type": "broadcast",
        "message": message,
        "sender": sender,
        "channel": channel
    })
}

pub fn queue_position(request_id: &str, position: usize, queue_size: usize) -> Value {
    json!({
        "type": "queue_position",
        "id": request_id,
        "position": position,
        "queueSize": queue_size,
        "message": {
            "data": {
                "status": "queued",
                "progress": 0,
                "message": format!("Queued at position {position} of {queue_size}")
            }
        }
    })
}
