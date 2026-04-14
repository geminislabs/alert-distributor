mod config;
mod db;
mod errors;
mod kafka;
mod logging;
mod models;
mod permissions;
mod sns;
mod websocket;

use config::AppConfig;
use db::postgres::connect_pool;
use errors::AppResult;
use futures::stream::{FuturesUnordered, StreamExt};
use kafka::consumer::AlertsConsumer;
use permissions::loader::{load_permission_snapshot, load_user_devices_snapshot};
use sns::{SnsBroadcaster, SnsClient, SnsDispatcher};
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
    let user_devices_cache = Arc::new(load_user_devices_snapshot(&db_pool).await?);

    // Initialize SNS client and broadcaster
    let sns_broadcaster = Arc::new(SnsBroadcaster::new(1000));
    let _sns_client = if config.sns_enabled {
        Some(SnsClient::new(&config).await?)
    } else {
        None
    };

    // Spawn SNS worker task if enabled
    if config.sns_enabled
        && let Some(sns_client) = _sns_client.clone()
    {
        let db_pool_worker = db_pool.clone();
        let user_devices_cache_worker = user_devices_cache.clone();
        let broadcaster_rx = sns_broadcaster.subscribe();
        let sns_batch_timeout = Duration::from_millis(config.sns_batch_timeout_ms);
        tokio::spawn(async move {
            sns_worker_task(
                broadcaster_rx,
                sns_client,
                db_pool_worker,
                user_devices_cache_worker,
                sns_batch_timeout,
            )
            .await;
        });
    }

    let registry = Arc::new(ConnectionRegistry::new());
    let dispatcher = Arc::new(AlertDispatcher::new(registry.clone()));
    let sns_dispatcher = Arc::new(SnsDispatcher::new(
        permission_cache.clone(),
        user_devices_cache.clone(),
        sns_broadcaster.clone(),
    ));
    let jwt_validator = Arc::new(JwtValidator::new(&config)?);
    let ws_state = Arc::new(WsServerState::new(
        registry,
        jwt_validator,
        permission_cache.clone(),
        config.ws_channel_capacity,
        Duration::from_secs(config.ws_heartbeat_interval_secs),
        Duration::from_secs(config.ws_heartbeat_timeout_secs),
    ));

    let consumer = AlertsConsumer::new(&config, dispatcher.clone(), sns_dispatcher.clone())?;

    info!(
        brokers = %config.kafka_brokers,
        topic = %config.kafka_topic,
        group_id = %config.kafka_group_id,
        rust_log = %config.rust_log,
        ws_bind_addr = %config.ws_bind_addr,
        permission_snapshot_entries = permission_cache.len(),
        user_devices_snapshot_entries = user_devices_cache.len().await,
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

async fn sns_worker_task(
    mut rx: tokio::sync::broadcast::Receiver<(sns::UserDevice, models::alert_event::AlertEvent)>,
    sns_client: SnsClient,
    db_pool: db::postgres::DbPool,
    user_devices_cache: Arc<permissions::cache::UserDevicesCache>,
    batch_timeout: Duration,
) {
    use tracing::warn;

    loop {
        let first_item = match rx.recv().await {
            Ok(item) => item,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!(skipped, "sns_worker_lagged_messages_skipped");
                continue;
            }
        };

        let mut batch = vec![first_item];
        let deadline = tokio::time::Instant::now() + batch_timeout;

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(item)) => batch.push(item),
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped))) => {
                    warn!(skipped, "sns_worker_lagged_messages_skipped");
                }
                Err(_) => break,
            }

            if batch.len() >= 256 {
                break;
            }
        }

        let batch_size = batch.len();
        let mut tasks = FuturesUnordered::new();

        for (device, event) in batch {
            let sns_client = sns_client.clone();
            let db_pool = db_pool.clone();
            let user_devices_cache = user_devices_cache.clone();

            tasks.push(async move {
                process_sns_delivery(device, event, sns_client, db_pool, user_devices_cache).await;
            });
        }

        while tasks.next().await.is_some() {}

        info!(batch_size, "sns_batch_processed");
    }
}

async fn process_sns_delivery(
    device: sns::UserDevice,
    event: models::alert_event::AlertEvent,
    sns_client: SnsClient,
    db_pool: db::postgres::DbPool,
    user_devices_cache: Arc<permissions::cache::UserDevicesCache>,
) {
    use tracing::warn;

    let message = sns::SnsMessage::new(
        &event.rule_id.to_string(),
        &event.unit_id,
        &event.alert_type,
    );
    let payload = message.to_json_payload();

    info!(
        device_id = %device.id,
        endpoint_arn = %device.endpoint_arn,
        event_id = %event.id,
        payload = %payload,
        "sns_message_payload"
    );

    match sns_client
        .publish_to_endpoint(&device.endpoint_arn, &payload)
        .await
    {
        Ok(_) => {
            info!(
                device_id = %device.id,
                endpoint_arn = %device.endpoint_arn,
                event_id = %event.id,
                "sns_message_published_successfully"
            );
        }
        Err(sns::SnsError::InvalidEndpoint(msg)) => {
            warn!(
                device_id = %device.id,
                endpoint_arn = %device.endpoint_arn,
                error = %msg,
                event_id = %event.id,
                "sns_invalid_endpoint_marking_inactive"
            );

            if let Err(err) = sns::db_update::mark_device_inactive(&db_pool, device.id).await {
                warn!(
                    device_id = %device.id,
                    error = %err,
                    "failed_to_mark_device_inactive"
                );
            } else if user_devices_cache.deactivate_device(device.id).await {
                warn!(
                    device_id = %device.id,
                    event_id = %event.id,
                    "sns_invalid_endpoint_removed_from_runtime_cache"
                );
            }
        }
        Err(err) => {
            warn!(
                device_id = %device.id,
                endpoint_arn = %device.endpoint_arn,
                error = %err,
                event_id = %event.id,
                "sns_publish_error"
            );
        }
    }
}
