pub mod auth;
pub mod dispatcher;
pub mod handler;
pub mod registry;

use std::sync::Arc;

use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tracing::info;

use crate::errors::AppResult;

use self::handler::{WsServerState, ws_handler};

pub async fn run_server(bind_addr: &str, state: Arc<WsServerState>) -> AppResult<()> {
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);
    let listener = TcpListener::bind(bind_addr).await?;
    info!(bind_addr = %bind_addr, "websocket_server_listening");

    axum::serve(listener, app).await?;
    Ok(())
}
