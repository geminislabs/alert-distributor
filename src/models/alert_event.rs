use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub unit_id: String,
    pub unit_name: Option<String>,
    pub rule_id: Uuid,
    pub source_type: String,
    pub source_id: Uuid,
    pub alert_type: String,
    pub alert_name: Option<String>,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

impl AlertEvent {
    pub fn notification_title(&self) -> String {
        self.alert_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.alert_type.clone())
    }

    pub fn notification_body(&self) -> String {
        self.unit_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    pub fn sns_notification_title(&self) -> String {
        let title = self.notification_title();

        if self.is_geofence_alert() {
            return format!("📍 {}", title);
        }

        if self.is_ignition_alert() {
            return format!("⏻ {}", title);
        }

        title
    }

    fn is_geofence_alert(&self) -> bool {
        self.matches_alert_keyword(&["geocerca", "geofence"])
    }

    fn is_ignition_alert(&self) -> bool {
        self.matches_alert_keyword(&["ignicion", "ignición", "ignition", "engine", "motor"])
    }

    fn matches_alert_keyword(&self, keywords: &[&str]) -> bool {
        let alert_type = self.alert_type.to_lowercase();
        let alert_name = self
            .alert_name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();

        keywords
            .iter()
            .any(|keyword| alert_type.contains(keyword) || alert_name.contains(keyword))
    }
}

#[cfg(test)]
mod tests {
    use super::AlertEvent;

    const SAMPLE_EVENT: &str = r#"{
  "id": "11111111-1111-1111-1111-111111111111",
  "organization_id": "22222222-2222-2222-2222-222222222222",
  "unit_id": "33333333-3333-3333-3333-333333333333",
    "unit_name": "Unit Name",
  "rule_id": "44444444-4444-4444-4444-444444444444",
  "source_type": "event",
  "source_id": "55555555-5555-5555-5555-555555555555",
  "alert_type": "Engine OFF",
    "alert_name": "Engine is OFF",
  "payload": {
    "backup_batery_voltage": "4.2",
    "engine_status": "OFF",
    "fix_status": "1",
    "latitude": 19.216813,
    "longitude": -102.575137,
    "main_battery_voltage": "13.69",
    "satellites": 7,
    "uuid": "66666666-6666-6666-6666-666666666666"
  },
  "occurred_at": "2026-03-29T20:56:34Z"
}"#;

    #[test]
    fn parses_full_alert_event() {
        let parsed: AlertEvent = serde_json::from_str(SAMPLE_EVENT).expect("event should parse");

        assert_eq!(
            parsed.id.to_string(),
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(parsed.unit_id, "33333333-3333-3333-3333-333333333333");
        assert_eq!(parsed.unit_name.as_deref(), Some("Unit Name"));
        assert_eq!(parsed.alert_type, "Engine OFF");
        assert_eq!(parsed.alert_name.as_deref(), Some("Engine is OFF"));
        assert_eq!(parsed.source_type, "event");
    }

    #[test]
    fn preserves_payload_fields() {
        let parsed: AlertEvent = serde_json::from_str(SAMPLE_EVENT).expect("event should parse");

        assert_eq!(
            parsed.payload["backup_batery_voltage"].as_str(),
            Some("4.2")
        );
        assert_eq!(parsed.payload["satellites"].as_i64(), Some(7));
        assert_eq!(parsed.payload["latitude"].as_f64(), Some(19.216813_f64));
    }

    #[test]
    fn notification_fields_use_preferred_values() {
        let parsed: AlertEvent = serde_json::from_str(SAMPLE_EVENT).expect("event should parse");

        assert_eq!(parsed.notification_title(), "Engine is OFF");
        assert_eq!(parsed.notification_body(), "Unit Name");
    }

    #[test]
    fn notification_fields_use_fallbacks_when_empty_or_null() {
        let sample_event = r#"{
  "id": "11111111-1111-1111-1111-111111111111",
  "organization_id": "22222222-2222-2222-2222-222222222222",
  "unit_id": "33333333-3333-3333-3333-333333333333",
  "unit_name": null,
  "rule_id": "44444444-4444-4444-4444-444444444444",
  "source_type": "event",
  "source_id": "55555555-5555-5555-5555-555555555555",
  "alert_type": "Engine OFF",
  "alert_name": "   ",
  "payload": {
    "engine_status": "OFF"
  },
  "occurred_at": "2026-03-29T20:56:34Z"
}"#;

        let parsed: AlertEvent = serde_json::from_str(sample_event).expect("event should parse");

        assert_eq!(parsed.notification_title(), "Engine OFF");
        assert_eq!(parsed.notification_body(), "");
    }

    #[test]
    fn sns_title_includes_geofence_emoji_for_geofence_alerts() {
        let sample_event = r#"{
  "id": "11111111-1111-1111-1111-111111111111",
  "organization_id": "22222222-2222-2222-2222-222222222222",
  "unit_id": "33333333-3333-3333-3333-333333333333",
  "unit_name": "Unidad 1",
  "rule_id": "44444444-4444-4444-4444-444444444444",
  "source_type": "event",
  "source_id": "55555555-5555-5555-5555-555555555555",
  "alert_type": "Ingreso a geocerca",
  "alert_name": "Entrada geocerca",
  "payload": {},
  "occurred_at": "2026-03-29T20:56:34Z"
}"#;

        let parsed: AlertEvent = serde_json::from_str(sample_event).expect("event should parse");
        assert_eq!(parsed.sns_notification_title(), "📍 Entrada geocerca");
    }

    #[test]
    fn sns_title_includes_ignition_emoji_for_ignition_alerts() {
        let parsed: AlertEvent = serde_json::from_str(SAMPLE_EVENT).expect("event should parse");
        assert_eq!(parsed.sns_notification_title(), "⏻ Engine is OFF");
    }
}
