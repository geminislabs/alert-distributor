use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::FromRow;

use crate::db::postgres::DbPool;
use crate::errors::AppResult;
use crate::sns::models::UserDevice;

use super::cache::{DeviceId, OrganizationId, PermissionCache, UnitId, UserDevicesCache, UserId};

type PermissionKey = (OrganizationId, UserId);

#[derive(Debug, Clone, FromRow)]
struct PermissionRow {
    organization_id: OrganizationId,
    user_id: UserId,
    unit_id: UnitId,
}

#[derive(Debug, Clone, FromRow)]
struct DeviceRow {
    organization_id: OrganizationId,
    user_id: UserId,
    device_id: DeviceId,
    device_token: String,
    platform: String,
    endpoint_arn: Option<String>,
    is_active: bool,
}

pub async fn load_permission_snapshot(pool: &DbPool) -> AppResult<PermissionCache> {
    let rows = sqlx::query_as::<_, PermissionRow>(
        r#"
        SELECT DISTINCT
            u.organization_id,
            ou.user_id,
            u.id AS unit_id
        FROM units u
        LEFT JOIN unit_devices ud
            ON ud.unit_id = u.id
           AND ud.unassigned_at IS NULL
        LEFT JOIN devices d
            ON d.device_id = ud.device_id
        INNER JOIN organizations o
            ON o.id = u.organization_id
        INNER JOIN organization_users ou
            ON o.id = ou.organization_id
        WHERE u.deleted_at IS NULL
          AND ud.is_active = TRUE
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(cache_from_rows(rows))
}

pub async fn load_user_devices_snapshot(pool: &DbPool) -> AppResult<UserDevicesCache> {
    let rows = sqlx::query_as::<_, DeviceRow>(
        r#"
        SELECT
            ou.organization_id,
            ud.user_id,
            ud.id AS device_id,
            ud.device_token,
            ud.platform,
            ud.endpoint_arn,
            ud.is_active
        FROM user_devices ud
        INNER JOIN organization_users ou
            ON ud.user_id = ou.user_id
        WHERE ud.is_active = TRUE
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(device_cache_from_rows(rows))
}

fn cache_from_rows(rows: Vec<PermissionRow>) -> PermissionCache {
    let mut grouped: HashMap<PermissionKey, HashSet<UnitId>> = HashMap::new();
    let mut by_unit_grouped: HashMap<UnitId, HashSet<PermissionKey>> = HashMap::new();

    for row in rows {
        grouped
            .entry((row.organization_id, row.user_id))
            .or_default()
            .insert(row.unit_id);

        by_unit_grouped
            .entry(row.unit_id)
            .or_default()
            .insert((row.organization_id, row.user_id));
    }

    let mut cache = HashMap::with_capacity(grouped.len());
    for (key, units) in grouped {
        let mut unit_ids = units.into_iter().collect::<Vec<UnitId>>();
        unit_ids.sort_unstable();
        cache.insert(key, Arc::new(unit_ids));
    }

    let mut by_unit = HashMap::with_capacity(by_unit_grouped.len());
    for (unit_id, users) in by_unit_grouped {
        let mut allowed_users = users.into_iter().collect::<Vec<PermissionKey>>();
        allowed_users.sort_unstable();
        by_unit.insert(unit_id, Arc::new(allowed_users));
    }

    PermissionCache::new(cache, by_unit)
}

fn device_cache_from_rows(rows: Vec<DeviceRow>) -> UserDevicesCache {
    let mut grouped: HashMap<PermissionKey, Vec<UserDevice>> = HashMap::new();

    for row in rows {
        let device = UserDevice {
            id: row.device_id,
            user_id: row.user_id,
            device_token: row.device_token,
            platform: row.platform,
            endpoint_arn: row.endpoint_arn.unwrap_or_default(),
            is_active: row.is_active,
        };

        grouped
            .entry((row.organization_id, row.user_id))
            .or_default()
            .push(device);
    }

    let mut cache = HashMap::with_capacity(grouped.len());
    for (key, devices) in grouped {
        cache.insert(key, Arc::new(devices));
    }

    UserDevicesCache::new(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_builder_deduplicates_units() {
        let org = OrganizationId::new_v4();
        let user = UserId::new_v4();
        let unit = UnitId::new_v4();

        let rows = vec![
            PermissionRow {
                organization_id: org,
                user_id: user,
                unit_id: unit,
            },
            PermissionRow {
                organization_id: org,
                user_id: user,
                unit_id: unit,
            },
        ];

        let cache = cache_from_rows(rows);
        let units = cache.units_for(org, user).expect("units should exist");

        assert_eq!(units.len(), 1);
        assert_eq!(units[0], unit);
    }
}
