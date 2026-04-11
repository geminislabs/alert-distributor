use std::env;

use crate::errors::{AppError, AppResult};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub db_host: String,
    pub db_user: String,
    pub db_password: String,
    pub db_port: u16,
    pub db_name: String,
    pub kafka_brokers: String,
    pub kafka_topic: String,
    pub kafka_group_id: String,
    pub rust_log: String,
    pub kafka_sasl_mechanism: String,
    pub kafka_username: String,
    pub kafka_password: String,
    pub kafka_security_protocol: String,
    pub ws_bind_addr: String,
    pub ws_channel_capacity: usize,
    pub ws_heartbeat_interval_secs: u64,
    pub ws_heartbeat_timeout_secs: u64,
    pub jwt_public_key_pem: String,
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        let db_port = get_required_var("DB_PORT")?
            .parse::<u16>()
            .map_err(|_| AppError::InvalidEnvVar("DB_PORT".to_string()))?;

        let ws_channel_capacity = get_required_var("WS_CHANNEL_CAPACITY")?
            .parse::<usize>()
            .map_err(|_| AppError::InvalidEnvVar("WS_CHANNEL_CAPACITY".to_string()))?;

        let ws_heartbeat_interval_secs = get_optional_var("WS_HEARTBEAT_INTERVAL_SECS")
            .unwrap_or_else(|| "30".to_string())
            .parse::<u64>()
            .map_err(|_| AppError::InvalidEnvVar("WS_HEARTBEAT_INTERVAL_SECS".to_string()))?;

        let ws_heartbeat_timeout_secs = get_optional_var("WS_HEARTBEAT_TIMEOUT_SECS")
            .unwrap_or_else(|| "60".to_string())
            .parse::<u64>()
            .map_err(|_| AppError::InvalidEnvVar("WS_HEARTBEAT_TIMEOUT_SECS".to_string()))?;

        if ws_heartbeat_timeout_secs <= ws_heartbeat_interval_secs {
            return Err(AppError::InvalidEnvVar(
                "WS_HEARTBEAT_TIMEOUT_SECS".to_string(),
            ));
        }

        let jwt_public_key_pem = get_required_var("JWT_PUBLIC_KEY_PEM")?.replace("\\n", "\n");

        Ok(Self {
            db_host: get_required_var("DB_HOST")?,
            db_user: get_required_var("DB_USER")?,
            db_password: get_required_var("DB_PASSWORD")?,
            db_port,
            db_name: get_required_var("DB_NAME")?,
            kafka_brokers: get_required_var("KAFKA_BROKERS")?,
            kafka_topic: get_required_var("KAFKA_TOPIC")?,
            kafka_group_id: get_required_var("KAFKA_GROUP_ID")?,
            rust_log: get_required_var("RUST_LOG")?,
            kafka_sasl_mechanism: get_required_var("KAFKA_SASL_MECHANISM")?,
            kafka_username: get_required_var("KAFKA_USERNAME")?,
            kafka_password: get_required_var("KAFKA_PASSWORD")?,
            kafka_security_protocol: get_required_var("KAFKA_SECURITY_PROTOCOL")?,
            ws_bind_addr: get_required_var("WS_BIND_ADDR")?,
            ws_channel_capacity,
            ws_heartbeat_interval_secs,
            ws_heartbeat_timeout_secs,
            jwt_public_key_pem,
        })
    }
}

fn get_required_var(name: &str) -> AppResult<String> {
    env::var(name).map_err(|_| AppError::MissingEnvVar(name.to_string()))
}

fn get_optional_var(name: &str) -> Option<String> {
    env::var(name).ok()
}
