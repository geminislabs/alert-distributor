use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

pub type OrganizationId = Uuid;
pub type UserId = Uuid;
pub type UnitId = Uuid;

type PermissionKey = (OrganizationId, UserId);

#[derive(Clone, Default)]
pub struct PermissionCache {
    inner: HashMap<PermissionKey, Arc<Vec<UnitId>>>,
}

impl PermissionCache {
    pub fn new(inner: HashMap<PermissionKey, Arc<Vec<UnitId>>>) -> Self {
        Self { inner }
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
}
