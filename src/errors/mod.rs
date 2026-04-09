use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("missing required environment variable: {0}")]
    MissingEnvVar(String),

    #[error("invalid log filter from RUST_LOG: {0}")]
    InvalidLogFilter(String),

    #[error("failed to initialize logger: {0}")]
    LoggingInit(String),

    #[error("kafka client error: {0}")]
    Kafka(#[from] rdkafka::error::KafkaError),
}
