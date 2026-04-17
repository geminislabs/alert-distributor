use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type DeviceId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDevice {
    pub id: DeviceId,
    pub user_id: Uuid,
    pub device_token: String,
    pub platform: String, // "ios" | "android"
    pub endpoint_arn: String,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct SnsMessage {
    pub body: String,
    pub title: String,
}

impl SnsMessage {
    pub fn new(title: &str, body: &str) -> Self {
        let title = title.to_string();
        let body = body.to_string();

        Self { body, title }
    }

    pub fn to_json_payload(&self) -> String {
        let gcm_payload = format!(
            r#"{{ "notification": {{ "title": "{}", "body": "{}", "sound": "default" }}, "data": {{ "message": "{}" }} }}"#,
            self.title, self.body, self.body
        );

        format!(
            r#"{{"default":"alert","GCM":"{}"}}"#,
            escape_json(&gcm_payload)
        )
    }
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[derive(Debug, Clone)]
pub enum SnsError {
    InvalidEndpoint(String),
    Throttled,
    AuthError(String),
    NetworkError(String),
    Unknown(String),
}

impl std::fmt::Display for SnsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnsError::InvalidEndpoint(msg) => write!(f, "InvalidEndpoint: {}", msg),
            SnsError::Throttled => write!(f, "Throttled"),
            SnsError::AuthError(msg) => write!(f, "AuthError: {}", msg),
            SnsError::NetworkError(msg) => write!(f, "NetworkError: {}", msg),
            SnsError::Unknown(msg) => write!(f, "Unknown: {}", msg),
        }
    }
}

impl std::error::Error for SnsError {}

#[cfg(test)]
mod tests {
    use super::SnsMessage;

    #[test]
    fn sns_payload_format_is_correct() {
        let message = SnsMessage::new("Ingreso a geocerca", "Camioneta Juan");
        let payload = message.to_json_payload();

        println!("SNS payload generated: {}", payload);

        let expected = r#"{"default":"alert","GCM":"{ \"notification\": { \"title\": \"Ingreso a geocerca\", \"body\": \"Camioneta Juan\", \"sound\": \"default\" }, \"data\": { \"message\": \"Camioneta Juan\" } }"}"#;
        assert_eq!(payload, expected);
    }

    #[test]
    fn sns_payload_allows_empty_body() {
        let message = SnsMessage::new("Motor apagado", "");
        let payload = message.to_json_payload();

        let expected = r#"{"default":"alert","GCM":"{ \"notification\": { \"title\": \"Motor apagado\", \"body\": \"\", \"sound\": \"default\" }, \"data\": { \"message\": \"\" } }"}"#;
        assert_eq!(payload, expected);
    }
}
