use std::sync::Arc;

use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::models::alert_event::AlertEvent;
use crate::permissions::cache::{PermissionCache, UserDevicesCache};

use super::metrics::SnsMetrics;
use super::publisher::SnsBroadcaster;

#[derive(Clone)]
pub struct SnsDispatcher {
    permission_cache: Arc<PermissionCache>,
    user_devices_cache: Arc<UserDevicesCache>,
    broadcaster: Arc<SnsBroadcaster>,
    metrics: Arc<SnsMetrics>,
}

impl SnsDispatcher {
    pub fn new(
        permission_cache: Arc<PermissionCache>,
        user_devices_cache: Arc<UserDevicesCache>,
        broadcaster: Arc<SnsBroadcaster>,
        metrics: Arc<SnsMetrics>,
    ) -> Self {
        Self {
            permission_cache,
            user_devices_cache,
            broadcaster,
            metrics,
        }
    }

    pub async fn dispatch_event(&self, event: &AlertEvent) {
        let Ok(unit_id) = Uuid::parse_str(&event.unit_id) else {
            debug!(unit_id = %event.unit_id, "sns_event_with_invalid_unit_id_skipped");
            return;
        };

        let mut sns_targets = Vec::new();
        let mut permitted_user_count = 0usize;
        let mut candidate_device_count = 0usize;
        let mut skipped_empty_endpoint_count = 0usize;
        let mut skipped_unsupported_endpoint_count = 0usize;

        if let Some(users) = self.permission_cache.users_for_unit(unit_id) {
            for (organization_id, user_id) in users.iter() {
                permitted_user_count += 1;

                if let Some(devices) = self
                    .user_devices_cache
                    .devices_for(*organization_id, *user_id)
                    .await
                {
                    for device in devices.iter() {
                        candidate_device_count += 1;

                        if !device.is_active {
                            continue;
                        }

                        if device.endpoint_arn.is_empty() {
                            skipped_empty_endpoint_count += 1;
                            debug!(
                                event_id = %event.id,
                                unit_id = %event.unit_id,
                                organization_id = %organization_id,
                                user_id = %user_id,
                                device_id = %device.id,
                                "sns_target_skipped_empty_endpoint"
                            );
                            continue;
                        }

                        if !device.endpoint_is_supported_for_platform() {
                            skipped_unsupported_endpoint_count += 1;
                            warn!(
                                event_id = %event.id,
                                unit_id = %event.unit_id,
                                organization_id = %organization_id,
                                user_id = %user_id,
                                device_id = %device.id,
                                platform = %device.platform,
                                endpoint_arn = %device.endpoint_arn,
                                endpoint_channel = ?device.endpoint_channel(),
                                "sns_target_has_unsupported_platform_endpoint"
                            );
                            continue;
                        }

                        info!(
                            event_id = %event.id,
                            unit_id = %event.unit_id,
                            organization_id = %organization_id,
                            user_id = %user_id,
                            device_id = %device.id,
                            platform = %device.platform,
                            endpoint_channel = ?device.endpoint_channel(),
                            "sns_target_resolved"
                        );

                        self.metrics.inc_resolved();
                        sns_targets.push(device.clone());
                    }
                } else {
                    debug!(
                        event_id = %event.id,
                        unit_id = %event.unit_id,
                        organization_id = %organization_id,
                        user_id = %user_id,
                        "sns_target_user_has_no_devices"
                    );
                }
            }
        }

        info!(
            event_id = %event.id,
            unit_id = %event.unit_id,
            permitted_user_count,
            candidate_device_count,
            resolved_target_count = sns_targets.len(),
            skipped_empty_endpoint_count,
            skipped_unsupported_endpoint_count,
            "sns_targets_evaluated"
        );

        if sns_targets.is_empty() {
            info!(
                event_id = %event.id,
                unit_id = %event.unit_id,
                "no_sns_targets_for_unit"
            );
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
