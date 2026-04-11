use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::auth::{AuthContext, JwtValidator};
use super::registry::ConnectionRegistry;
use crate::permissions::cache::PermissionCache;

pub struct WsServerState {
    pub registry: Arc<ConnectionRegistry>,
    pub jwt_validator: Arc<JwtValidator>,
    pub permission_cache: Arc<PermissionCache>,
    pub channel_capacity: usize,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
}

impl WsServerState {
    pub fn new(
        registry: Arc<ConnectionRegistry>,
        jwt_validator: Arc<JwtValidator>,
        permission_cache: Arc<PermissionCache>,
        channel_capacity: usize,
        heartbeat_interval: Duration,
        heartbeat_timeout: Duration,
    ) -> Self {
        Self {
            registry,
            jwt_validator,
            permission_cache,
            channel_capacity,
            heartbeat_interval,
            heartbeat_timeout,
        }
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WsServerState>>,
    request: Request,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let auth_context = state
        .jwt_validator
        .validate_bearer(auth_header)
        .map_err(|err| {
            warn!(error = %err, "websocket_auth_failed");
            StatusCode::UNAUTHORIZED
        })?;

    let Some(unit_ids) = state
        .permission_cache
        .units_for(auth_context.organization_id, auth_context.user_id)
        .filter(|unit_ids| !unit_ids.is_empty())
    else {
        warn!(
            user_id = %auth_context.user_id,
            organization_id = %auth_context.organization_id,
            "websocket_authz_failed_no_units"
        );
        return Err(StatusCode::FORBIDDEN);
    };

    info!(
        user_id = %auth_context.user_id,
        organization_id = %auth_context.organization_id,
        "websocket_auth_succeeded"
    );

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, auth_context, unit_ids)))
}

async fn handle_socket(
    socket: WebSocket,
    state: Arc<WsServerState>,
    auth: AuthContext,
    unit_ids: Arc<Vec<Uuid>>,
) {
    let (sender, mut receiver) = mpsc::channel(state.channel_capacity);
    let connection_id = state
        .registry
        .add_connection(auth.user_id, auth.organization_id, unit_ids, sender)
        .await;

    let connections = state.registry.connection_count().await;
    let user_connections = state
        .registry
        .connections_for_user(auth.organization_id, auth.user_id)
        .await
        .len();

    info!(
        connection_id = %connection_id,
        user_id = %auth.user_id,
        organization_id = %auth.organization_id,
        active_connections = connections,
        user_connections,
        "websocket_connection_opened"
    );

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut last_pong = Instant::now();
    let mut heartbeat = tokio::time::interval(state.heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            outbound = receiver.recv() => {
                let Some(message) = outbound else {
                    break;
                };

                let payload = match serde_json::to_string(&message) {
                    Ok(value) => value,
                    Err(err) => {
                        error!(error = %err, "failed_to_serialize_outbound_message");
                        continue;
                    }
                };

                if ws_sender.send(Message::Text(payload)).await.is_err() {
                    warn!(connection_id = %connection_id, "websocket_unexpected_disconnect");
                    break;
                }
            }
            incoming = ws_receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if ws_sender.send(Message::Pong(payload)).await.is_err() {
                            warn!(connection_id = %connection_id, "websocket_unexpected_disconnect");
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = Instant::now();
                    }
                    Some(Ok(Message::Text(_))) => {}
                    Some(Ok(Message::Binary(_))) => {}
                    Some(Err(err)) => {
                        warn!(connection_id = %connection_id, error = %err, "websocket_unexpected_disconnect");
                        break;
                    }
                    None => break,
                }
            }
            _ = heartbeat.tick() => {
                if last_pong.elapsed() >= state.heartbeat_timeout {
                    warn!(connection_id = %connection_id, "websocket_heartbeat_timeout");
                    break;
                }

                if ws_sender.send(Message::Ping(Vec::new())).await.is_err() {
                    warn!(connection_id = %connection_id, "websocket_unexpected_disconnect");
                    break;
                }
            }
        }
    }

    let _ = state.registry.remove_connection(&connection_id).await;

    let connections = state.registry.connection_count().await;
    info!(
        connection_id = %connection_id,
        active_connections = connections,
        "websocket_connection_closed"
    );
}
