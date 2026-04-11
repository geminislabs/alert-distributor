mod config;
mod db;
mod errors;
mod kafka;
mod logging;
mod models;
mod permissions;
mod websocket;

use config::AppConfig;
use db::postgres::connect_pool;
use errors::AppResult;
use kafka::consumer::AlertsConsumer;
use permissions::loader::load_permission_snapshot;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use websocket::auth::JwtValidator;
use websocket::dispatcher::AlertDispatcher;
use websocket::handler::WsServerState;
use websocket::registry::ConnectionRegistry;

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();
    logging::init()?;

    let config = AppConfig::from_env()?;
    let db_pool = connect_pool(&config).await?;
    let permission_cache = Arc::new(load_permission_snapshot(&db_pool).await?);

    let registry = Arc::new(ConnectionRegistry::new());
    let dispatcher = Arc::new(AlertDispatcher::new(registry.clone()));
    let jwt_validator = Arc::new(JwtValidator::new(&config)?);
    let ws_state = Arc::new(WsServerState::new(
        registry,
        jwt_validator,
        permission_cache.clone(),
        config.ws_channel_capacity,
        Duration::from_secs(config.ws_heartbeat_interval_secs),
        Duration::from_secs(config.ws_heartbeat_timeout_secs),
    ));

    let consumer = AlertsConsumer::new(&config, dispatcher.clone())?;

    info!(
        brokers = %config.kafka_brokers,
        topic = %config.kafka_topic,
        group_id = %config.kafka_group_id,
        rust_log = %config.rust_log,
        ws_bind_addr = %config.ws_bind_addr,
        permission_snapshot_entries = permission_cache.len(),
        "alert_distributor_started"
    );

    tokio::select! {
        result = consumer.run(&config.kafka_topic) => {
            if let Err(err) = result {
                error!(error = %err, "consumer_stopped_with_error");
                return Err(err);
            }
        }
        result = websocket::run_server(&config.ws_bind_addr, ws_state) => {
            if let Err(err) = result {
                error!(error = %err, "websocket_server_stopped_with_error");
                return Err(err);
            }
        }
        signal = tokio::signal::ctrl_c() => {
            if let Err(err) = signal {
                error!(error = %err, "failed_to_listen_shutdown_signal");
            } else {
                info!("shutdown_signal_received");
            }
        }
    }

    Ok(())
}
