use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("missing required environment variable: {0}")]
    MissingEnvVar(String),

    #[error("invalid value for environment variable: {0}")]
    InvalidEnvVar(String),

    #[error("invalid log filter from RUST_LOG: {0}")]
    InvalidLogFilter(String),

    #[error("failed to initialize logger: {0}")]
    LoggingInit(String),

    #[error("kafka client error: {0}")]
    Kafka(#[from] rdkafka::error::KafkaError),

    #[error("jwt error: {0}")]
    Jwt(String),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("invalid uuid value: {0}")]
    InvalidUuid(String),

    #[error("http server error: {0}")]
    HttpServer(#[from] std::io::Error),
}
