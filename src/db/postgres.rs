use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

use crate::config::AppConfig;
use crate::errors::AppResult;

pub type DbPool = PgPool;

pub async fn connect_pool(config: &AppConfig) -> AppResult<DbPool> {
    let options = PgConnectOptions::new()
        .host(&config.db_host)
        .port(config.db_port)
        .username(&config.db_user)
        .password(&config.db_password)
        .database(&config.db_name);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    Ok(pool)
}
