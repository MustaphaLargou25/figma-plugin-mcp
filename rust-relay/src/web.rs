use crate::{
    protocol::Outbound,
    state::{AppState, HealthSnapshot, StatusSnapshot},
};
use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use std::{borrow::Cow, sync::Arc};
use tokio::sync::mpsc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{debug, warn};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/status", get(status))
        .route("/", get(ws_handler))
        .fallback(ws_handler)
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

async fn healthz(State(state): State<Arc<AppState>>) -> Json<HealthSnapshot> {
    Json(state.health_snapshot())
}

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusSnapshot> {
    Json(state.status_snapshot().await)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (tx, mut rx) = mpsc::channel::<Outbound>(state.config.client_buffer_size);
    let client_id = state.register_client(tx);
    let (mut sender, mut receiver) = socket.split();

    let writer_client_id = client_id.clone();
    let writer = tokio::spawn(async move {
        while let Some(outbound) = rx.recv().await {
            match outbound {
                Outbound::Text(text) => {
                    if sender.send(Message::Text(text.to_string())).await.is_err() {
                        break;
                    }
                }
                Outbound::Close { code, reason } => {
                    let frame = CloseFrame {
                        code,
                        reason: Cow::Owned(reason),
                    };
                    let _ = sender.send(Message::Close(Some(frame))).await;
                    break;
                }
            }
        }
        debug!(client_id = %writer_client_id, "websocket writer stopped");
    });

    while let Some(message) = receiver.next().await {
        match message {
            Ok(Message::Text(text)) => {
                state.handle_incoming(&client_id, text).await;
            }
            Ok(Message::Binary(bytes)) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => state.handle_incoming(&client_id, text).await,
                Err(_) => warn!(%client_id, "discarded non-utf8 websocket binary message"),
            },
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
            Err(error) => {
                warn!(%client_id, %error, "websocket read error");
                break;
            }
        }
    }

    state.disconnect_client(&client_id).await;
    writer.abort();
}
