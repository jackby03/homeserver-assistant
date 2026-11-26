//! Allow-listed Composio action gateway.
//!
//! Security model:
//! - disabled by default;
//! - the project API key is stored encrypted and read per call, never logged;
//! - the model can only reach named capabilities mapped to fixed Composio
//!   action slugs (no arbitrary slug selection);
//! - OAuth tokens never enter the appliance: Composio stores a connected
//!   account per toolkit and the daemon keeps only its id;
//! - auth configs and connected accounts are selected per toolkit;
//! - the base URL is validated by the config layer (Composio HTTPS or loopback
//!   for the local POC);
//! - every outbound call has a timeout and returns only the action `data`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hs_core::config::ComposioConfig;
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

    fn api_key(&self, config: &ComposioConfig) -> Result<String, String> {
        self.secrets
            .get(&config.api_key_secret)
            .map_err(|error| format!("cannot read the Composio API key: {error}"))?
            .map(|key| key.to_string())
            .ok_or_else(|| {
                "no hay API key de Composio guardada; configura una en administración".to_owned()
            })
    }

    fn endpoint(&self, config: &ComposioConfig, path: &str) -> String {
        format!("{}{}", config.base_url.trim_end_matches('/'), path)
    }

    fn action(&self, capability: &str) -> Option<(&'static str, String)> {
        let composio = &self.config.snapshot().integrations.composio;
        let (toolkit, slug) = match capability {
            "calendar.list" => ("googlecalendar", composio.calendar_list_action.clone()),
            "calendar.create" => ("googlecalendar", composio.calendar_create_action.clone()),
            "email.send" => ("gmail", composio.email_send_action.clone()),
            _ => return None,
        };
        Some((toolkit, slug))
    }

    fn connected_account(config: &ComposioConfig, toolkit: &str) -> Option<String> {
        config
            .connected_accounts
            .get(toolkit)
            .cloned()
            .or_else(|| config.connected_account_id.clone())
    }

    /// Create a Composio connected account for `toolkit` and return the OAuth
    /// challenge. The connected account id is persisted immediately; Composio
    /// activates it after the user grants consent.
    ///
    /// # Errors
    /// Returns a human-readable error when the integration is disabled, the
    /// toolkit has no auth config, or Composio rejects the request.
    pub async fn create_connection(&self, toolkit: &str) -> Result<Value, String> {
        let config = self.config.snapshot().integrations.composio;
        if !config.enabled {
            return Err("la integración Composio está desactivada".to_owned());
        }
        let auth_config_id = config.auth_configs.get(toolkit).cloned().ok_or_else(|| {
            format!("no hay auth config para `{toolkit}`; define auth_configs.{toolkit} primero")
        })?;
        let api_key = self.api_key(&config)?;
        let response = self
            .http
            .post(self.endpoint(&config, "/api/v3.1/connected_accounts"))
            .header("x-api-key", api_key)
            .json(&json!({
                "user_id": config.user_id,
                "auth_config_id": auth_config_id,
                "toolkit_slug": toolkit,
            }))
            .send()
            .await
            .map_err(|error| format!("Composio connection request failed: {error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("Composio returned invalid JSON: {error}"))?;
        if !status.is_success() {
            let message = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Composio rejected the connection request");
            return Err(format!("Composio error ({status}): {message}"));
        }
        let id = body
            .get("id")
            .or_else(|| body.get("connected_account_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Composio did not return a connected account id".to_owned())?;
        let redirect_url = body
            .get("redirect_url")
            .or_else(|| body.get("redirectUrl"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.config
            .update(|current| {
                current
                    .integrations
                    .composio
                    .connected_accounts
                    .insert(toolkit.to_owned(), id.to_owned());
                Ok(())
            })
            .map_err(|error| format!("cannot persist the connected account: {error}"))?;
        Ok(json!({
            "toolkit": toolkit,
            "connected_account_id": id,
            "redirect_url": redirect_url,
            "status": body.get("status").cloned().unwrap_or(Value::Null),
        }))
    }

    /// Discover the Composio tool schemas exposed for `toolkit`.
    ///
    /// # Errors
    /// Returns a human-readable error when the integration is disabled or the
    /// catalog request fails.
    pub async fn discover_tools(&self, toolkit: &str) -> Result<Value, String> {
        let config = self.config.snapshot().integrations.composio;
        if !config.enabled {
            return Err("la integración Composio está desactivada".to_owned());
        }
        let api_key = self.api_key(&config)?;
        let response = self
            .http
            .get(self.endpoint(&config, "/api/v3.1/tools"))
            .header("x-api-key", api_key)
            .query(&[("toolkit_slug", toolkit), ("toolkit_versions", "latest")])
            .send()
            .await
            .map_err(|error| format!("Composio tools request failed: {error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("Composio returned invalid JSON: {error}"))?;
        if !status.is_success() {
            let message = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Composio rejected the tools request");
            return Err(format!("Composio error ({status}): {message}"));
        }
        Ok(body)
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
        let Some((toolkit, slug)) = self.action(capability) else {
            return Err(format!("capability `{capability}` is not allow-listed"));
        };
        let api_key = self.api_key(&config)?;
        let connected_account_id = Self::connected_account(&config, toolkit).ok_or_else(|| {
            format!(
                "no hay connected account para `{toolkit}`; conecta la app desde administración"
            )
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
        let endpoint = self.endpoint(&config, &format!("/api/v3.1/tools/execute/{slug}"));
        let response = self
            .http
            .post(endpoint)
            .header("x-api-key", api_key)
            .json(&json!({
                "user_id": config.user_id,
                "connected_account_id": connected_account_id,
                "toolkit_versions": "latest",
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
