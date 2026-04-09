use std::env;

use crate::errors::{AppError, AppResult};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub kafka_brokers: String,
    pub kafka_topic: String,
    pub kafka_group_id: String,
    pub rust_log: String,
    pub kafka_sasl_mechanism: String,
    pub kafka_username: String,
    pub kafka_password: String,
    pub kafka_security_protocol: String,
}

impl AppConfig {
    pub fn from_env() -> AppResult<Self> {
        Ok(Self {
            kafka_brokers: get_required_var("KAFKA_BROKERS")?,
            kafka_topic: get_required_var("KAFKA_TOPIC")?,
            kafka_group_id: get_required_var("KAFKA_GROUP_ID")?,
            rust_log: get_required_var("RUST_LOG")?,
            kafka_sasl_mechanism: get_required_var("KAFKA_SASL_MECHANISM")?,
            kafka_username: get_required_var("KAFKA_USERNAME")?,
            kafka_password: get_required_var("KAFKA_PASSWORD")?,
            kafka_security_protocol: get_required_var("KAFKA_SECURITY_PROTOCOL")?,
        })
    }
}

fn get_required_var(name: &str) -> AppResult<String> {
    env::var(name).map_err(|_| AppError::MissingEnvVar(name.to_string()))
}
