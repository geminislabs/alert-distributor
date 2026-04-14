use std::sync::Arc;

use tracing::debug;
use uuid::Uuid;

use crate::models::alert_event::AlertEvent;
use crate::permissions::cache::{PermissionCache, UserDevicesCache};

use super::publisher::SnsBroadcaster;

#[derive(Clone)]
pub struct SnsDispatcher {
    permission_cache: Arc<PermissionCache>,
    user_devices_cache: Arc<UserDevicesCache>,
    broadcaster: Arc<SnsBroadcaster>,
}

impl SnsDispatcher {
    pub fn new(
        permission_cache: Arc<PermissionCache>,
        user_devices_cache: Arc<UserDevicesCache>,
        broadcaster: Arc<SnsBroadcaster>,
    ) -> Self {
        Self {
            permission_cache,
            user_devices_cache,
            broadcaster,
        }
    }

    pub async fn dispatch_event(&self, event: &AlertEvent) {
        let Ok(unit_id) = Uuid::parse_str(&event.unit_id) else {
            debug!(unit_id = %event.unit_id, "sns_event_with_invalid_unit_id_skipped");
            return;
        };

        let mut sns_targets = Vec::new();
        if let Some(users) = self.permission_cache.users_for_unit(unit_id) {
            for (organization_id, user_id) in users.iter() {
                if let Some(devices) = self
                    .user_devices_cache
                    .devices_for(*organization_id, *user_id)
                    .await
                {
                    for device in devices.iter() {
                        if device.is_active && !device.endpoint_arn.is_empty() {
                            sns_targets.push(device.clone());
                        }
                    }
                }
            }
        }

        if sns_targets.is_empty() {
            debug!(unit_id = %event.unit_id, "no_sns_targets_for_unit");
            return;
        }

        if let Err(err) = self.broadcaster.send_alert(event, sns_targets).await {
            debug!(
                unit_id = %event.unit_id,
                error = %err,
                "sns_broadcast_enqueue_failed"
            );
        }
    }
}
