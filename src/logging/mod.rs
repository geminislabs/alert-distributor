use tracing_subscriber::{EnvFilter, fmt};

use crate::errors::{AppError, AppResult};

pub fn init() -> AppResult<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .map_err(|err| AppError::InvalidLogFilter(err.to_string()))?;

    fmt()
        .json()
        .with_env_filter(env_filter)
        .with_current_span(false)
        .with_span_list(false)
        .try_init()
        .map_err(|err| AppError::LoggingInit(err.to_string()))?;

    Ok(())
}
