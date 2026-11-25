//! Shared, reconfigurable Home Assistant handle used by the daemon.
//!
//! The daemon stores the Home Assistant URL/token after onboarding and may
//! change them at runtime. [`HaService`] holds the configured [`HaClient`] behind
//! an [`tokio::sync::RwLock`] so every request path reads the current client
//! without holding a lock across an `await`. When Home Assistant is not
//! configured yet every method returns [`HaError::Unavailable`] and the API layer
//! maps it to `409/503` style errors instead of panicking.

use std::sync::Arc;

use serde_json::Value;

use crate::client::HaClient;
use crate::error::{HaError, HaResult};
use crate::model::{HaEntity, HaStatus};

/// Reconfigurable handle to the managed Home Assistant instance.
pub struct HaService {
    client: tokio::sync::RwLock<Option<Arc<HaClient>>>,
}

impl HaService {
    /// Create an unconfigured service. Configure it with [`Self::configure`]
    /// once the Home Assistant URL and token are known.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: tokio::sync::RwLock::new(None),
        }
    }

    /// Install (or replace) the client used for subsequent calls.
    pub async fn configure(&self, client: Arc<HaClient>) {
        *self.client.write().await = Some(client);
    }

    /// Forget the configured client (Home Assistant disabled or removed).
    pub async fn clear(&self) {
        *self.client.write().await = None;
    }

    /// Whether a client is currently configured.
    pub async fn is_configured(&self) -> bool {
        self.client.read().await.is_some()
    }

    /// Clone the current client out of the lock, or report it is unconfigured.
    async fn require_client(&self) -> HaResult<Arc<HaClient>> {
        self.client
            .read()
            .await
            .clone()
            .ok_or_else(|| HaError::Unavailable("Home Assistant is not configured".to_owned()))
    }

    /// Probe the configured instance, or fail with [`HaError::Unavailable`].
    ///
    /// # Errors
    /// See [`HaClient::status`]; [`HaError::Unavailable`] when not configured.
    pub async fn status(&self) -> HaResult<HaStatus> {
        self.require_client().await?.status().await
    }

    /// List entities of the configured instance.
    ///
    /// # Errors
    /// See [`HaClient::entities`]; [`HaError::Unavailable`] when not configured.
    pub async fn entities(&self) -> HaResult<Vec<HaEntity>> {
        self.require_client().await?.entities().await
    }

    /// Fetch a single entity of the configured instance.
    ///
    /// # Errors
    /// See [`HaClient::entity`]; [`HaError::Unavailable`] when not configured.
    pub async fn entity(&self, entity_id: &str) -> HaResult<HaEntity> {
        self.require_client().await?.entity(entity_id).await
    }

    /// Read the raw `/api/config` payload of the configured instance.
    ///
    /// # Errors
    /// See [`HaClient::config`]; [`HaError::Unavailable`] when not configured.
    pub async fn config(&self) -> HaResult<Value> {
        self.require_client().await?.config().await
    }

    /// Call a Home Assistant service on the configured instance.
    ///
    /// # Errors
    /// See [`HaClient::call_service`]; [`HaError::Unavailable`] when not
    /// configured.
    pub async fn call_service(&self, domain: &str, service: &str, data: Value) -> HaResult<()> {
        self.require_client()
            .await?
            .call_service(domain, service, data)
            .await
    }
}

impl Default for HaService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use super::HaService;
    use crate::client::HaClient;
    use crate::error::HaError;
    use crate::transport::HaTransport;

    struct MockTransport {
        responses: Mutex<VecDeque<(u16, String)>>,
    }

    impl MockTransport {
        fn new(responses: Vec<(u16, String)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl HaTransport for MockTransport {
        async fn get(&self, _path: &str) -> crate::error::HaResult<(u16, String)> {
            Ok(self
                .responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .unwrap_or((500, String::new())))
        }

        async fn post_json(
            &self,
            _path: &str,
            _body: Value,
        ) -> crate::error::HaResult<(u16, String)> {
            Ok(self
                .responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .unwrap_or((500, String::new())))
        }
    }

    fn client(responses: Vec<(u16, String)>) -> Arc<HaClient> {
        Arc::new(HaClient::new(Arc::new(MockTransport::new(responses))))
    }

    #[tokio::test]
    async fn unconfigured_service_returns_unavailable() {
        let service = HaService::new();
        assert!(!service.is_configured().await);
        assert!(matches!(
            service.status().await,
            Err(HaError::Unavailable(_))
        ));
        assert!(matches!(
            service.entities().await,
            Err(HaError::Unavailable(_))
        ));
        assert!(matches!(
            service.call_service("light", "turn_on", json!({})).await,
            Err(HaError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn configure_and_clear_switch_the_client() {
        let service = HaService::new();
        service
            .configure(client(vec![(
                200,
                json!({"message": "API running."}).to_string(),
            )]))
            .await;
        assert!(service.is_configured().await);
        let status = service.status().await.expect("status");
        assert!(status.running);

        service.clear().await;
        assert!(!service.is_configured().await);
        assert!(matches!(
            service.status().await,
            Err(HaError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn configured_service_delegates_calls() {
        let service = HaService::new();
        service
            .configure(client(vec![
                (
                    200,
                    json!([{"entity_id": "light.kitchen", "state": "on", "attributes": {}}])
                        .to_string(),
                ),
                (200, "{}".to_owned()),
                (401, String::new()),
            ]))
            .await;

        let entities = service.entities().await.expect("entities");
        assert_eq!(entities.len(), 1);
        service
            .call_service("light", "turn_off", json!({"entity_id": "light.kitchen"}))
            .await
            .expect("service call");
        assert!(matches!(service.status().await, Err(HaError::Unauthorized)));
    }
}
