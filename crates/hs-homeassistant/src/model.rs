//! Home Assistant domain models exposed by the REST client.
//!
//! The structs mirror the JSON returned by Home Assistant but are normalized for
//! the appliance: `name` is always populated (falling back to the entity id) and
//! `domain` is derived from the entity id, so the UI never has to parse entity
//! identifiers.

use serde::{Deserialize, Serialize};

use crate::error::{HaError, HaResult};

/// A Home Assistant entity state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HaEntity {
    /// Full entity id, e.g. `light.living_room`.
    pub entity_id: String,
    /// Current state string, e.g. `on`, `off`, `unavailable`.
    pub state: String,
    /// Friendly name reported in the attributes, falling back to the entity id.
    pub name: String,
    /// Integration domain derived from the entity id, e.g. `light`.
    pub domain: String,
    /// Raw attribute object returned by Home Assistant.
    pub attributes: serde_json::Value,
    /// Last state change timestamp as reported by Home Assistant (RFC 3339).
    pub last_changed: Option<String>,
}

impl HaEntity {
    /// Build an entity from a `/api/states` JSON object.
    ///
    /// # Errors
    /// Returns [`HaError::Parse`] when the object has no `entity_id` or no
    /// string `state`.
    pub fn from_value(value: &serde_json::Value) -> HaResult<Self> {
        let entity_id = value
            .get("entity_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| HaError::Parse("entity object is missing `entity_id`".to_owned()))?;
        let state = value
            .get("state")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                HaError::Parse(format!("entity `{entity_id}` is missing string `state`"))
            })?;
        let attributes = value
            .get("attributes")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        let name = attributes
            .get("friendly_name")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(entity_id)
            .to_owned();
        let domain = domain_of(entity_id).to_owned();
        let last_changed = value
            .get("last_changed")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);

        Ok(Self {
            entity_id: entity_id.to_owned(),
            state: state.to_owned(),
            name,
            domain,
            attributes,
            last_changed,
        })
    }
}

/// Extract the integration domain from an entity id (`light.kitchen` -> `light`).
fn domain_of(entity_id: &str) -> &str {
    entity_id
        .split_once('.')
        .map_or(entity_id, |(domain, _object_id)| domain)
}

/// Health and identity information reported by `/api/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaStatus {
    /// Whether the API answered successfully.
    pub running: bool,
    /// Home Assistant version when the payload reports it.
    pub version: Option<String>,
    /// Configured location name ("home") when the payload reports it.
    pub location_name: Option<String>,
}

impl HaStatus {
    /// Build the status from a `/api/` JSON payload.
    #[must_use]
    pub fn from_api_value(value: &serde_json::Value) -> Self {
        Self {
            running: true,
            version: value
                .get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            location_name: value
                .get("location_name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{HaEntity, HaStatus};

    #[test]
    fn parses_entity_payload() {
        let entity = HaEntity::from_value(&json!({
            "entity_id": "light.living_room",
            "state": "on",
            "attributes": {
                "friendly_name": "Living Room Lamp",
                "brightness": 120
            },
            "last_changed": "2026-01-01T10:00:00+00:00"
        }))
        .expect("entity parsed");
        assert_eq!(entity.entity_id, "light.living_room");
        assert_eq!(entity.state, "on");
        assert_eq!(entity.name, "Living Room Lamp");
        assert_eq!(entity.domain, "light");
        assert_eq!(entity.attributes["brightness"], json!(120));
        assert_eq!(
            entity.last_changed.as_deref(),
            Some("2026-01-01T10:00:00+00:00")
        );
    }

    #[test]
    fn falls_back_to_entity_id_when_friendly_name_is_missing() {
        let entity = HaEntity::from_value(&json!({
            "entity_id": "sensor.outdoor_temperature",
            "state": "21.5",
            "attributes": {}
        }))
        .expect("entity parsed");
        assert_eq!(entity.name, "sensor.outdoor_temperature");
        assert_eq!(entity.domain, "sensor");
        assert_eq!(entity.last_changed, None);
    }

    #[test]
    fn rejects_incomplete_entity_payloads() {
        assert!(HaEntity::from_value(&json!({"state": "on"})).is_err());
        assert!(HaEntity::from_value(&json!({"entity_id": "light.x"})).is_err());
    }

    #[test]
    fn parses_status_payload() {
        let status = HaStatus::from_api_value(&json!({
            "message": "API running.",
            "version": "2026.1.0",
            "location_name": "Home"
        }));
        assert!(status.running);
        assert_eq!(status.version.as_deref(), Some("2026.1.0"));
        assert_eq!(status.location_name.as_deref(), Some("Home"));

        let minimal = HaStatus::from_api_value(&json!({"message": "API running."}));
        assert!(minimal.running);
        assert_eq!(minimal.version, None);
    }
}
