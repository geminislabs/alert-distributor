//! Quién recibe las alertas de cada unidad, y a qué dispositivos se mandan.
//!
//! Es el modelo de lectura que este servicio mantiene de `control-plane-events`
//! (ADR-008 de siscom-admin-api): arranca de la foto de las vistas
//! `control_plane.*` y después cada evento cambia solo su entrada. No se vuelve
//! a leer la base.
//!
//! La regla de visibilidad es la del ADR §6, por membresía y no por
//! `users.is_master`:
//!
//! - membresía `owner` → todas las unidades de esa organización;
//! - cualquier otro rol → solo sus concesiones de `user_units` en esa
//!   organización;
//! - nada si la membresía o el usuario no están `ACTIVE`. Reactivar
//!   cualquiera de los dos devuelve el acceso.
//!
//! Una unidad solo cuenta mientras tiene un equipo asignado: sin equipo no
//! genera alertas, y así lo hacía también la consulta anterior.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::sns::models::UserDevice;

pub type OrganizationId = Uuid;
pub type UserId = Uuid;
pub type UnitId = Uuid;
pub type DeviceId = Uuid;

const OWNER: &str = "owner";
const ACTIVE: &str = "ACTIVE";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Membership {
    owner: bool,
    active: bool,
}

#[derive(Default)]
struct State {
    /// `users.status == ACTIVE`. Un usuario que no aparece se da por activo:
    /// la foto trae a todos y cada alta llega con su evento `user`.
    user_active: HashMap<UserId, bool>,
    memberships: HashMap<OrganizationId, HashMap<UserId, Membership>>,
    /// Concesiones: usuario → unidad → organización de la unidad.
    user_units: HashMap<UserId, HashMap<UnitId, OrganizationId>>,
    unit_users: HashMap<UnitId, HashSet<UserId>>,
    /// Equipo (IMEI) → su unidad y la organización de la unidad.
    unit_devices: HashMap<String, (UnitId, OrganizationId)>,
    /// Unidades con equipo, y cuántos equipos tiene cada una.
    units_by_org: HashMap<OrganizationId, HashMap<UnitId, usize>>,
    unit_org: HashMap<UnitId, OrganizationId>,
    /// Solo los dispositivos activos, por persona: un dispositivo es de la
    /// persona, no de una organización.
    user_devices: HashMap<UserId, Vec<UserDevice>>,
}

impl State {
    fn user_is_active(&self, user_id: &UserId) -> bool {
        self.user_active.get(user_id).copied().unwrap_or(true)
    }

    fn active_membership(&self, org: &OrganizationId, user: &UserId) -> Option<&Membership> {
        self.memberships
            .get(org)
            .and_then(|members| members.get(user))
            .filter(|m| m.active && self.user_is_active(user))
    }

    fn unassign_device(&mut self, imei: &str) {
        let Some((unit, org)) = self.unit_devices.remove(imei) else {
            return;
        };
        if let Some(units) = self.units_by_org.get_mut(&org) {
            if let Some(count) = units.get_mut(&unit) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    units.remove(&unit);
                    self.unit_org.remove(&unit);
                }
            }
            if units.is_empty() {
                self.units_by_org.remove(&org);
            }
        }
    }
}

/// El estado de control completo. Barato de clonar: comparte el `Arc`.
#[derive(Clone, Default)]
pub struct ControlPlaneCache {
    state: Arc<RwLock<State>>,
}

impl ControlPlaneCache {
    // ---------------------------------------------------------------- lectura

    /// `(organización, usuario)` que deben recibir las alertas de `unit_id`.
    pub async fn users_for_unit(&self, unit_id: UnitId) -> Vec<(OrganizationId, UserId)> {
        let state = self.state.read().await;
        let Some(org) = state.unit_org.get(&unit_id).copied() else {
            return Vec::new();
        };

        let mut users: HashSet<UserId> = HashSet::new();
        if let Some(members) = state.memberships.get(&org) {
            for (user, membership) in members {
                if membership.owner && membership.active && state.user_is_active(user) {
                    users.insert(*user);
                }
            }
        }
        if let Some(granted) = state.unit_users.get(&unit_id) {
            for user in granted {
                if state
                    .active_membership(&org, user)
                    .is_some_and(|m| !m.owner)
                {
                    users.insert(*user);
                }
            }
        }

        let mut result: Vec<_> = users.into_iter().map(|user| (org, user)).collect();
        result.sort_unstable();
        result
    }

    /// Unidades que `user_id` ve en `organization_id` (el WebSocket se suscribe a ellas).
    pub async fn units_for(&self, organization_id: OrganizationId, user_id: UserId) -> Vec<UnitId> {
        let state = self.state.read().await;
        let Some(membership) = state.active_membership(&organization_id, &user_id) else {
            return Vec::new();
        };

        let mut units: Vec<UnitId> = if membership.owner {
            state
                .units_by_org
                .get(&organization_id)
                .map(|units| units.keys().copied().collect())
                .unwrap_or_default()
        } else {
            state
                .user_units
                .get(&user_id)
                .map(|granted| {
                    granted
                        .iter()
                        .filter(|(unit, org)| {
                            **org == organization_id && state.unit_org.contains_key(unit)
                        })
                        .map(|(unit, _)| *unit)
                        .collect()
                })
                .unwrap_or_default()
        };
        units.sort_unstable();
        units
    }

    pub async fn devices_for(&self, user_id: UserId) -> Vec<UserDevice> {
        let state = self.state.read().await;
        state
            .user_devices
            .get(&user_id)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn counts(&self) -> ControlPlaneCounts {
        let state = self.state.read().await;
        ControlPlaneCounts {
            users: state.user_active.len(),
            memberships: state.memberships.values().map(HashMap::len).sum(),
            user_units: state.user_units.values().map(HashMap::len).sum(),
            unit_devices: state.unit_devices.len(),
            user_devices: state.user_devices.values().map(Vec::len).sum(),
        }
    }

    // ------------------------------------------------------------- escritura

    pub async fn apply_user(&self, user_id: UserId, status: &str) {
        let mut state = self.state.write().await;
        state.user_active.insert(user_id, status == ACTIVE);
    }

    pub async fn upsert_membership(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
        role: &str,
        status: &str,
    ) {
        let mut state = self.state.write().await;
        state
            .memberships
            .entry(organization_id)
            .or_default()
            .insert(
                user_id,
                Membership {
                    owner: role == OWNER,
                    active: status == ACTIVE,
                },
            );
    }

    pub async fn delete_membership(&self, organization_id: OrganizationId, user_id: UserId) {
        let mut state = self.state.write().await;
        if let Some(members) = state.memberships.get_mut(&organization_id) {
            members.remove(&user_id);
            if members.is_empty() {
                state.memberships.remove(&organization_id);
            }
        }
    }

    pub async fn upsert_user_unit(
        &self,
        organization_id: OrganizationId,
        user_id: UserId,
        unit_id: UnitId,
    ) {
        let mut state = self.state.write().await;
        state
            .user_units
            .entry(user_id)
            .or_default()
            .insert(unit_id, organization_id);
        state.unit_users.entry(unit_id).or_default().insert(user_id);
    }

    pub async fn delete_user_unit(&self, user_id: UserId, unit_id: UnitId) {
        let mut state = self.state.write().await;
        if let Some(granted) = state.user_units.get_mut(&user_id) {
            granted.remove(&unit_id);
            if granted.is_empty() {
                state.user_units.remove(&user_id);
            }
        }
        if let Some(users) = state.unit_users.get_mut(&unit_id) {
            users.remove(&user_id);
            if users.is_empty() {
                state.unit_users.remove(&unit_id);
            }
        }
    }

    /// Asignación vigente de un equipo. Reemplaza la anterior si se movió.
    pub async fn upsert_unit_device(
        &self,
        device_id: &str,
        unit_id: UnitId,
        organization_id: OrganizationId,
    ) {
        let mut state = self.state.write().await;
        state.unassign_device(device_id);
        state
            .unit_devices
            .insert(device_id.to_string(), (unit_id, organization_id));
        *state
            .units_by_org
            .entry(organization_id)
            .or_default()
            .entry(unit_id)
            .or_default() += 1;
        state.unit_org.insert(unit_id, organization_id);
    }

    pub async fn delete_unit_device(&self, device_id: &str) {
        let mut state = self.state.write().await;
        state.unassign_device(device_id);
    }

    /// Alta, cambio o baja de un dispositivo móvil. Uno inactivo sale del mapa.
    pub async fn upsert_user_device(&self, device: UserDevice) {
        let mut state = self.state.write().await;
        for devices in state.user_devices.values_mut() {
            devices.retain(|d| d.id != device.id);
        }
        state.user_devices.retain(|_, devices| !devices.is_empty());
        if device.is_active && !device.endpoint_arn.is_empty() {
            state
                .user_devices
                .entry(device.user_id)
                .or_default()
                .push(device);
        }
    }

    /// Lo usa el worker de SNS cuando un endpoint resulta inválido.
    pub async fn deactivate_device(&self, device_id: DeviceId) -> bool {
        let mut state = self.state.write().await;
        let mut removed = false;
        for devices in state.user_devices.values_mut() {
            let before = devices.len();
            devices.retain(|d| d.id != device_id);
            removed |= devices.len() != before;
        }
        state.user_devices.retain(|_, devices| !devices.is_empty());
        removed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPlaneCounts {
    pub users: usize,
    pub memberships: usize,
    pub user_units: usize,
    pub unit_devices: usize,
    pub user_devices: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> Uuid {
        Uuid::new_v4()
    }

    fn device(user_id: UserId, active: bool) -> UserDevice {
        UserDevice {
            id: id(),
            user_id,
            device_token: "token".to_string(),
            platform: "ios".to_string(),
            endpoint_arn: "arn:aws:sns:us-east-1:1:endpoint/GCM/app/x".to_string(),
            is_active: active,
        }
    }

    struct Escena {
        cache: ControlPlaneCache,
        org: OrganizationId,
        unidad: UnitId,
        otra_unidad: UnitId,
        owner: UserId,
        miembro: UserId,
    }

    /// Una organización con dos unidades con equipo, su owner y un miembro con
    /// concesión solo sobre la primera.
    async fn escena() -> Escena {
        let cache = ControlPlaneCache::default();
        let (org, unidad, otra_unidad, owner, miembro) = (id(), id(), id(), id(), id());
        cache.upsert_unit_device("111", unidad, org).await;
        cache.upsert_unit_device("222", otra_unidad, org).await;
        cache.upsert_membership(org, owner, "owner", "ACTIVE").await;
        cache
            .upsert_membership(org, miembro, "member", "ACTIVE")
            .await;
        cache.upsert_user_unit(org, miembro, unidad).await;
        Escena {
            cache,
            org,
            unidad,
            otra_unidad,
            owner,
            miembro,
        }
    }

    #[tokio::test]
    async fn owner_ve_todo_y_el_miembro_solo_lo_concedido() {
        let e = escena().await;

        let mut esperados = vec![(e.org, e.owner), (e.org, e.miembro)];
        esperados.sort_unstable();
        assert_eq!(e.cache.users_for_unit(e.unidad).await, esperados);
        assert_eq!(
            e.cache.users_for_unit(e.otra_unidad).await,
            vec![(e.org, e.owner)]
        );

        let mut todas = vec![e.unidad, e.otra_unidad];
        todas.sort_unstable();
        assert_eq!(e.cache.units_for(e.org, e.owner).await, todas);
        assert_eq!(e.cache.units_for(e.org, e.miembro).await, vec![e.unidad]);
    }

    #[tokio::test]
    async fn pausar_la_membresia_quita_todo_y_reactivarla_lo_devuelve() {
        let e = escena().await;

        e.cache
            .upsert_membership(e.org, e.miembro, "member", "INACTIVE")
            .await;
        assert_eq!(
            e.cache.users_for_unit(e.unidad).await,
            vec![(e.org, e.owner)]
        );
        assert!(e.cache.units_for(e.org, e.miembro).await.is_empty());

        e.cache
            .upsert_membership(e.org, e.miembro, "member", "ACTIVE")
            .await;
        assert_eq!(e.cache.units_for(e.org, e.miembro).await, vec![e.unidad]);
    }

    #[tokio::test]
    async fn dar_de_baja_al_usuario_le_quita_todo_aunque_la_membresia_siga() {
        let e = escena().await;

        e.cache.apply_user(e.owner, "INACTIVE").await;
        assert_eq!(
            e.cache.users_for_unit(e.otra_unidad).await,
            Vec::<(OrganizationId, UserId)>::new()
        );
        assert!(e.cache.units_for(e.org, e.owner).await.is_empty());

        e.cache.apply_user(e.owner, "ACTIVE").await;
        assert_eq!(
            e.cache.users_for_unit(e.otra_unidad).await,
            vec![(e.org, e.owner)]
        );
    }

    #[tokio::test]
    async fn quitar_la_membresia_o_la_concesion() {
        let e = escena().await;

        e.cache.delete_user_unit(e.miembro, e.unidad).await;
        assert!(e.cache.units_for(e.org, e.miembro).await.is_empty());

        e.cache.upsert_user_unit(e.org, e.miembro, e.unidad).await;
        e.cache.delete_membership(e.org, e.miembro).await;
        assert!(e.cache.units_for(e.org, e.miembro).await.is_empty());
        assert_eq!(
            e.cache.users_for_unit(e.unidad).await,
            vec![(e.org, e.owner)]
        );
    }

    #[tokio::test]
    async fn una_concesion_no_cuenta_en_otra_organizacion() {
        // El caso del maestro con dos organizaciones: ser owner en una no da
        // nada en la otra, y una concesión solo vale en la organización de la
        // unidad.
        let e = escena().await;
        let otra_org = id();
        e.cache
            .upsert_membership(otra_org, e.owner, "member", "ACTIVE")
            .await;

        assert!(e.cache.units_for(otra_org, e.owner).await.is_empty());
    }

    #[tokio::test]
    async fn mover_un_equipo_cambia_la_organizacion_de_quien_recibe() {
        let e = escena().await;
        let (otra_org, otro_owner, nueva_unidad) = (id(), id(), id());
        e.cache
            .upsert_membership(otra_org, otro_owner, "owner", "ACTIVE")
            .await;

        e.cache
            .upsert_unit_device("222", nueva_unidad, otra_org)
            .await;

        assert!(e.cache.users_for_unit(e.otra_unidad).await.is_empty());
        assert_eq!(
            e.cache.users_for_unit(nueva_unidad).await,
            vec![(otra_org, otro_owner)]
        );
        assert_eq!(e.cache.units_for(e.org, e.owner).await, vec![e.unidad]);

        e.cache.delete_unit_device("111").await;
        assert!(e.cache.units_for(e.org, e.owner).await.is_empty());
    }

    #[tokio::test]
    async fn los_dispositivos_son_de_la_persona_y_los_inactivos_salen() {
        let e = escena().await;
        let movil = device(e.owner, true);
        e.cache.upsert_user_device(movil.clone()).await;
        assert_eq!(e.cache.devices_for(e.owner).await.len(), 1);

        let mut apagado = movil.clone();
        apagado.is_active = false;
        e.cache.upsert_user_device(apagado).await;
        assert!(e.cache.devices_for(e.owner).await.is_empty());

        e.cache.upsert_user_device(movil.clone()).await;
        assert!(e.cache.deactivate_device(movil.id).await);
        assert!(e.cache.devices_for(e.owner).await.is_empty());
    }
}
