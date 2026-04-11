use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::FromRow;

use crate::db::postgres::DbPool;
use crate::errors::AppResult;

use super::cache::{OrganizationId, PermissionCache, UnitId, UserId};

type PermissionKey = (OrganizationId, UserId);

#[derive(Debug, Clone, FromRow)]
struct PermissionRow {
    organization_id: OrganizationId,
    user_id: UserId,
    unit_id: UnitId,
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

fn cache_from_rows(rows: Vec<PermissionRow>) -> PermissionCache {
    let mut grouped: HashMap<PermissionKey, HashSet<UnitId>> = HashMap::new();

    for row in rows {
        grouped
            .entry((row.organization_id, row.user_id))
            .or_default()
            .insert(row.unit_id);
    }

    let mut cache = HashMap::with_capacity(grouped.len());
    for (key, units) in grouped {
        let mut unit_ids = units.into_iter().collect::<Vec<UnitId>>();
        unit_ids.sort_unstable();
        cache.insert(key, Arc::new(unit_ids));
    }

    PermissionCache::new(cache)
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
