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

use crate::sns::models::UserDevice;

#[derive(Clone, Default)]
struct PermissionMaps {
    inner: HashMap<PermissionKey, Arc<Vec<UnitId>>>,
    by_unit: HashMap<UnitId, Arc<Vec<UnitPermissionValue>>>,
}

#[derive(Clone, Default)]
pub struct PermissionCache {
    state: Arc<RwLock<PermissionMaps>>,
}

impl PermissionCache {
    pub fn new(
        inner: HashMap<PermissionKey, Arc<Vec<UnitId>>>,
        by_unit: HashMap<UnitId, Arc<Vec<UnitPermissionValue>>>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(PermissionMaps { inner, by_unit })),
        }
    }

    pub async fn units_for(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
    ) -> Option<Arc<Vec<UnitId>>> {
        let state = self.state.read().await;
        state.inner.get(&(organization_id, user_id)).cloned()
    }

    pub async fn len(&self) -> usize {
        let state = self.state.read().await;
        state.inner.len()
    }

    pub async fn users_for_unit(&self, unit_id: UnitId) -> Option<Arc<Vec<UnitPermissionValue>>> {
        let state = self.state.read().await;
        state.by_unit.get(&unit_id).cloned()
    }

    pub async fn replace(&self, snapshot: PermissionCache) {
        let maps = snapshot.state.read().await.clone();
        *self.state.write().await = maps;
    }
}

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

    pub async fn upsert(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
        device: UserDevice,
    ) {
        self.deactivate_device(device.id).await;

        let mut state = self.inner.write().await;
        let key = (organization_id, user_id);
        let mut devices = state
            .get(&key)
            .map(|current| current.as_ref().clone())
            .unwrap_or_default();
        devices.push(device);
        state.insert(key, Arc::new(devices));
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
