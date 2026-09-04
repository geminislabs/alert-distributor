use futures::StreamExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::Message;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::db::postgres::DbPool;
use crate::errors::AppResult;
use crate::permissions::cache::{PermissionCache, UserDevicesCache};
use crate::permissions::loader::load_permission_snapshot;
use crate::sns::models::UserDevice;

#[derive(Debug, Deserialize)]
struct ControlEnvelope {
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    entity: String,
    organization_id: Option<Uuid>,
    #[serde(default)]
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct UserDeviceUpdateData {
    id: Uuid,
    user_id: Uuid,
    device_token: String,
    platform: String,
    #[serde(default)]
    endpoint_arn: Option<String>,
    #[serde(default)]
    is_active: bool,
}

pub async fn run_permission_reload_consumer(
    config: AppConfig,
    topic: String,
    group_id: String,
    client_id: &'static str,
    db_pool: DbPool,
    permission_cache: Arc<PermissionCache>,
) -> AppResult<()> {
    let consumer = build_consumer(&config, &group_id, client_id)?;
    consumer.subscribe(&[&topic])?;
    info!(topic = %topic, group_id = %group_id, "control_consumer_subscribed");

    let mut stream = consumer.stream();
    while let Some(result) = stream.next().await {
        match result {
            Ok(message) => {
                if message.payload().is_some() {
                    reload_permissions(&db_pool, &permission_cache, &topic).await;
                } else {
                    warn!(
                        topic = %topic,
                        partition = message.partition(),
                        offset = message.offset(),
                        "empty_control_payload_skipped"
                    );
                }

                if let Err(err) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(
                        error = %err,
                        topic = %topic,
                        partition = message.partition(),
                        offset = message.offset(),
                        "control_commit_failed"
                    );
                }
            }
            Err(err) => {
                warn!(error = %err, topic = %topic, "control_receive_error");
            }
        }
    }

    Ok(())
}

pub async fn run_user_devices_updates_consumer(
    config: AppConfig,
    topic: String,
    group_id: String,
    user_devices_cache: Arc<UserDevicesCache>,
) -> AppResult<()> {
    let consumer = build_consumer(
        &config,
        &group_id,
        "alert-distributor-user-devices-updates",
    )?;
    consumer.subscribe(&[&topic])?;
    info!(topic = %topic, group_id = %group_id, "user_devices_updates_consumer_subscribed");

    let mut stream = consumer.stream();
    while let Some(result) = stream.next().await {
        match result {
            Ok(message) => {
                if let Some(payload) = message.payload() {
                    apply_user_device_update(payload, &user_devices_cache).await;
                } else {
                    warn!(
                        partition = message.partition(),
                        offset = message.offset(),
                        "empty_user_device_update_payload_skipped"
                    );
                }

                if let Err(err) = consumer.commit_message(&message, CommitMode::Async) {
                    warn!(
                        error = %err,
                        partition = message.partition(),
                        offset = message.offset(),
                        "user_devices_update_commit_failed"
                    );
                }
            }
            Err(err) => {
                warn!(error = %err, "user_devices_updates_receive_error");
            }
        }
    }

    Ok(())
}

async fn reload_permissions(
    db_pool: &DbPool,
    permission_cache: &PermissionCache,
    topic: &str,
) {
    match load_permission_snapshot(db_pool).await {
        Ok(snapshot) => {
            let entries = snapshot.len().await;
            permission_cache.replace(snapshot).await;
            info!(topic, entries, "permission_snapshot_reloaded");
        }
        Err(err) => {
            warn!(error = %err, topic, "permission_snapshot_reload_failed");
        }
    }
}

async fn apply_user_device_update(payload: &[u8], user_devices_cache: &UserDevicesCache) {
    let envelope = match serde_json::from_slice::<ControlEnvelope>(payload) {
        Ok(envelope) => envelope,
        Err(err) => {
            warn!(error = %err, "malformed_user_device_update_skipped");
            return;
        }
    };

    if !envelope.entity.is_empty() && !envelope.entity.eq_ignore_ascii_case("user_device") {
        warn!(entity = %envelope.entity, "user_device_update_ignored_unknown_entity");
        return;
    }

    let data = match serde_json::from_value::<UserDeviceUpdateData>(envelope.data) {
        Ok(data) => data,
        Err(err) => {
            warn!(error = %err, "user_device_update_data_invalid");
            return;
        }
    };

    let event_type = envelope.event_type.to_ascii_uppercase();
    if event_type == "DELETE" || !data.is_active {
        let removed = user_devices_cache.deactivate_device(data.id).await;
        info!(
            device_id = %data.id,
            user_id = %data.user_id,
            removed,
            "user_device_removed_from_cache"
        );
        return;
    }

    let Some(organization_id) = envelope.organization_id else {
        warn!(
            device_id = %data.id,
            user_id = %data.user_id,
            "user_device_upsert_missing_organization_id"
        );
        return;
    };

    user_devices_cache
        .upsert(
            organization_id,
            data.user_id,
            UserDevice {
                id: data.id,
                user_id: data.user_id,
                device_token: data.device_token,
                platform: data.platform,
                endpoint_arn: data.endpoint_arn.unwrap_or_default(),
                is_active: data.is_active,
            },
        )
        .await;

    info!(
        device_id = %data.id,
        user_id = %data.user_id,
        organization_id = %organization_id,
        "user_device_upserted_in_cache"
    );
}

fn build_consumer(
    config: &AppConfig,
    group_id: &str,
    client_id: &str,
) -> AppResult<StreamConsumer> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &config.kafka_brokers)
        .set("group.id", group_id)
        .set("client.id", client_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("security.protocol", &config.kafka_security_protocol)
        .set("sasl.mechanism", &config.kafka_sasl_mechanism)
        .set("sasl.username", &config.kafka_username)
        .set("sasl.password", &config.kafka_password)
        .create()?;

    Ok(consumer)
}
