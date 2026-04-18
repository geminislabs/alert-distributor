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
        use serde_json::json;

        // APNS Payload (for direct APNS or APNS_SANDBOX endpoints)
        let apns_payload = json!({
            "aps": {
                "alert": {
                    "title": self.title,
                    "body": self.body,
                },
                "sound": "alert_default.caf"
            }
        });

        // GCM Payload (for FCM endpoints)
        let gcm_payload = json!({
            "notification": {
                "title": self.title,
                "body": self.body,
                "sound": "alert_default.caf"
            },
            "data": {
                "message": self.body
            }
        });

        // SNS Wrapper
        // Note: SNS expects the platform-specific payloads to be JSON strings themselves.
        let sns_payload = json!({
            "default": "alert",
            "APNS": apns_payload.to_string(),
            "APNS_SANDBOX": apns_payload.to_string(),
            "GCM": gcm_payload.to_string()
        });

        sns_payload.to_string()
    }
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

        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(parsed["default"], "alert");
        assert!(parsed.get("APNS").is_some());
        assert!(parsed.get("APNS_SANDBOX").is_some());
        assert!(parsed.get("GCM").is_some());

        // Verify GCM content
        let gcm_str = parsed["GCM"].as_str().unwrap();
        let gcm: serde_json::Value = serde_json::from_str(gcm_str).unwrap();
        assert_eq!(gcm["notification"]["title"], "Ingreso a geocerca");
        assert_eq!(gcm["notification"]["body"], "Camioneta Juan");
    }

    #[test]
    fn sns_payload_handles_special_characters() {
        let message = SnsMessage::new("Alerta \"Crítica\"", "Línea 1\nLínea 2");
        let payload = message.to_json_payload();

        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let gcm_str = parsed["GCM"].as_str().unwrap();
        let gcm: serde_json::Value = serde_json::from_str(gcm_str).unwrap();

        assert_eq!(gcm["notification"]["title"], "Alerta \"Crítica\"");
        assert_eq!(gcm["notification"]["body"], "Línea 1\nLínea 2");
    }
}
