use futures::StreamExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{BorrowedMessage, Message};
use tracing::{debug, error, info, warn};

use crate::config::AppConfig;
use crate::errors::AppResult;
use crate::models::alert_event::AlertEvent;

pub struct AlertsConsumer {
    consumer: StreamConsumer,
}

impl AlertsConsumer {
    pub fn new(config: &AppConfig) -> AppResult<Self> {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", &config.kafka_brokers)
            .set("group.id", &config.kafka_group_id)
            .set("enable.auto.commit", "false")
            .set("auto.offset.reset", "earliest")
            .set("security.protocol", &config.kafka_security_protocol)
            .set("sasl.mechanism", &config.kafka_sasl_mechanism)
            .set("sasl.username", &config.kafka_username)
            .set("sasl.password", &config.kafka_password)
            .create()?;

        Ok(Self { consumer })
    }

    pub async fn run(&self, topic: &str) -> AppResult<()> {
        self.consumer.subscribe(&[topic])?;
        info!(topic = %topic, "kafka_consumer_subscribed");

        let mut stream = self.consumer.stream();
        while let Some(result) = stream.next().await {
            match result {
                Ok(message) => {
                    self.process_message(message).await;
                }
                Err(err) => {
                    error!(error = %err, "kafka_receive_error");
                }
            }
        }

        warn!("kafka_stream_ended");
        Ok(())
    }

    async fn process_message(&self, message: BorrowedMessage<'_>) {
        let Some(payload) = message.payload() else {
            warn!(
                partition = message.partition(),
                offset = message.offset(),
                "empty_payload_skipped"
            );
            return;
        };

        match serde_json::from_slice::<AlertEvent>(payload) {
            Ok(event) => {
                info!(
                    event_id = %event.id,
                    unit_id = %event.unit_id,
                    organization_id = %event.organization_id,
                    alert_type = %event.alert_type,
                    partition = message.partition(),
                    offset = message.offset(),
                    "alert_event_ingested"
                );

                if let Err(err) = self.consumer.commit_message(&message, CommitMode::Async) {
                    error!(
                        error = %err,
                        event_id = %event.id,
                        partition = message.partition(),
                        offset = message.offset(),
                        "manual_commit_failed"
                    );
                } else {
                    debug!(
                        event_id = %event.id,
                        partition = message.partition(),
                        offset = message.offset(),
                        "manual_commit_succeeded"
                    );
                }
            }
            Err(err) => {
                let payload_preview =
                    std::str::from_utf8(payload).map_or("<non_utf8_payload>", |text| text);

                error!(
                    error = %err,
                    payload = payload_preview,
                    partition = message.partition(),
                    offset = message.offset(),
                    "alert_event_parse_failed"
                );
            }
        }
    }
}
