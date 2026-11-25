//! Typed Home Assistant REST client.
//!
//! [`HaClient`] depends only on the [`crate::transport::HaTransport`] port, so
//! every endpoint is unit tested with a fake transport and no network access.
//! The client owns the status handling shared by all calls:
//!
//! - `401` maps to [`crate::HaError::Unauthorized`];
//! - other non-2xx statuses map to [`crate::HaError::Http`] with the status and
//!   (truncated) body for diagnostics;
//! - invalid JSON maps to [`crate::HaError::Parse`].

use std::sync::Arc;

use serde_json::Value;

use crate::error::{HaError, HaResult};
use crate::model::{HaEntity, HaStatus};
use crate::transport::HaTransport;

/// Maximum number of response bytes kept in an error message.
const MAX_ERROR_BODY_CHARS: usize = 512;

/// Home Assistant REST client.
#[derive(Clone)]
pub struct HaClient {
    transport: Arc<dyn HaTransport>,
}

impl HaClient {
    /// Create a client around a transport implementation.
    #[must_use]
    pub fn new(transport: Arc<dyn HaTransport>) -> Self {
        Self { transport }
    }

    /// Probe the API (`GET /api/`) and return the running status.
    ///
    /// # Errors
    /// Returns [`HaError::Unauthorized`] for a rejected token,
    /// [`HaError::Http`] for other unexpected statuses and [`HaError::Parse`]
    /// for an invalid payload.
    pub async fn status(&self) -> HaResult<HaStatus> {
        let (status, body) = self.transport.get("/api/").await?;
        ensure_success(status, "/api/", &body)?;
        let value = parse_json(&body, "/api/")?;
        Ok(HaStatus::from_api_value(&value))
    }

    /// List every entity state (`GET /api/states`).
    ///
    /// # Errors
    /// Propagates transport, status and parse errors.
    pub async fn entities(&self) -> HaResult<Vec<HaEntity>> {
        let (status, body) = self.transport.get("/api/states").await?;
        ensure_success(status, "/api/states", &body)?;
        let values: Vec<Value> = serde_json::from_str(&body).map_err(|error| {
            HaError::Parse(format!("invalid entity list from `/api/states`: {error}"))
        })?;
        values
            .iter()
            .map(HaEntity::from_value)
            .collect::<HaResult<Vec<HaEntity>>>()
    }

    /// Fetch a single entity state (`GET /api/states/{entity_id}`).
    ///
    /// # Errors
    /// Returns [`HaError::Http`] with status 404 when the entity does not exist,
    /// and propagates transport and parse errors.
    pub async fn entity(&self, entity_id: &str) -> HaResult<HaEntity> {
        if !is_entity_id(entity_id) {
            return Err(HaError::Http(format!(
                "invalid Home Assistant entity id `{entity_id}`"
            )));
        }
        let path = format!("/api/states/{entity_id}");
        let (status, body) = self.transport.get(&path).await?;
        ensure_success(status, &path, &body)?;
        let value = parse_json(&body, &path)?;
        HaEntity::from_value(&value)
    }

    /// Call a service (`POST /api/services/{domain}/{service}`).
    ///
    /// The service response (the list of changed states) is discarded: the
    /// appliance only needs to know the call was accepted.
    ///
    /// # Errors
    /// Returns [`HaError::Http`] for an invalid domain/service identifier or a
    /// non-2xx status, and propagates transport errors.
    pub async fn call_service(&self, domain: &str, service: &str, data: Value) -> HaResult<()> {
        if !is_service_identifier(domain) || !is_service_identifier(service) {
            return Err(HaError::Http(format!(
                "invalid Home Assistant service `{domain}.{service}`"
            )));
        }
        let path = format!("/api/services/{domain}/{service}");
        let (status, body) = self.transport.post_json(&path, data).await?;
        ensure_success(status, &path, &body)
    }

    /// Fetch the raw `/api/config` payload (location, timezone, units, version).
    ///
    /// # Errors
    /// Propagates transport, status and parse errors.
    pub async fn config(&self) -> HaResult<Value> {
        let (status, body) = self.transport.get("/api/config").await?;
        ensure_success(status, "/api/config", &body)?;
        parse_json(&body, "/api/config")
    }
}

/// Map an HTTP status to a client error, accepting any 2xx response.
fn ensure_success(status: u16, path: &str, body: &str) -> HaResult<()> {
    match status {
        401 => Err(HaError::Unauthorized),
        status if (200..300).contains(&status) => Ok(()),
        status => Err(HaError::Http(format!(
            "`{path}` returned HTTP {status}: {}",
            truncate(body, MAX_ERROR_BODY_CHARS)
        ))),
    }
}

/// Parse a JSON body, mapping serde errors to [`HaError::Parse`].
fn parse_json(body: &str, path: &str) -> HaResult<Value> {
    serde_json::from_str(body)
        .map_err(|error| HaError::Parse(format!("invalid JSON from `{path}`: {error}")))
}

/// Truncate a diagnostic body to a bounded number of characters.
fn truncate(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_owned()
    } else {
        let truncated: String = trimmed.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// Entity ids are `domain.object_id` with lowercase letters, digits and `_`.
fn is_entity_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.'))
        && value.matches('.').count() == 1
        && !value.starts_with('.')
        && !value.ends_with('.')
}

/// Service domain/service names are lowercase snake_case identifiers.
fn is_service_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !value.starts_with('_')
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use super::HaClient;
    use crate::error::HaError;
    use crate::transport::HaTransport;

    /// Deterministic in-memory transport: responses are served in order and
    /// every request is recorded.
    struct MockTransport {
        responses: Mutex<VecDeque<(u16, String)>>,
        requests: Mutex<Vec<(String, Option<Value>)>>,
    }

    impl MockTransport {
        fn new(responses: Vec<(&str, u16, String)>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(_label, status, body)| (status, body))
                        .collect(),
                ),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<(String, Option<Value>)> {
            self.requests.lock().expect("requests lock").clone()
        }

        fn next_response(&self) -> (u16, String) {
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .unwrap_or((500, "mock response queue exhausted".to_owned()))
        }
    }

    #[async_trait]
    impl HaTransport for MockTransport {
        async fn get(&self, path: &str) -> crate::error::HaResult<(u16, String)> {
            self.requests
                .lock()
                .expect("requests lock")
                .push((path.to_owned(), None));
            Ok(self.next_response())
        }

        async fn post_json(
            &self,
            path: &str,
            body: Value,
        ) -> crate::error::HaResult<(u16, String)> {
            self.requests
                .lock()
                .expect("requests lock")
                .push((path.to_owned(), Some(body)));
            Ok(self.next_response())
        }
    }

    fn mock_client(responses: Vec<(&str, u16, String)>) -> (HaClient, Arc<MockTransport>) {
        let transport = Arc::new(MockTransport::new(responses));
        (HaClient::new(transport.clone()), transport)
    }

    #[tokio::test]
    async fn status_success() {
        let (client, transport) = mock_client(vec![(
            "/api/",
            200,
            json!({"message": "API running.", "version": "2026.1.0"}).to_string(),
        )]);
        let status = client.status().await.expect("status");
        assert!(status.running);
        assert_eq!(status.version.as_deref(), Some("2026.1.0"));
        assert_eq!(status.location_name, None);
        assert_eq!(transport.requests(), vec![("/api/".to_owned(), None)]);
    }

    #[tokio::test]
    async fn entities_success() {
        let body = json!([
            {
                "entity_id": "light.kitchen",
                "state": "on",
                "attributes": {"friendly_name": "Kitchen Light"},
                "last_changed": "2026-01-01T10:00:00+00:00"
            },
            {
                "entity_id": "sensor.temperature",
                "state": "21.5",
                "attributes": {}
            }
        ])
        .to_string();
        let (client, _transport) = mock_client(vec![("/api/states", 200, body)]);
        let entities = client.entities().await.expect("entities");
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].domain, "light");
        assert_eq!(entities[0].name, "Kitchen Light");
        assert_eq!(entities[1].domain, "sensor");
        assert_eq!(entities[1].name, "sensor.temperature");
    }

    #[tokio::test]
    async fn entity_success_and_not_found() {
        let (client, transport) = mock_client(vec![
            (
                "/api/states/light.kitchen",
                200,
                json!({"entity_id": "light.kitchen", "state": "off", "attributes": {}}).to_string(),
            ),
            ("/api/states/light.missing", 404, "not found".to_owned()),
        ]);
        let entity = client.entity("light.kitchen").await.expect("entity");
        assert_eq!(entity.state, "off");
        assert_eq!(entity.domain, "light");

        let error = client
            .entity("light.missing")
            .await
            .expect_err("missing entity");
        assert!(matches!(error, HaError::Http(message) if message.contains("404")));
        assert_eq!(
            transport.requests()[0].0,
            "/api/states/light.kitchen".to_owned()
        );
    }

    #[tokio::test]
    async fn unauthorized_maps_to_unauthorized() {
        let (client, _transport) =
            mock_client(vec![("/api/states", 401, "unauthorized".to_owned())]);
        let error = client.entities().await.expect_err("401");
        assert!(matches!(error, HaError::Unauthorized));

        let (client, _transport) = mock_client(vec![("/api/config", 401, String::new())]);
        let error = client.config().await.expect_err("401");
        assert!(matches!(error, HaError::Unauthorized));
    }

    #[tokio::test]
    async fn call_service_posts_to_the_right_endpoint() {
        let (client, transport) = mock_client(vec![(
            "/api/services/light/turn_on",
            200,
            json!([{"entity_id": "light.kitchen", "state": "on"}]).to_string(),
        )]);
        client
            .call_service(
                "light",
                "turn_on",
                json!({"entity_id": "light.kitchen", "brightness": 200}),
            )
            .await
            .expect("service call");
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "/api/services/light/turn_on");
        assert_eq!(
            requests[0]
                .1
                .as_ref()
                .and_then(|body| body.get("entity_id")),
            Some(&json!("light.kitchen"))
        );
    }

    #[tokio::test]
    async fn call_service_rejects_invalid_identifiers() {
        let (client, transport) = mock_client(Vec::new());
        assert!(matches!(
            client
                .call_service("light/../config", "turn_on", json!({}))
                .await,
            Err(HaError::Http(_))
        ));
        assert!(matches!(
            client.call_service("Light", "turn_on", json!({})).await,
            Err(HaError::Http(_))
        ));
        assert!(matches!(
            client.call_service("light", "turn on", json!({})).await,
            Err(HaError::Http(_))
        ));
        assert!(transport.requests().is_empty());
    }

    #[tokio::test]
    async fn config_success() {
        let (client, _transport) = mock_client(vec![(
            "/api/config",
            200,
            json!({"location_name": "Home", "time_zone": "Europe/Madrid"}).to_string(),
        )]);
        let config = client.config().await.expect("config");
        assert_eq!(config["location_name"], json!("Home"));
        assert_eq!(config["time_zone"], json!("Europe/Madrid"));
    }

    #[tokio::test]
    async fn server_errors_and_malformed_json_are_reported() {
        let (client, _transport) = mock_client(vec![("/api/", 500, "boom".to_owned())]);
        let error = client.status().await.expect_err("500");
        assert!(matches!(error, HaError::Http(message) if message.contains("500")));

        let (client, _transport) = mock_client(vec![("/api/states", 200, "not json".to_owned())]);
        let error = client.entities().await.expect_err("invalid json");
        assert!(matches!(error, HaError::Parse(_)));

        let (client, _transport) = mock_client(vec![(
            "/api/states",
            200,
            json!([{"state": "on"}]).to_string(),
        )]);
        let error = client.entities().await.expect_err("entity without id");
        assert!(matches!(error, HaError::Parse(_)));
    }

    #[test]
    fn entity_id_validation() {
        assert!(super::is_entity_id("light.kitchen"));
        assert!(super::is_entity_id("sensor.living_room_temperature"));
        assert!(!super::is_entity_id("light"));
        assert!(!super::is_entity_id("light.kitchen.extra"));
        assert!(!super::is_entity_id("light.Kitchen"));
        assert!(!super::is_entity_id("../etc/passwd"));
        assert!(!super::is_entity_id(""));
        assert!(super::is_service_identifier("turn_on"));
        assert!(!super::is_service_identifier("turn on"));
        assert!(!super::is_service_identifier("_turn_on"));
    }
}
