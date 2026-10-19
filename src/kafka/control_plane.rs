//! Consumidor de `control-plane-events` (ADR-008 de siscom-admin-api).
//!
//! El contrato vive en siscom-admin-api (`docs/architecture/eventos-de-control.md`);
//! lo que hace este servicio con él está en `docs/eventos-de-control.md`.
//!
//! El arranque tiene tres pasos y el orden importa:
//!
//! 1. `prepare_control_plane_consumer` anota el final de cada partición. Si no
//!    puede leer el topic, el proceso no arranca.
//! 2. `main` carga la foto de las vistas `control_plane.*`.
//! 3. `run_control_plane_consumer` consume desde lo anotado en 1, sin grupo y
//!    sin commit: cada arranque empieza de su foto. Lo publicado entre 1 y 2 se
//!    aplica dos veces, lo cual no hace daño porque cada evento trae el estado
//!    completo.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::{Offset, TopicPartitionList};
use serde::Deserialize;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::permissions::cache::ControlPlaneCache;
use crate::sns::models::UserDevice;

/// Versión del `data` de cada entidad que este servicio entiende.
const SCHEMA_VERSION: u32 = 1;

/// Cada cuánto se pregunta al broker por el final del topic.
const PROBE_INTERVAL: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Salud
// ---------------------------------------------------------------------------

/// Estado del consumidor de control para `/health`.
///
/// Un topic de control puede pasar horas sin mensajes, así que «recibí algo»
/// no sirve para saber si se recuperó de una caída: el sondeo al broker de cada
/// `PROBE_INTERVAL` es lo que vuelve a poner `ok` y mide el atraso.
#[derive(Default)]
pub struct ControlPlaneHealth {
    ok: AtomicBool,
    applied_offset: AtomicI64,
    lag: AtomicI64,
}

impl ControlPlaneHealth {
    pub fn is_ok(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }

    pub fn applied_offset(&self) -> i64 {
        self.applied_offset.load(Ordering::Relaxed)
    }

    pub fn lag(&self) -> i64 {
        self.lag.load(Ordering::Relaxed)
    }

    fn mark_ok(&self, lag: i64) {
        self.lag.store(lag, Ordering::Relaxed);
        self.ok.store(true, Ordering::Relaxed);
    }

    fn mark_applied(&self, offset: i64) {
        self.applied_offset.store(offset, Ordering::Relaxed);
        self.ok.store(true, Ordering::Relaxed);
    }

    fn mark_error(&self) {
        self.ok.store(false, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Los eventos
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
enum EventType {
    Upsert,
    Delete,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    event_type: EventType,
    entity: String,
    #[serde(default)]
    schema_version: Option<u32>,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct UserData {
    id: Uuid,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
struct MembershipData {
    organization_id: Uuid,
    user_id: Uuid,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize)]
struct UserUnitData {
    #[serde(default)]
    organization_id: Option<Uuid>,
    user_id: Uuid,
    unit_id: Uuid,
}

#[derive(Deserialize)]
struct UnitDeviceData {
    device_id: String,
    #[serde(default)]
    unit_id: Option<Uuid>,
    #[serde(default)]
    organization_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct UserDeviceData {
    id: Uuid,
    #[serde(default)]
    user_id: Option<Uuid>,
    #[serde(default)]
    device_token: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    endpoint_arn: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
}

/// Qué hizo `apply_event` con un mensaje. Sirve a los tests y al log.
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Yes(&'static str),
    UnknownEntity,
    UnknownVersion,
    Malformed,
}

/// Aplica un mensaje del topic al estado. Nunca falla: un mensaje que no se
/// entiende se registra y se salta, para no detener el consumo.
pub async fn apply_event(cache: &ControlPlaneCache, payload: &[u8]) -> Applied {
    let Ok(envelope) = serde_json::from_slice::<Envelope>(payload) else {
        return Applied::Malformed;
    };
    if envelope.schema_version.is_some_and(|v| v != SCHEMA_VERSION) {
        return Applied::UnknownVersion;
    }
    let upsert = envelope.event_type == EventType::Upsert;

    macro_rules! data {
        ($t:ty) => {
            match serde_json::from_value::<$t>(envelope.data) {
                Ok(data) => data,
                Err(_) => return Applied::Malformed,
            }
        };
    }

    match envelope.entity.as_str() {
        "user" => {
            let d = data!(UserData);
            // Los usuarios no se borran por API; un DELETE se trata como baja.
            let status = if upsert {
                d.status.unwrap_or_default()
            } else {
                String::new()
            };
            cache.apply_user(d.id, &status).await;
            Applied::Yes("user")
        }
        "organization_user" => {
            let d = data!(MembershipData);
            if upsert {
                cache
                    .upsert_membership(
                        d.organization_id,
                        d.user_id,
                        d.role.as_deref().unwrap_or_default(),
                        d.status.as_deref().unwrap_or_default(),
                    )
                    .await;
            } else {
                cache.delete_membership(d.organization_id, d.user_id).await;
            }
            Applied::Yes("organization_user")
        }
        "user_unit" => {
            let d = data!(UserUnitData);
            match (upsert, d.organization_id) {
                (true, Some(org)) => cache.upsert_user_unit(org, d.user_id, d.unit_id).await,
                (true, None) => return Applied::Malformed,
                (false, _) => cache.delete_user_unit(d.user_id, d.unit_id).await,
            }
            Applied::Yes("user_unit")
        }
        "unit_device" => {
            let d = data!(UnitDeviceData);
            match (upsert, d.unit_id, d.organization_id) {
                (true, Some(unit), Some(org)) => {
                    cache.upsert_unit_device(&d.device_id, unit, org).await
                }
                (true, _, _) => return Applied::Malformed,
                (false, _, _) => cache.delete_unit_device(&d.device_id).await,
            }
            Applied::Yes("unit_device")
        }
        "user_device" => {
            let d = data!(UserDeviceData);
            if !upsert {
                cache.deactivate_device(d.id).await;
                return Applied::Yes("user_device");
            }
            let Some(user_id) = d.user_id else {
                return Applied::Malformed;
            };
            cache
                .upsert_user_device(UserDevice {
                    id: d.id,
                    user_id,
                    device_token: d.device_token.unwrap_or_default(),
                    platform: d.platform.unwrap_or_default(),
                    endpoint_arn: d.endpoint_arn.unwrap_or_default(),
                    is_active: d.is_active.unwrap_or(false),
                })
                .await;
            Applied::Yes("user_device")
        }
        _ => Applied::UnknownEntity,
    }
}

// ---------------------------------------------------------------------------
// El consumidor
// ---------------------------------------------------------------------------

/// Dónde empieza a leer el consumidor: el final de cada partición, anotado
/// antes de la foto.
pub struct ControlPlaneStart {
    consumer: StreamConsumer,
    topic: String,
    offsets: Vec<(i32, i64)>,
}

/// Prepara el consumidor y anota los offsets de partida.
///
/// Falla si no puede leer el topic —no existe o falta la ACL—, y `main` no
/// arranca (ADR-008 §9). Los consumidores que había antes registraban
/// `UnknownTopicOrPartition` y seguían como si nada.
pub fn prepare_control_plane_consumer(config: &AppConfig) -> AppResult<ControlPlaneStart> {
    let topic = config.kafka_control_plane_topic.clone();
    // Sin commits ni suscripción: el grupo solo existe porque librdkafka lo pide
    // para crear un consumidor; nunca se registra en el coordinador.
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &config.kafka_brokers)
        .set(
            "group.id",
            format!("{}-control-plane", config.kafka_group_id),
        )
        .set("client.id", "alert-distributor-control-plane")
        .set("enable.auto.commit", "false")
        .set("security.protocol", &config.kafka_security_protocol)
        .set("sasl.mechanism", &config.kafka_sasl_mechanism)
        .set("sasl.username", &config.kafka_username)
        .set("sasl.password", &config.kafka_password)
        .create()?;

    let timeout = Duration::from_secs(10);
    let metadata = consumer
        .fetch_metadata(Some(&topic), timeout)
        .map_err(|err| AppError::ControlPlaneUnavailable(format!("{topic}: {err}")))?;
    let mut partitions = Vec::new();
    for t in metadata.topics().iter().filter(|t| t.name() == topic) {
        if let Some(err) = t.error() {
            return Err(AppError::ControlPlaneUnavailable(format!(
                "{topic}: {err:?}; does the topic exist and does this user have READ?"
            )));
        }
        partitions.extend(t.partitions().iter().map(|p| p.id()));
    }
    if partitions.is_empty() {
        return Err(AppError::ControlPlaneUnavailable(format!(
            "{topic}: no readable partitions; does the topic exist and does this user have READ?"
        )));
    }

    let mut offsets = Vec::with_capacity(partitions.len());
    for partition in partitions {
        let (_, high) = consumer
            .fetch_watermarks(&topic, partition, timeout)
            .map_err(|err| {
                AppError::ControlPlaneUnavailable(format!("{topic}[{partition}]: {err}"))
            })?;
        offsets.push((partition, high));
    }

    Ok(ControlPlaneStart {
        consumer,
        topic,
        offsets,
    })
}

pub async fn run_control_plane_consumer(
    start: ControlPlaneStart,
    cache: Arc<ControlPlaneCache>,
    health: Arc<ControlPlaneHealth>,
) -> AppResult<()> {
    let ControlPlaneStart {
        consumer,
        topic,
        offsets,
    } = start;

    let mut assignment = TopicPartitionList::new();
    for (partition, offset) in &offsets {
        assignment.add_partition_offset(&topic, *partition, Offset::Offset(*offset))?;
    }
    consumer.assign(&assignment)?;
    let mut next: HashMap<i32, i64> = offsets.iter().copied().collect();
    health.mark_ok(0);
    info!(topic = %topic, offsets = ?offsets, "control_plane_consumer_started");

    let mut probe = tokio::time::interval(PROBE_INTERVAL);
    probe.tick().await;
    let mut stream = consumer.stream();

    loop {
        tokio::select! {
            _ = probe.tick() => {
                match lag(&consumer, &topic, &next) {
                    Ok(lag) => health.mark_ok(lag),
                    Err(err) => {
                        health.mark_error();
                        warn!(error = %err, topic = %topic, "control_plane_probe_failed");
                    }
                }
            }
            message = stream.next() => {
                let Some(message) = message else { break };
                match message {
                    Ok(message) => {
                        let applied = match message.payload() {
                            Some(payload) => apply_event(&cache, payload).await,
                            None => Applied::Malformed,
                        };
                        match applied {
                            Applied::Yes(entity) => debug!(
                                entity,
                                offset = message.offset(),
                                "control_plane_event_applied"
                            ),
                            Applied::UnknownEntity => {}
                            other => warn!(
                                result = ?other,
                                offset = message.offset(),
                                "control_plane_event_skipped"
                            ),
                        }
                        next.insert(message.partition(), message.offset() + 1);
                        health.mark_applied(message.offset());
                    }
                    Err(err) => {
                        health.mark_error();
                        error!(error = %err, topic = %topic, "control_plane_receive_failed");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Mensajes publicados que aún no se aplicaron, sumando particiones.
fn lag(consumer: &StreamConsumer, topic: &str, next: &HashMap<i32, i64>) -> AppResult<i64> {
    let mut total = 0;
    for (partition, next_offset) in next {
        // Llamada bloqueante de librdkafka; corta, y el runtime es multihilo.
        let (_, high) = tokio::task::block_in_place(|| {
            consumer.fetch_watermarks(topic, *partition, Duration::from_secs(5))
        })?;
        total += (high - next_offset).max(0);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn evento(event_type: &str, entity: &str, data: serde_json::Value) -> Vec<u8> {
        json!({
            "schema_version": 1,
            "event_id": Uuid::new_v4(),
            "event_type": event_type,
            "entity": entity,
            "timestamp": "2026-10-19T00:00:00Z",
            "organization_id": null,
            "data": data,
        })
        .to_string()
        .into_bytes()
    }

    #[tokio::test]
    async fn la_secuencia_de_admin_api_produce_destinatarios() {
        let cache = ControlPlaneCache::default();
        let (org, unit, owner, member) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );

        for (t, e, d) in [
            (
                "UPSERT",
                "unit_device",
                json!({"device_id": "111", "unit_id": unit, "organization_id": org}),
            ),
            (
                "UPSERT",
                "organization_user",
                json!({"organization_id": org, "user_id": owner, "role": "owner", "status": "ACTIVE"}),
            ),
            ("UPSERT", "user", json!({"id": owner, "status": "ACTIVE"})),
            (
                "UPSERT",
                "organization_user",
                json!({"organization_id": org, "user_id": member, "role": "member", "status": "ACTIVE"}),
            ),
            (
                "UPSERT",
                "user_unit",
                json!({"organization_id": org, "user_id": member, "unit_id": unit, "role": "viewer"}),
            ),
        ] {
            assert!(matches!(
                apply_event(&cache, &evento(t, e, d)).await,
                Applied::Yes(_)
            ));
        }

        let mut esperados = vec![(org, owner), (org, member)];
        esperados.sort_unstable();
        assert_eq!(cache.users_for_unit(unit).await, esperados);

        // Pausa del miembro y DELETE de la asignación del equipo.
        apply_event(
            &cache,
            &evento("UPSERT", "organization_user", json!({"organization_id": org, "user_id": member, "role": "member", "status": "INACTIVE"})),
        )
        .await;
        assert_eq!(cache.users_for_unit(unit).await, vec![(org, owner)]);

        apply_event(
            &cache,
            &evento("DELETE", "unit_device", json!({"device_id": "111"})),
        )
        .await;
        assert!(cache.users_for_unit(unit).await.is_empty());
    }

    #[tokio::test]
    async fn el_dispositivo_desactivado_es_un_upsert_y_sale() {
        let cache = ControlPlaneCache::default();
        let (id, user) = (Uuid::new_v4(), Uuid::new_v4());
        let datos = |activo: bool| {
            json!({
                "id": id, "user_id": user, "device_token": "t", "platform": "ios",
                "endpoint_arn": "arn:aws:sns:us-east-1:1:endpoint/GCM/app/x",
                "is_active": activo, "updated_at": "2026-10-19T00:00:00Z"
            })
        };

        apply_event(&cache, &evento("UPSERT", "user_device", datos(true))).await;
        assert_eq!(cache.devices_for(user).await.len(), 1);
        apply_event(&cache, &evento("UPSERT", "user_device", datos(false))).await;
        assert!(cache.devices_for(user).await.is_empty());
    }

    #[tokio::test]
    async fn lo_que_no_se_entiende_se_salta_sin_tocar_nada() {
        let cache = ControlPlaneCache::default();

        assert_eq!(apply_event(&cache, b"no es json").await, Applied::Malformed);
        assert_eq!(
            apply_event(&cache, &evento("UPSERT", "geofence", json!({}))).await,
            Applied::UnknownEntity
        );

        let mut v2: serde_json::Value = serde_json::from_slice(&evento(
            "UPSERT",
            "user",
            json!({"id": Uuid::new_v4(), "status": "ACTIVE"}),
        ))
        .unwrap();
        v2["schema_version"] = json!(2);
        assert_eq!(
            apply_event(&cache, v2.to_string().as_bytes()).await,
            Applied::UnknownVersion
        );
        assert_eq!(cache.counts().await.users, 0);
    }
}
