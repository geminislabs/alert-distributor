//! La foto de arranque: una lectura de las vistas `control_plane.*`.
//!
//! Son las vistas que siscom-admin-api mantiene como contrato (ADR-008 §4b),
//! no sus tablas. La consulta anterior leía `users.organization_id`; la `036`
//! la renombró y el siguiente arranque habría fallado. Ahora un cambio así
//! rompe la CI de admin-api, no este arranque.

use sqlx::FromRow;
use uuid::Uuid;

use crate::db::postgres::DbPool;
use crate::errors::AppResult;
use crate::sns::models::UserDevice;

use super::cache::ControlPlaneCache;

#[derive(FromRow)]
struct UserRow {
    id: Uuid,
    status: String,
}

#[derive(FromRow)]
struct MembershipRow {
    organization_id: Uuid,
    user_id: Uuid,
    role: String,
    status: String,
}

#[derive(FromRow)]
struct UserUnitRow {
    organization_id: Uuid,
    user_id: Uuid,
    unit_id: Uuid,
}

#[derive(FromRow)]
struct UnitDeviceRow {
    device_id: String,
    unit_id: Uuid,
    organization_id: Uuid,
}

#[derive(FromRow)]
struct UserDeviceRow {
    id: Uuid,
    user_id: Uuid,
    device_token: String,
    platform: String,
    endpoint_arn: Option<String>,
    is_active: bool,
}

pub async fn load_control_plane_snapshot(pool: &DbPool) -> AppResult<ControlPlaneCache> {
    let cache = ControlPlaneCache::default();

    for row in sqlx::query_as::<_, UserRow>("SELECT id, status FROM control_plane.users")
        .fetch_all(pool)
        .await?
    {
        cache.apply_user(row.id, &row.status).await;
    }

    for row in sqlx::query_as::<_, MembershipRow>(
        "SELECT organization_id, user_id, role, status FROM control_plane.organization_users",
    )
    .fetch_all(pool)
    .await?
    {
        cache
            .upsert_membership(row.organization_id, row.user_id, &row.role, &row.status)
            .await;
    }

    for row in sqlx::query_as::<_, UserUnitRow>(
        "SELECT organization_id, user_id, unit_id FROM control_plane.user_units",
    )
    .fetch_all(pool)
    .await?
    {
        cache
            .upsert_user_unit(row.organization_id, row.user_id, row.unit_id)
            .await;
    }

    for row in sqlx::query_as::<_, UnitDeviceRow>(
        "SELECT device_id, unit_id, organization_id FROM control_plane.unit_devices",
    )
    .fetch_all(pool)
    .await?
    {
        cache
            .upsert_unit_device(&row.device_id, row.unit_id, row.organization_id)
            .await;
    }

    for row in sqlx::query_as::<_, UserDeviceRow>(
        "SELECT id, user_id, device_token, platform, endpoint_arn, is_active \
         FROM control_plane.user_devices WHERE is_active",
    )
    .fetch_all(pool)
    .await?
    {
        cache
            .upsert_user_device(UserDevice {
                id: row.id,
                user_id: row.user_id,
                device_token: row.device_token,
                platform: row.platform,
                endpoint_arn: row.endpoint_arn.unwrap_or_default(),
                is_active: row.is_active,
            })
            .await;
    }

    Ok(cache)
}
