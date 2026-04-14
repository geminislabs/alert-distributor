use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

pub type OrganizationId = Uuid;
pub type UserId = Uuid;
pub type UnitId = Uuid;
pub type DeviceId = Uuid;

type PermissionKey = (OrganizationId, UserId);
type UnitPermissionValue = (OrganizationId, UserId);

#[derive(Clone, Default)]
pub struct PermissionCache {
    inner: HashMap<PermissionKey, Arc<Vec<UnitId>>>,
    by_unit: HashMap<UnitId, Arc<Vec<UnitPermissionValue>>>,
}

impl PermissionCache {
    pub fn new(
        inner: HashMap<PermissionKey, Arc<Vec<UnitId>>>,
        by_unit: HashMap<UnitId, Arc<Vec<UnitPermissionValue>>>,
    ) -> Self {
        Self { inner, by_unit }
    }

    pub fn units_for(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
    ) -> Option<Arc<Vec<UnitId>>> {
        self.inner.get(&(organization_id, user_id)).cloned()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn users_for_unit(&self, unit_id: UnitId) -> Option<Arc<Vec<UnitPermissionValue>>> {
        self.by_unit.get(&unit_id).cloned()
    }
}

// New cache for user devices (Phase 2+)
use crate::sns::models::UserDevice;

#[derive(Clone, Default)]
pub struct UserDevicesCache {
    inner: Arc<RwLock<HashMap<PermissionKey, Arc<Vec<UserDevice>>>>>,
}

impl UserDevicesCache {
    pub fn new(inner: HashMap<PermissionKey, Arc<Vec<UserDevice>>>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(inner)),
        }
    }

    pub async fn devices_for(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
    ) -> Option<Arc<Vec<UserDevice>>> {
        let state = self.inner.read().await;
        state.get(&(organization_id, user_id)).cloned()
    }

    pub async fn len(&self) -> usize {
        let state = self.inner.read().await;
        state.len()
    }

    pub async fn deactivate_device(&self, device_id: DeviceId) -> bool {
        let mut state = self.inner.write().await;
        let mut removed = false;

        for devices in state.values_mut() {
            let filtered = devices
                .iter()
                .filter(|device| device.id != device_id)
                .cloned()
                .collect::<Vec<UserDevice>>();

            if filtered.len() != devices.len() {
                *devices = Arc::new(filtered);
                removed = true;
            }
        }

        removed
    }
}
