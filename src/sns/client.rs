use aws_sdk_sns::operation::publish::PublishOutput;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{error, warn};

use super::models::SnsError;
use crate::config::AppConfig;
use crate::errors::AppResult;

#[derive(Clone)]
pub struct SnsClient {
    client: Arc<aws_sdk_sns::Client>,
    max_retries: u32,
}

impl SnsClient {
    pub async fn new(config: &AppConfig) -> AppResult<Self> {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(config.aws_region.clone()))
            .load()
            .await;

        let client = aws_sdk_sns::Client::new(&sdk_config);

        Ok(Self {
            client: Arc::new(client),
            max_retries: config.sns_retry_count,
        })
    }

    pub async fn publish_to_endpoint(
        &self,
        endpoint_arn: &str,
        message: &str,
    ) -> Result<PublishOutput, SnsError> {
        let mut retries = 0;
        let mut base_delay_ms = 100u64;

        loop {
            match self
                .client
                .publish()
                .target_arn(endpoint_arn)
                .message(message)
                .message_structure("json")
                .send()
                .await
            {
                Ok(output) => {
                    return Ok(output);
                }
                Err(sdk_err) => {
                    let err_str = sdk_err.to_string();

                    let sns_error = if err_str.contains("InvalidEndpointException")
                        || err_str.contains("EndpointDisabled")
                    {
                        SnsError::InvalidEndpoint(err_str.clone())
                    } else if err_str.contains("ThrottlingException") {
                        SnsError::Throttled
                    } else if err_str.contains("AuthorizationError")
                        || err_str.contains("AccessDenied")
                    {
                        SnsError::AuthError(err_str.clone())
                    } else if err_str.contains("NetworkingError") {
                        SnsError::NetworkError(err_str.clone())
                    } else {
                        SnsError::Unknown(err_str.clone())
                    };

                    if retries >= self.max_retries {
                        return Err(sns_error);
                    }

                    match &sns_error {
                        SnsError::InvalidEndpoint(_) => {
                            return Err(sns_error);
                        }
                        SnsError::AuthError(_) => {
                            error!(
                                error = %sns_error,
                                endpoint_arn,
                                "sns_auth_error_fatal"
                            );
                            return Err(sns_error);
                        }
                        _ => {
                            warn!(
                                error = %sns_error,
                                retry = retries,
                                max_retries = self.max_retries,
                                delay_ms = base_delay_ms,
                                endpoint_arn,
                                "sns_publish_error_retrying"
                            );

                            sleep(Duration::from_millis(base_delay_ms)).await;
                            base_delay_ms *= 2;
                            retries += 1;
                        }
                    }
                }
            }
        }
    }
}
