use aws_sdk_sns::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_sns::operation::publish::{PublishError, PublishOutput};
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
                    let sns_error = classify_sdk_error(&sdk_err);

                    if let SnsError::AuthError(_) | SnsError::PlatformDisabled(_) = &sns_error {
                        error!(error = %sns_error, endpoint_arn, "sns_publish_error_fatal");
                    }

                    if !sns_error.is_retryable() || retries >= self.max_retries {
                        return Err(sns_error);
                    }

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

/// Clasifica un error del SDK por su **tipo**, no por su texto.
///
/// Antes se buscaba `"EndpointDisabled"` en `sdk_err.to_string()`, pero el
/// `Display` de un error de servicio es literalmente `"service error"`: la
/// detección nunca funcionó, ningún endpoint muerto se desactivaba y todos
/// acababan como `Unknown: service error`, reintentados en cada alerta.
fn classify_sdk_error<R>(sdk_err: &SdkError<PublishError, R>) -> SnsError
where
    R: std::fmt::Debug,
{
    match sdk_err.as_service_error() {
        Some(service_err) => classify_publish_error(service_err),
        // Timeout, fallo de conexión, respuesta ilegible: transitorio.
        None => SnsError::NetworkError(DisplayErrorContext(sdk_err).to_string()),
    }
}

fn classify_publish_error(err: &PublishError) -> SnsError {
    let detail = format!(
        "{}: {}",
        err.code().unwrap_or("?"),
        err.message().unwrap_or_default()
    );
    match err {
        PublishError::EndpointDisabledException(_) | PublishError::NotFoundException(_) => {
            SnsError::InvalidEndpoint(detail)
        }
        // SNS responde InvalidParameter cuando el ARN del endpoint ya no existe.
        // El mensaje se lee de la excepción y no de `err.message()`: este último
        // sale de los metadatos de la respuesta y puede venir vacío.
        PublishError::InvalidParameterException(e)
            if e.message()
                .or(err.message())
                .is_some_and(|m| m.to_ascii_lowercase().contains("endpoint")) =>
        {
            SnsError::InvalidEndpoint(detail)
        }
        PublishError::PlatformApplicationDisabledException(_) => SnsError::PlatformDisabled(detail),
        PublishError::AuthorizationErrorException(_)
        | PublishError::InvalidSecurityException(_)
        | PublishError::KmsAccessDeniedException(_) => SnsError::AuthError(detail),
        PublishError::KmsThrottlingException(_) => SnsError::Throttled,
        PublishError::InternalErrorException(_) => SnsError::NetworkError(detail),
        _ if err.code().is_some_and(|c| c.contains("Throttl")) => SnsError::Throttled,
        _ => SnsError::Unknown(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_sns::error::ErrorMetadata;
    use aws_sdk_sns::types::error::{
        EndpointDisabledException, InvalidParameterException, PlatformApplicationDisabledException,
    };

    #[test]
    fn el_endpoint_deshabilitado_se_reconoce_por_su_tipo() {
        let err = PublishError::EndpointDisabledException(
            EndpointDisabledException::builder()
                .message("Endpoint is disabled")
                .build(),
        );
        assert!(matches!(
            classify_publish_error(&err),
            SnsError::InvalidEndpoint(_)
        ));
    }

    #[test]
    fn un_arn_que_ya_no_existe_es_endpoint_invalido_y_otro_parametro_no() {
        let sin_endpoint = PublishError::InvalidParameterException(
            InvalidParameterException::builder()
                .message("Invalid parameter: TargetArn Reason: No endpoint found for the target arn specified")
                .build(),
        );
        assert!(matches!(
            classify_publish_error(&sin_endpoint),
            SnsError::InvalidEndpoint(_)
        ));

        let otro = PublishError::InvalidParameterException(
            InvalidParameterException::builder()
                .message("Invalid parameter: Message too long")
                .build(),
        );
        assert!(matches!(
            classify_publish_error(&otro),
            SnsError::Unknown(_)
        ));
    }

    #[test]
    fn la_aplicacion_deshabilitada_no_desactiva_dispositivos() {
        let err = PublishError::PlatformApplicationDisabledException(
            PlatformApplicationDisabledException::builder()
                .message("Platform application is disabled")
                .build(),
        );
        let clasificado = classify_publish_error(&err);
        assert!(matches!(clasificado, SnsError::PlatformDisabled(_)));
        assert!(!clasificado.is_retryable());
    }

    #[test]
    fn el_throttling_se_reintenta_y_lo_desconocido_no() {
        let throttling = PublishError::generic(
            ErrorMetadata::builder()
                .code("Throttling")
                .message("Rate exceeded")
                .build(),
        );
        assert!(classify_publish_error(&throttling).is_retryable());

        let raro = PublishError::generic(
            ErrorMetadata::builder()
                .code("SomethingNew")
                .message("x")
                .build(),
        );
        let clasificado = classify_publish_error(&raro);
        assert!(!clasificado.is_retryable());
        // El código real queda en el mensaje, no un "service error" sin más.
        assert_eq!(clasificado.to_string(), "Unknown: SomethingNew: x");
    }
}
