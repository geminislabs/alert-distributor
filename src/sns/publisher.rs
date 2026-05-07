use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::info;

use crate::models::alert_event::AlertEvent;

use super::metrics::SnsMetrics;
use super::models::UserDevice;

#[derive(Clone)]
pub struct SnsBroadcaster {
    tx: broadcast::Sender<(UserDevice, AlertEvent)>,
    metrics: Arc<SnsMetrics>,
}

impl SnsBroadcaster {
    pub fn new(capacity: usize, metrics: Arc<SnsMetrics>) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx, metrics }
    }

    pub async fn send_alert(
        &self,
        event: &AlertEvent,
        devices: Vec<UserDevice>,
    ) -> Result<(), broadcast::error::SendError<(UserDevice, AlertEvent)>> {
        info!(
            event_id = %event.id,
            target_count = devices.len(),
            receiver_count = self.tx.receiver_count(),
            "sns_broadcast_dispatching"
        );

        for device in devices {
            self.tx.send((device, event.clone()))?;
            self.metrics.inc_enqueued();
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(UserDevice, AlertEvent)> {
        self.tx.subscribe()
    }
}
