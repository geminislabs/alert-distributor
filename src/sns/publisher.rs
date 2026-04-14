use tokio::sync::broadcast;

use crate::models::alert_event::AlertEvent;

use super::models::UserDevice;

#[derive(Clone)]
pub struct SnsBroadcaster {
    tx: broadcast::Sender<(UserDevice, AlertEvent)>,
}

impl SnsBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub async fn send_alert(
        &self,
        event: &AlertEvent,
        devices: Vec<UserDevice>,
    ) -> Result<(), broadcast::error::SendError<(UserDevice, AlertEvent)>> {
        for device in devices {
            let _ = self.tx.send((device, event.clone()));
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(UserDevice, AlertEvent)> {
        self.tx.subscribe()
    }
}
