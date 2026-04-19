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
        // High priority for iOS is essential for background delivery in production (TestFlight)
        let apns_payload = json!({
            "aps": {
                "alert": {
                    "title": self.title,
                    "body": self.body,
                },
                "sound": "alert_default.caf",
                "content-available": 1
            }
        });

        // GCM Payload (for FCM endpoints)
        // Note: For iOS devices registered via GCM/FCM:
        // 1. "priority": "high" is mandatory for reliable delivery in production.
        // 2. "content_available": true ensures the app is woken up.
        let gcm_payload = json!({
            "priority": "high",
            "content_available": true,
            "notification": {
                "title": self.title,
                "body": self.body,
                "sound": "alert_default.caf",
                "click_action": "TOP_STORY_NOTIFICATION"
            },
            "data": {
                "message": self.body,
                "title": self.title,
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
        assert_eq!(gcm["priority"], "high");
        assert_eq!(gcm["content_available"], true);
        assert_eq!(gcm["notification"]["title"], "Ingreso a geocerca");
        assert_eq!(gcm["notification"]["body"], "Camioneta Juan");
        assert_eq!(gcm["notification"]["sound"], "alert_default.caf");

        // Verify APNS content
        let apns_str = parsed["APNS"].as_str().unwrap();
        let apns: serde_json::Value = serde_json::from_str(apns_str).unwrap();
        assert_eq!(apns["aps"]["alert"]["title"], "Ingreso a geocerca");
        assert_eq!(apns["aps"]["sound"], "alert_default.caf");
        assert_eq!(apns["aps"]["content-available"], 1);
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
