//! Allow-listed Composio action gateway.
//!
//! Security model:
//! - disabled by default;
//! - API key stored encrypted and read per call, never logged;
//! - the model can only reach three named capabilities, mapped to configured
//!   Composio action slugs (no arbitrary slug selection);
//! - the base URL is validated by the config layer (HTTPS Composio or loopback
//!   for the local POC);
//! - every outbound call has a timeout and returns only the action `data`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hs_core::state::{ConfigManager, SecretStore};
use hs_tools::ExternalActions;
use serde_json::{json, Value};

/// Composio-backed implementation of the calendar/email capabilities.
pub struct ComposioAdapter {
    config: Arc<ConfigManager>,
    secrets: Arc<SecretStore>,
    http: reqwest::Client,
}

impl std::fmt::Debug for ComposioAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ComposioAdapter")
            .finish_non_exhaustive()
    }
}

impl ComposioAdapter {
    /// Create the adapter. The API key is looked up lazily, so rotating it in
    /// the secret store takes effect without rebuilding the tool registry.
    #[must_use]
    pub fn new(config: Arc<ConfigManager>, secrets: Arc<SecretStore>) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("HomeServerAssistant/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            config,
            secrets,
            http,
        }
    }

    fn action_slug(&self, capability: &str) -> Option<String> {
        let composio = &self.config.snapshot().integrations.composio;
        let slug = match capability {
            "calendar.list" => composio.calendar_list_action.as_str(),
            "calendar.create" => composio.calendar_create_action.as_str(),
            "email.send" => composio.email_send_action.as_str(),
            _ => return None,
        };
        Some(slug.to_owned())
    }
}

#[async_trait]
impl ExternalActions for ComposioAdapter {
    async fn execute(&self, capability: &str, arguments: Value) -> Result<Value, String> {
        let config = self.config.snapshot().integrations.composio;
        if !config.enabled {
            return Err(
                "la integración Composio está desactivada; actívala en el panel de administración"
                    .to_owned(),
            );
        }
        let Some(slug) = self.action_slug(capability) else {
            return Err(format!("capability `{capability}` is not allow-listed"));
        };
        let api_key = self
            .secrets
            .get(&config.api_key_secret)
            .map_err(|error| format!("cannot read the Composio API key: {error}"))?
            .ok_or_else(|| {
                "no hay API key de Composio guardada; configura una en administración".to_owned()
            })?;
        let payload_arguments = match capability {
            "calendar.list" => arguments,
            "calendar.create" => json!({
                "summary": arguments.get("summary"),
                "start_datetime": arguments.get("start"),
                "end_datetime": arguments.get("end"),
                "description": arguments.get("description"),
                "calendar_id": "primary",
            }),
            "email.send" => json!({
                "recipient_email": arguments.get("to"),
                "subject": arguments.get("subject"),
                "body": arguments.get("body"),
            }),
            _ => return Err("unsupported capability".to_owned()),
        };
        let endpoint = format!(
            "{}/api/v3/tools/execute/{}",
            config.base_url.trim_end_matches('/'),
            slug
        );
        let response = self
            .http
            .post(&endpoint)
            .header("x-api-key", api_key.as_str())
            .json(&json!({
                "user_id": config.user_id,
                "connected_account_id": config.connected_account_id,
                "arguments": payload_arguments,
            }))
            .send()
            .await
            .map_err(|error| format!("Composio request failed: {error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("Composio returned invalid JSON: {error}"))?;
        if !status.is_success() {
            let message = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Composio rejected the action");
            return Err(format!("Composio error ({status}): {message}"));
        }
        if body.get("successful").and_then(Value::as_bool) == Some(false) {
            let message = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Composio action failed");
            return Err(message.to_owned());
        }
        Ok(body.get("data").cloned().unwrap_or(body))
    }
}
