use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use super::dispatcher::ClientAlertMessage;

pub type ConnectionId = Uuid;
pub type UnitId = Uuid;
pub type UserId = Uuid;
pub type OrganizationId = Uuid;
type UserKey = (OrganizationId, UserId);

#[derive(Clone)]
pub struct ConnectionContext {
    pub connection_id: ConnectionId,
    pub user_id: UserId,
    pub organization_id: OrganizationId,
    pub unit_ids: Arc<Vec<UnitId>>,
    pub sender: mpsc::Sender<ClientAlertMessage>,
}

#[derive(Default)]
struct RegistryState {
    connections: HashMap<ConnectionId, ConnectionContext>,
    by_unit: HashMap<UnitId, Vec<ConnectionId>>,
    by_user: HashMap<UserKey, Vec<ConnectionId>>,
}

pub struct ConnectionRegistry {
    inner: RwLock<RegistryState>,
}

impl ConnectionRegistry {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(RegistryState::default()),
        }
    }

    pub async fn add_connection(
        &self,
        user_id: UserId,
        organization_id: OrganizationId,
        unit_ids: Arc<Vec<UnitId>>,
        sender: mpsc::Sender<ClientAlertMessage>,
    ) -> ConnectionId {
        let connection_id = Uuid::new_v4();
        let context = ConnectionContext {
            connection_id,
            user_id,
            organization_id,
            unit_ids,
            sender,
        };

        let mut state = self.inner.write().await;

        for unit_id in context.unit_ids.iter() {
            state
                .by_unit
                .entry(*unit_id)
                .or_default()
                .push(connection_id);
        }

        state
            .by_user
            .entry((context.organization_id, context.user_id))
            .or_default()
            .push(connection_id);

        state.connections.insert(connection_id, context);
        connection_id
    }

    pub async fn remove_connection(
        &self,
        connection_id: &ConnectionId,
    ) -> Option<ConnectionContext> {
        let mut state = self.inner.write().await;
        let removed = state.connections.remove(connection_id)?;

        for unit_id in removed.unit_ids.iter() {
            if let Some(connection_ids) = state.by_unit.get_mut(unit_id) {
                connection_ids.retain(|existing| existing != connection_id);
                if connection_ids.is_empty() {
                    state.by_unit.remove(unit_id);
                }
            }
        }

        let user_key = (removed.organization_id, removed.user_id);
        if let Some(connection_ids) = state.by_user.get_mut(&user_key) {
            connection_ids.retain(|existing| existing != connection_id);
            if connection_ids.is_empty() {
                state.by_user.remove(&user_key);
            }
        }

        Some(removed)
    }

    pub async fn senders_for_unit(
        &self,
        unit_id: &UnitId,
    ) -> Vec<(ConnectionId, mpsc::Sender<ClientAlertMessage>)> {
        let state = self.inner.read().await;

        let Some(connection_ids) = state.by_unit.get(unit_id) else {
            return Vec::new();
        };

        let mut targets = Vec::with_capacity(connection_ids.len());
        for connection_id in connection_ids {
            if let Some(context) = state.connections.get(connection_id) {
                targets.push((context.connection_id, context.sender.clone()));
            }
        }

        targets
    }

    pub async fn connections_for_user(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
    ) -> Vec<ConnectionId> {
        let state = self.inner.read().await;
        state
            .by_user
            .get(&(organization_id, user_id))
            .cloned()
            .unwrap_or_default()
    }

    pub async fn connection_count(&self) -> usize {
        let state = self.inner.read().await;
        state.connections.len()
    }
}
