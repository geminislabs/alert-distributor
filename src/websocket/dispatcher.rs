use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::error::TrySendError;
use tracing::{debug, info};
use uuid::Uuid;

use crate::models::alert_event::AlertEvent;

use super::registry::ConnectionRegistry;

#[derive(Debug, Clone, Serialize)]
pub struct ClientAlertMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub unit_id: String,
    pub data: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AlertDispatcher {
    registry: Arc<ConnectionRegistry>,
}

impl AlertDispatcher {
    pub fn new(registry: Arc<ConnectionRegistry>) -> Self {
        Self { registry }
    }

    pub async fn dispatch_event(&self, event: &AlertEvent) {
        let Ok(unit_id) = Uuid::parse_str(&event.unit_id) else {
            debug!(unit_id = %event.unit_id, "event_with_invalid_unit_id_skipped");
            return;
        };

        let message = ClientAlertMessage {
            message_type: "alert".to_string(),
            unit_id: event.unit_id.clone(),
            data: event.payload.clone(),
            occurred_at: event.occurred_at,
        };

        let targets = self.registry.senders_for_unit(&unit_id).await;
        if targets.is_empty() {
            debug!(unit_id = %event.unit_id, "no_subscribers_for_unit");
            return;
        }

        let mut sent_count = 0usize;
        for (connection_id, sender) in targets {
            match sender.try_send(message.clone()) {
                Ok(()) => {
                    sent_count += 1;
                }
                Err(TrySendError::Full(_)) => {
                    debug!(
                        connection_id = %connection_id,
                        unit_id = %event.unit_id,
                        "dropping_message_due_to_backpressure"
                    );
                }
                Err(TrySendError::Closed(_)) => {
                    let _ = self.registry.remove_connection(&connection_id).await;
                }
            }
        }

        info!(
            unit_id = %event.unit_id,
            sent_count,
            "alert_dispatched_to_connections"
        );
    }
}
