//! HTTP transport port for the Home Assistant REST client.
//!
//! [`HaTransport`] is the seam that makes the client unit testable: production
//! code uses [`ReqwestTransport`], tests use an in-memory fake. The port returns
//! the raw status code and body so the client owns status handling (401 mapping,
//! error envelopes) in one place.

use async_trait::async_trait;

use crate::error::{HaError, HaResult};

/// Low-level HTTP port used by [`crate::client::HaClient`].
#[async_trait]
pub trait HaTransport: Send + Sync {
    /// Perform a `GET` against the Home Assistant REST API.
    ///
    /// `path` starts with `/api/`. Returns the numeric status code and the UTF-8
    /// response body.
    async fn get(&self, path: &str) -> HaResult<(u16, String)>;

    /// Perform a `POST` with a JSON body against the REST API.
    async fn post_json(&self, path: &str, body: serde_json::Value) -> HaResult<(u16, String)>;
}

/// Production transport backed by `reqwest`.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Create a transport for `base_url` (e.g. `http://10.0.0.5:8123`) using a
    /// Home Assistant long-lived access token.
    ///
    /// The token is only ever sent in the `Authorization` header; it is never
    /// logged. The trailing `/` of `base_url` is normalized away.
    #[must_use]
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            token: token.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Full request URL for an API path.
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[async_trait]
impl HaTransport for ReqwestTransport {
    async fn get(&self, path: &str) -> HaResult<(u16, String)> {
        let response = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|error| {
                HaError::Unavailable(format!("request to Home Assistant failed: {error}"))
            })?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| {
            HaError::Http(format!("failed to read Home Assistant response: {error}"))
        })?;
        Ok((status, body))
    }

    async fn post_json(&self, path: &str, body: serde_json::Value) -> HaResult<(u16, String)> {
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                HaError::Unavailable(format!("request to Home Assistant failed: {error}"))
            })?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|error| {
            HaError::Http(format!("failed to read Home Assistant response: {error}"))
        })?;
        Ok((status, body))
    }
}

#[cfg(test)]
mod tests {
    use super::ReqwestTransport;
    use crate::transport::HaTransport;

    #[test]
    fn construction_does_not_require_a_network() {
        let transport = ReqwestTransport::new("http://127.0.0.1:8123/", "test-token");
        // Builds the request URL without a round trip; no I/O happens here.
        assert_eq!(transport.url("/api/"), "http://127.0.0.1:8123/api/");
    }

    #[tokio::test]
    async fn unreachable_instance_maps_to_unavailable() {
        // 127.0.0.1:9 (discard) is not listening; this is a local connection
        // failure, not an outbound network call.
        let transport = ReqwestTransport::new("http://127.0.0.1:9", "test-token");
        let error = transport
            .get("/api/")
            .await
            .expect_err("connection refused");
        assert!(matches!(error, crate::error::HaError::Unavailable(_)));
    }
}
