use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub unit_id: String,
    pub rule_id: Uuid,
    pub source_type: String,
    pub source_id: Uuid,
    pub alert_type: String,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::AlertEvent;

    const SAMPLE_EVENT: &str = r#"{
  "id": "11111111-1111-1111-1111-111111111111",
  "organization_id": "22222222-2222-2222-2222-222222222222",
  "unit_id": "33333333-3333-3333-3333-333333333333",
  "rule_id": "44444444-4444-4444-4444-444444444444",
  "source_type": "event",
  "source_id": "55555555-5555-5555-5555-555555555555",
  "alert_type": "Engine OFF",
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
        assert_eq!(parsed.alert_type, "Engine OFF");
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
}
