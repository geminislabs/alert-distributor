use tracing::warn;
use uuid::Uuid;

use crate::db::postgres::DbPool;
use crate::errors::AppResult;

pub async fn mark_device_inactive(pool: &DbPool, device_id: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE user_devices SET is_active = false WHERE id = $1")
        .bind(device_id)
        .execute(pool)
        .await?;

    warn!(device_id = %device_id, "device_marked_inactive_due_to_sns_error");

    Ok(())
}
