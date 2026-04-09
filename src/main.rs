mod config;
mod errors;
mod kafka;
mod logging;
mod models {
    pub mod alert_event;
}

use config::AppConfig;
use errors::AppResult;
use kafka::consumer::AlertsConsumer;
use tracing::{error, info};

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();
    logging::init()?;

    let config = AppConfig::from_env()?;
    let consumer = AlertsConsumer::new(&config)?;

    info!(
        brokers = %config.kafka_brokers,
        topic = %config.kafka_topic,
        group_id = %config.kafka_group_id,
        rust_log = %config.rust_log,
        "alert_distributor_started"
    );

    tokio::select! {
        result = consumer.run(&config.kafka_topic) => {
            if let Err(err) = result {
                error!(error = %err, "consumer_stopped_with_error");
                return Err(err);
            }
        }
        signal = tokio::signal::ctrl_c() => {
            if let Err(err) = signal {
                error!(error = %err, "failed_to_listen_shutdown_signal");
            } else {
                info!("shutdown_signal_received");
            }
        }
    }

    Ok(())
}
