//! Provider management and the agent tool-calling loop.
//!
//! [`AgentService`] ties together the encrypted secret store, the profile
//! permissions, the tool registry and an injected [`LlmClientFactory`]. The
//! chat loop calls the model, executes the requested tools (always through
//! [`hs_tools::ToolRegistry`], which enforces the profile allow-list), feeds the
//! results back, and repeats up to `max_tool_steps` times.
//!
//! API keys are only ever stored in [`hs_core::state::SecretStore`] under
//! `llm.provider.<provider-id>.api_key`; the persisted [`ProviderRecord`] holds
//! just the secret name.

use std::sync::Arc;

use chrono::Utc;
use hs_core::config::LlmProviderKind;
use hs_core::models::{
    AgentChatRequest, AgentChatResponse, ChatMessage, ChatRole, Profile, ProviderRecord,
    TokenUsage, ToolCall, ToolTraceEntry,
};
use hs_core::state::{Database, SecretStore};
use hs_core::validation;
use hs_profiles::ProfileService;
use hs_tools::{ToolError, ToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{AgentError, AgentResult};
use crate::llm::{LlmClientFactory, LlmRequest, LlmToolSpec};
use crate::prompt::build_system_prompt;
use crate::providers::{normalize_model, provider_secret_name, DEFAULT_MAX_TOKENS};

/// Result of a provider connectivity test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTest {
    /// Normalized model id that was actually sent to the provider.
    pub model: String,
    /// Model reply (small connectivity prompt).
    pub reply: String,
}

/// Maximum number of characters of a tool output forwarded to the model.
const MAX_TOOL_OUTPUT_CHARS: usize = 4000;

/// Request used to create or update an LLM provider.
///
/// Providers are matched by `label`: posting the same label again updates the
/// existing record (keeping its id and stored API key unless a new key is
/// provided).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpsertProviderRequest {
    /// Provider family.
    pub kind: LlmProviderKind,
    /// User-visible label, unique across configured providers.
    pub label: String,
    /// Model identifier, e.g. `gpt-4.1-mini`.
    pub model: String,
    /// Base URL override. Required for `custom` and `opencode_go`.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key. When `Some`, it is stored encrypted and replaces the old key;
    /// when `None`, an existing key is kept.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Whether the provider can be used by the agent.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Stable per-conversation id for providers with session-affinity requirements.
///
/// The chat API is stateless, so the id is derived from the profile id and the
/// first user message (the frontend resends history each turn). This keeps every
/// turn of the same conversation on the same gateway route without persisting
/// conversations server-side.
fn conversation_session_id(profile_id: Uuid, messages: &[ChatMessage]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64-bit offset basis.
    for byte in profile_id.as_bytes().iter().chain(
        messages
            .iter()
            .find(|message| message.role == ChatRole::User)
            .map(|message| message.content.as_bytes())
            .unwrap_or(profile_id.to_string().as_bytes()),
    ) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("hs-{profile_id}-{hash:016x}")
}

/// Agent service: provider CRUD and the tool-calling chat loop.
pub struct AgentService {
    db: Arc<Database>,
    secrets: Arc<SecretStore>,
    profiles: Arc<ProfileService>,
    tools: Arc<ToolRegistry>,
    factory: Arc<dyn LlmClientFactory>,
    temperature: f32,
    max_tool_steps: u8,
    /// Explicitly selected provider. When unset (or no longer usable) the first
    /// enabled provider is used.
    pub active_provider: tokio::sync::RwLock<Option<Uuid>>,
}

impl std::fmt::Debug for AgentService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentService")
            .field("temperature", &self.temperature)
            .field("max_tool_steps", &self.max_tool_steps)
            .finish_non_exhaustive()
    }
}

impl AgentService {
    /// Create an agent service.
    ///
    /// `temperature` is clamped to `0.0..=2.0` and `max_tool_steps` to
    /// `1..=20` so a bad configuration cannot produce invalid provider
    /// requests.
    #[must_use]
    pub fn new(
        db: Arc<Database>,
        secrets: Arc<SecretStore>,
        profiles: Arc<ProfileService>,
        tools: Arc<ToolRegistry>,
        factory: Arc<dyn LlmClientFactory>,
        temperature: f32,
        max_tool_steps: u8,
    ) -> Self {
        let temperature = if temperature.is_finite() {
            temperature.clamp(0.0, 2.0)
        } else {
            0.0
        };
        Self {
            db,
            secrets,
            profiles,
            tools,
            factory,
            temperature,
            max_tool_steps: max_tool_steps.clamp(1, 20),
            active_provider: tokio::sync::RwLock::new(None),
        }
    }

    /// List configured providers ordered by creation time.
    ///
    /// # Errors
    /// Returns [`AgentError::Storage`] when the database cannot be read.
    pub fn list_providers(&self) -> AgentResult<Vec<ProviderRecord>> {
        let mut providers: Vec<ProviderRecord> = self.db.list_providers()?;
        // Lazily migrate records saved by older builds: OpenCode Go now has a
        // documented default base URL and expects raw model ids.
        for record in &mut providers {
            if record.kind == LlmProviderKind::OpencodeGo {
                let model = normalize_model(record.kind, &record.model);
                let base_url =
                    crate::providers::openai_base_url(record.kind, record.base_url.as_deref()).ok();
                if record.model != model || record.base_url != base_url {
                    record.model = model;
                    record.base_url = base_url;
                    self.db.save_provider(record)?;
                }
            }
        }
        providers.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.label.cmp(&right.label))
        });
        Ok(providers)
    }

    /// Create a provider, or update the provider with the same `label`.
    ///
    /// The API key never touches the provider record: it is written to the
    /// encrypted secret store under `llm.provider.<id>.api_key`.
    ///
    /// # Errors
    /// Returns [`AgentError::InvalidRequest`] for invalid fields and
    /// [`AgentError::Storage`] when persistence or encryption fails.
    pub fn upsert_provider(&self, request: UpsertProviderRequest) -> AgentResult<ProviderRecord> {
        let UpsertProviderRequest {
            kind,
            label,
            model,
            base_url,
            api_key,
            enabled,
        } = request;

        let label = normalize_required(&label, "label", 64)?;
        let model = normalize_model(kind, &normalize_required(&model, "model", 128)?);
        let base_url = match base_url {
            Some(url) if url.trim().is_empty() => None,
            Some(url) => {
                let url = url.trim();
                validation::http_url(url)?;
                Some(url.to_owned())
            }
            None => None,
        };
        if kind == LlmProviderKind::Custom && base_url.is_none() {
            return Err(AgentError::InvalidRequest(
                "the `custom` provider requires a base URL".to_owned(),
            ));
        }
        // Normalize the base URL (OpenCode Go gets its documented `/v1` suffix
        // and a pasted full endpoint is trimmed).
        let base_url = match kind {
            LlmProviderKind::OpencodeGo => Some(crate::providers::openai_base_url(
                kind,
                base_url.as_deref(),
            )?),
            _ => base_url,
        };
        let api_key = match api_key {
            Some(key) => {
                let key = key.trim();
                if key.is_empty() {
                    return Err(AgentError::InvalidRequest(
                        "api_key must not be empty when provided".to_owned(),
                    ));
                }
                Some(key.to_owned())
            }
            None => None,
        };

        let existing = self
            .db
            .list_providers()?
            .into_iter()
            .find(|record| record.label == label);
        let record = match existing {
            Some(mut record) => {
                record.kind = kind;
                record.model = model;
                record.base_url = base_url;
                record.enabled = enabled;
                record
            }
            None => {
                let id = Uuid::new_v4();
                ProviderRecord {
                    id,
                    kind,
                    label,
                    model,
                    base_url,
                    api_key_secret: provider_secret_name(id),
                    enabled,
                    created_at: Utc::now(),
                }
            }
        };

        if let Some(api_key) = api_key.as_deref() {
            self.secrets.set(&record.api_key_secret, api_key)?;
        }
        self.db.save_provider(&record)?;
        tracing::info!(
            provider_id = %record.id,
            kind = record.kind.id(),
            "LLM provider configured"
        );
        Ok(record)
    }

    /// Delete a provider and its encrypted API key.
    ///
    /// Returns `false` when the provider did not exist. Clears the active
    /// provider selection when it pointed at the deleted record.
    ///
    /// # Errors
    /// Returns [`AgentError::Storage`] when persistence fails.
    pub async fn delete_provider(&self, id: Uuid) -> AgentResult<bool> {
        let deleted = self.db.delete_provider(id)?;
        if deleted {
            if let Err(error) = self.secrets.delete(&provider_secret_name(id)) {
                tracing::warn!(
                    provider_id = %id,
                    error = %error,
                    "failed to remove the provider API key"
                );
            }
            let mut active = self.active_provider.write().await;
            if *active == Some(id) {
                *active = None;
            }
            tracing::info!(provider_id = %id, "LLM provider deleted");
        }
        Ok(deleted)
    }

    /// Select the provider used by [`AgentService::chat`].
    ///
    /// # Errors
    /// Returns [`AgentError::InvalidRequest`] when the provider does not exist.
    pub async fn set_active_provider(&self, id: Uuid) -> AgentResult<()> {
        let Some(record) = self.db.provider(id)? else {
            return Err(AgentError::InvalidRequest(format!(
                "provider {id} does not exist"
            )));
        };
        if !record.enabled {
            tracing::warn!(
                provider_id = %id,
                "selecting a disabled LLM provider; it will not be used until enabled"
            );
        }
        *self.active_provider.write().await = Some(id);
        Ok(())
    }

    /// Currently selected provider id, if one was selected explicitly.
    pub async fn active_provider(&self) -> Option<Uuid> {
        *self.active_provider.read().await
    }

    /// Test a configured provider with a minimal completion request.
    ///
    /// Used by the admin portal to surface the real provider error (invalid
    /// model id, wrong base URL, rejected key) instead of a generic failure.
    ///
    /// # Errors
    /// Returns [`AgentError`] when the provider is missing, disabled, has no
    /// API key, or the provider request fails.
    pub async fn test_provider(&self, id: Uuid) -> AgentResult<ProviderTest> {
        let provider = self
            .db
            .provider(id)?
            .ok_or_else(|| AgentError::InvalidRequest("provider not found".to_owned()))?;
        if !provider.enabled {
            return Err(AgentError::InvalidRequest(
                "the provider is disabled; enable it before testing".to_owned(),
            ));
        }
        let api_key = self.secrets.get(&provider.api_key_secret)?.ok_or_else(|| {
            AgentError::Provider(format!(
                "provider `{}` has no API key stored",
                provider.label
            ))
        })?;
        let client = self.factory.create(&provider, &api_key).await?;
        let model = normalize_model(provider.kind, &provider.model);
        let request = LlmRequest {
            model: model.clone(),
            system: "You are a connectivity test. Reply with the single word OK.".to_owned(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "ping".to_owned(),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }],
            tools: Vec::new(),
            temperature: 0.0,
            max_tokens: 16,
            session_id: Some(format!("provider-test-{id}")),
        };
        let response = client.complete(request).await?;
        Ok(ProviderTest {
            model,
            reply: response
                .content
                .unwrap_or_else(|| "(empty reply)".to_owned()),
        })
    }

    /// Run one agent turn: call the model, execute its tool calls and repeat
    /// until it produces a final answer.
    ///
    /// # Errors
    /// - [`AgentError::InvalidRequest`] when the conversation or profile is
    ///   invalid.
    /// - [`AgentError::NotConfigured`] when no enabled provider exists.
    /// - [`AgentError::Provider`] / [`AgentError::Unauthorized`] for provider
    ///   failures and tool-step exhaustion.
    /// - [`AgentError::Storage`] when secrets cannot be read.
    pub async fn chat(&self, request: AgentChatRequest) -> AgentResult<AgentChatResponse> {
        validate_chat_request(&request)?;
        let profile = self.profiles.get(request.profile_id)?;
        let provider = self.resolve_provider().await?;
        let api_key = self.secrets.get(&provider.api_key_secret)?.ok_or_else(|| {
            AgentError::Provider(format!(
                "provider `{}` has no API key stored; upsert it with an api_key",
                provider.label
            ))
        })?;
        let client = self.factory.create(&provider, &api_key).await?;
        let tools = if request.use_tools {
            self.llm_tools(&profile)
        } else {
            Vec::new()
        };
        let system = build_system_prompt(&profile);

        let mut messages = request.messages;
        let session_id = conversation_session_id(request.profile_id, &messages);
        let mut tool_trace: Vec<ToolTraceEntry> = Vec::new();
        let mut usage = TokenUsage::default();
        let mut model = provider.model.clone();
        let mut step: u8 = 0;

        loop {
            if step >= self.max_tool_steps {
                return Err(AgentError::Provider(format!(
                    "the model exceeded the maximum of {} tool steps without producing a final answer",
                    self.max_tool_steps
                )));
            }
            step += 1;

            let llm_request = LlmRequest {
                model: normalize_model(provider.kind, &provider.model),
                system: system.clone(),
                messages: messages.clone(),
                tools: tools.clone(),
                temperature: self.temperature,
                max_tokens: DEFAULT_MAX_TOKENS,
                session_id: Some(session_id.clone()),
            };
            let response = client.complete(llm_request).await?;
            usage.prompt_tokens += response.usage.prompt_tokens;
            usage.completion_tokens += response.usage.completion_tokens;
            if !response.model.is_empty() {
                model.clone_from(&response.model);
            }

            if response.tool_calls.is_empty() {
                return Ok(AgentChatResponse {
                    reply: response.content.unwrap_or_default(),
                    model,
                    tool_trace,
                    usage,
                });
            }

            let tool_calls = response.tool_calls;
            messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: response.content.unwrap_or_default(),
                tool_calls: tool_calls.clone(),
                tool_call_id: None,
            });
            for call in tool_calls {
                self.run_tool_call(&profile, &mut messages, &mut tool_trace, call)
                    .await;
            }
        }
    }

    async fn run_tool_call(
        &self,
        profile: &Profile,
        messages: &mut Vec<ChatMessage>,
        trace: &mut Vec<ToolTraceEntry>,
        call: ToolCall,
    ) {
        match self
            .tools
            .invoke(profile, &call.name, call.arguments.clone())
            .await
        {
            Ok(outcome) => {
                let content = truncate_tool_output(serialized_output(&outcome.output));
                trace.push(ToolTraceEntry {
                    tool: outcome.tool,
                    arguments: call.arguments,
                    allowed: true,
                    output: Some(content.clone()),
                    error: None,
                    duration_ms: outcome.duration_ms,
                });
                messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    content,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call.id),
                });
            }
            Err(error) => {
                let (allowed, message) = match &error {
                    ToolError::Forbidden { .. } => (
                        false,
                        format!(
                            "permission denied: the active profile may not use tool `{}`",
                            call.name
                        ),
                    ),
                    other => (true, other.to_string()),
                };
                tracing::info!(
                    profile_id = %profile.id,
                    tool = %call.name,
                    allowed,
                    "tool call reported to the model"
                );
                let content = serde_json::json!({
                    "error": message,
                    "tool": call.name,
                })
                .to_string();
                trace.push(ToolTraceEntry {
                    tool: call.name.clone(),
                    arguments: call.arguments,
                    allowed,
                    output: None,
                    error: Some(message),
                    duration_ms: 0,
                });
                messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    content,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call.id),
                });
            }
        }
    }

    fn llm_tools(&self, profile: &Profile) -> Vec<LlmToolSpec> {
        self.tools
            .list()
            .into_iter()
            .filter(|descriptor| profile.may_use_tool(&descriptor.name))
            .map(|descriptor| LlmToolSpec {
                name: descriptor.name,
                description: descriptor.description,
                parameters: descriptor.parameters,
            })
            .collect()
    }

    async fn resolve_provider(&self) -> AgentResult<ProviderRecord> {
        let active = *self.active_provider.read().await;
        if let Some(id) = active {
            match self.db.provider(id)? {
                Some(record) if record.enabled => return Ok(record),
                Some(_) => tracing::warn!(
                    provider_id = %id,
                    "the active LLM provider is disabled; falling back to the first enabled provider"
                ),
                None => tracing::warn!(
                    provider_id = %id,
                    "the active LLM provider no longer exists; falling back to the first enabled provider"
                ),
            }
        }
        self.list_providers()?
            .into_iter()
            .find(|record| record.enabled)
            .ok_or(AgentError::NotConfigured)
    }
}

fn validate_chat_request(request: &AgentChatRequest) -> AgentResult<()> {
    let Some(last) = request.messages.last() else {
        return Err(AgentError::InvalidRequest(
            "the conversation must contain at least one message".to_owned(),
        ));
    };
    if last.role != ChatRole::User {
        return Err(AgentError::InvalidRequest(
            "the last conversation message must be from the user".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_required(value: &str, field: &str, max_len: usize) -> AgentResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max_len {
        return Err(AgentError::InvalidRequest(format!(
            "{field} must be between 1 and {max_len} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(AgentError::InvalidRequest(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(trimmed.to_owned())
}

fn serialized_output(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn truncate_tool_output(output: String) -> String {
    if output.chars().count() <= MAX_TOOL_OUTPUT_CHARS {
        return output;
    }
    let mut truncated: String = output.chars().take(MAX_TOOL_OUTPUT_CHARS).collect();
    truncated.push_str("... [truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use hs_core::state::{SecretVault, Table};
    use hs_profiles::{CreateProfileRequest, ProfileRole, Tone};
    use hs_tools::{
        AudioControl, DomoticsControl, EntityLite, InstanceSummary, RuntimeControl, SystemControl,
        ToolPorts, WebhookClient,
    };
    use serde_json::json;

    use crate::llm::{LlmClient, LlmResponse};

    // -- scripted LLM client -------------------------------------------------

    enum ScriptStep {
        Reply(LlmResponse),
        Fail(AgentError),
    }

    struct ScriptedClient {
        requests: Mutex<Vec<LlmRequest>>,
        steps: Mutex<VecDeque<ScriptStep>>,
    }

    impl ScriptedClient {
        fn new(steps: Vec<ScriptStep>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                steps: Mutex::new(steps.into()),
            }
        }

        fn requests(&self) -> Vec<LlmRequest> {
            self.requests.lock().expect("request lock").clone()
        }
    }

    #[async_trait]
    impl LlmClient for ScriptedClient {
        async fn complete(&self, request: LlmRequest) -> AgentResult<LlmResponse> {
            self.requests.lock().expect("request lock").push(request);
            let next = self.steps.lock().expect("script lock").pop_front();
            match next {
                Some(ScriptStep::Reply(response)) => Ok(response),
                Some(ScriptStep::Fail(error)) => Err(error),
                None => Err(AgentError::Provider("scripted client exhausted".to_owned())),
            }
        }
    }

    struct ScriptedFactory {
        client: Arc<ScriptedClient>,
        created_for: Mutex<Vec<Uuid>>,
    }

    #[async_trait]
    impl LlmClientFactory for ScriptedFactory {
        async fn create(
            &self,
            provider: &ProviderRecord,
            _api_key: &str,
        ) -> AgentResult<Arc<dyn LlmClient>> {
            self.created_for
                .lock()
                .expect("factory lock")
                .push(provider.id);
            Ok(self.client.clone())
        }
    }

    // -- fake tool ports -----------------------------------------------------

    struct UnusedRuntime;

    #[async_trait]
    impl RuntimeControl for UnusedRuntime {
        async fn list_instances(&self) -> Result<Vec<InstanceSummary>, String> {
            Ok(Vec::new())
        }
        async fn start_instance(&self, _name: &str) -> Result<(), String> {
            Ok(())
        }
        async fn stop_instance(&self, _name: &str) -> Result<(), String> {
            Ok(())
        }
    }

    struct UnusedDomotics;

    #[async_trait]
    impl DomoticsControl for UnusedDomotics {
        async fn list_entities(&self) -> Result<Vec<EntityLite>, String> {
            Ok(Vec::new())
        }
        async fn call_service(
            &self,
            _domain: &str,
            _service: &str,
            _data: Value,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct UnusedAudio;

    #[async_trait]
    impl AudioControl for UnusedAudio {
        async fn play(&self) -> Result<(), String> {
            Ok(())
        }
        async fn pause(&self) -> Result<(), String> {
            Ok(())
        }
        async fn set_volume(&self, _percent: u8) -> Result<(), String> {
            Ok(())
        }
    }

    struct CountingSystem {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SystemControl for CountingSystem {
        async fn overview(&self) -> Result<Value, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "status": "ok", "cpu": { "usage_percent": 12.5 } }))
        }
    }

    struct UnusedWebhooks;

    #[async_trait]
    impl WebhookClient for UnusedWebhooks {
        async fn post(&self, _url: &str, _payload: Value) -> Result<u16, String> {
            Ok(204)
        }
    }

    fn tool_registry(system_calls: Arc<AtomicUsize>) -> Arc<ToolRegistry> {
        ToolRegistry::builtin(ToolPorts {
            runtime: Arc::new(UnusedRuntime),
            domotics: Arc::new(UnusedDomotics),
            audio: Arc::new(UnusedAudio),
            system: Arc::new(CountingSystem {
                calls: system_calls,
            }),
            webhooks: Arc::new(UnusedWebhooks),
        })
    }

    // -- fixture -------------------------------------------------------------

    struct Env {
        service: AgentService,
        db: Arc<Database>,
        secrets: Arc<SecretStore>,
        profiles: Arc<ProfileService>,
        scripted: Arc<ScriptedClient>,
        factory: Arc<ScriptedFactory>,
        system_calls: Arc<AtomicUsize>,
    }

    fn env(steps: Vec<ScriptStep>, max_tool_steps: u8) -> Env {
        let db = Arc::new(Database::open_in_memory().expect("in-memory database"));
        let secrets = Arc::new(SecretStore::new(Arc::clone(&db), SecretVault::generate()));
        let profiles = Arc::new(ProfileService::new(Arc::clone(&db), Arc::clone(&secrets)));
        let system_calls = Arc::new(AtomicUsize::new(0));
        let scripted = Arc::new(ScriptedClient::new(steps));
        let factory_inner = Arc::new(ScriptedFactory {
            client: Arc::clone(&scripted),
            created_for: Mutex::new(Vec::new()),
        });
        let factory: Arc<dyn LlmClientFactory> = factory_inner.clone();
        let service = AgentService::new(
            Arc::clone(&db),
            Arc::clone(&secrets),
            Arc::clone(&profiles),
            tool_registry(Arc::clone(&system_calls)),
            factory,
            0.2,
            max_tool_steps,
        );
        Env {
            service,
            db,
            secrets,
            profiles,
            scripted,
            factory: factory_inner,
            system_calls,
        }
    }

    impl Env {
        fn create_profile(&self, name: &str, tools: &[&str]) -> Profile {
            self.profiles
                .create(CreateProfileRequest {
                    name: name.to_owned(),
                    role: ProfileRole::Adult,
                    avatar: hs_core::models::Avatar::default(),
                    language: "es".to_owned(),
                    tone: Tone::Friendly,
                    allowed_tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
                })
                .expect("profile should be created")
        }

        fn create_provider(&self, label: &str, api_key: Option<&str>) -> ProviderRecord {
            self.service
                .upsert_provider(UpsertProviderRequest {
                    kind: LlmProviderKind::DeepSeek,
                    label: label.to_owned(),
                    model: "deepseek-chat".to_owned(),
                    base_url: None,
                    api_key: api_key.map(str::to_owned),
                    enabled: true,
                })
                .expect("provider should be created")
        }
    }

    fn text_reply(content: &str, prompt_tokens: u64, completion_tokens: u64) -> LlmResponse {
        LlmResponse {
            content: Some(content.to_owned()),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
            },
            model: "mock-model".to_owned(),
        }
    }

    fn tool_reply(
        id: &str,
        name: &str,
        arguments: Value,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> LlmResponse {
        LlmResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: id.to_owned(),
                name: name.to_owned(),
                arguments,
            }],
            usage: TokenUsage {
                prompt_tokens,
                completion_tokens,
            },
            model: "mock-model".to_owned(),
        }
    }

    fn user_request(profile_id: Uuid, text: &str) -> AgentChatRequest {
        AgentChatRequest {
            profile_id,
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: text.to_owned(),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }],
            use_tools: true,
        }
    }

    // -- tests ---------------------------------------------------------------

    #[tokio::test]
    async fn chat_executes_tool_call_and_returns_final_answer() {
        let env = env(
            vec![
                ScriptStep::Reply(tool_reply("call_1", "system.status", json!({}), 10, 5)),
                ScriptStep::Reply(text_reply("The system is healthy.", 20, 7)),
            ],
            4,
        );
        let profile = env.create_profile("Mamá", &["system.status"]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));

        let response = env
            .service
            .chat(user_request(profile.id, "How is the system?"))
            .await
            .expect("chat should succeed");

        assert_eq!(response.reply, "The system is healthy.");
        assert_eq!(response.model, "mock-model");
        assert_eq!(response.tool_trace.len(), 1);
        assert!(response.tool_trace[0].allowed);
        assert_eq!(response.tool_trace[0].tool, "system.status");
        assert!(response.tool_trace[0]
            .output
            .as_deref()
            .is_some_and(|output| output.contains("\"status\":\"ok\"")));
        assert!(response.tool_trace[0].error.is_none());
        assert_eq!(response.usage.prompt_tokens, 30);
        assert_eq!(response.usage.completion_tokens, 12);
        assert_eq!(env.system_calls.load(Ordering::SeqCst), 1);

        let requests = env.scripted.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].tools.len(), 1);
        assert_eq!(requests[0].tools[0].name, "system.status");
        assert_eq!(requests[1].messages.len(), 3);
        assert_eq!(requests[1].messages[1].role, ChatRole::Assistant);
        assert_eq!(requests[1].messages[1].tool_calls.len(), 1);
        assert_eq!(requests[1].messages[2].role, ChatRole::Tool);
        assert_eq!(
            requests[1].messages[2].tool_call_id.as_deref(),
            Some("call_1")
        );
    }

    #[tokio::test]
    async fn denied_tool_is_not_executed_and_is_reported_to_the_model() {
        let env = env(
            vec![
                ScriptStep::Reply(tool_reply("call_1", "system.status", json!({}), 4, 2)),
                ScriptStep::Reply(text_reply("I cannot do that.", 6, 3)),
            ],
            4,
        );
        let profile = env.create_profile("Invitado", &[]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));

        let response = env
            .service
            .chat(user_request(profile.id, "Check the system"))
            .await
            .expect("chat should still answer");

        assert_eq!(response.reply, "I cannot do that.");
        assert_eq!(response.tool_trace.len(), 1);
        assert!(!response.tool_trace[0].allowed);
        assert!(response.tool_trace[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("permission denied")));
        assert_eq!(env.system_calls.load(Ordering::SeqCst), 0);
        let requests = env.scripted.requests();
        assert!(requests[0].tools.is_empty());
        assert_eq!(requests[1].messages[2].role, ChatRole::Tool);
        assert!(requests[1].messages[2]
            .content
            .contains("permission denied"));
    }

    #[tokio::test]
    async fn provider_errors_are_mapped() {
        let env = env(
            vec![ScriptStep::Fail(AgentError::Provider(
                "connection refused".to_owned(),
            ))],
            4,
        );
        let profile = env.create_profile("Mamá", &[]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));

        let error = env
            .service
            .chat(user_request(profile.id, "hello"))
            .await
            .expect_err("provider failure must surface");
        assert!(matches!(error, AgentError::Provider(_)), "{error:?}");
    }

    #[tokio::test]
    async fn no_provider_is_reported_as_not_configured() {
        let env = env(Vec::new(), 4);
        let profile = env.create_profile("Mamá", &[]);
        let error = env
            .service
            .chat(user_request(profile.id, "hello"))
            .await
            .expect_err("no provider configured");
        assert!(matches!(error, AgentError::NotConfigured), "{error:?}");
    }

    #[tokio::test]
    async fn missing_api_key_is_reported() {
        let env = env(Vec::new(), 4);
        let profile = env.create_profile("Mamá", &[]);
        env.create_provider("DeepSeek", None);
        let error = env
            .service
            .chat(user_request(profile.id, "hello"))
            .await
            .expect_err("provider without key cannot be used");
        assert!(matches!(error, AgentError::Provider(_)), "{error:?}");
    }

    #[tokio::test]
    async fn invalid_chat_requests_are_rejected() {
        let env = env(Vec::new(), 4);
        let profile = env.create_profile("Mamá", &[]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));

        let mut request = user_request(profile.id, "hello");
        request.messages.clear();
        let error = env
            .service
            .chat(request)
            .await
            .expect_err("empty conversation must be rejected");
        assert!(matches!(error, AgentError::InvalidRequest(_)), "{error:?}");

        let mut request = user_request(profile.id, "hello");
        request.messages[0].role = ChatRole::Assistant;
        let error = env
            .service
            .chat(request)
            .await
            .expect_err("last message must be from the user");
        assert!(matches!(error, AgentError::InvalidRequest(_)), "{error:?}");

        let error = env
            .service
            .chat(user_request(Uuid::new_v4(), "hello"))
            .await
            .expect_err("unknown profile must be rejected");
        assert!(matches!(error, AgentError::InvalidRequest(_)), "{error:?}");
    }

    #[tokio::test]
    async fn system_prompt_is_profile_aware_and_tools_are_filtered() {
        let env = env(vec![ScriptStep::Reply(text_reply("Hola", 1, 1))], 4);
        let profile = env.create_profile("Mamá", &["system.status"]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));

        let mut request = user_request(profile.id, "hola");
        request.use_tools = false;
        env.service
            .chat(request)
            .await
            .expect("chat should succeed");

        let requests = env.scripted.requests();
        assert!(requests[0].system.contains("Mamá"));
        assert!(requests[0].system.contains("Spanish"));
        assert!(
            requests[0].tools.is_empty(),
            "use_tools=false must send no tools"
        );
        assert_eq!(requests[0].messages.len(), 1);
        assert_eq!(requests[0].messages[0].content, "hola");
    }

    #[tokio::test]
    async fn conversation_history_is_preserved() {
        let env = env(vec![ScriptStep::Reply(text_reply("Done", 1, 1))], 4);
        let profile = env.create_profile("Mamá", &[]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));

        let request = AgentChatRequest {
            profile_id: profile.id,
            messages: vec![
                ChatMessage {
                    role: ChatRole::User,
                    content: "first".to_owned(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: "second".to_owned(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: "third".to_owned(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
            ],
            use_tools: true,
        };
        env.service
            .chat(request)
            .await
            .expect("chat should succeed");

        let requests = env.scripted.requests();
        assert_eq!(requests[0].messages.len(), 3);
        assert_eq!(requests[0].messages[0].content, "first");
        assert_eq!(requests[0].messages[1].content, "second");
        assert_eq!(requests[0].messages[2].content, "third");
    }

    #[tokio::test]
    async fn max_tool_steps_is_enforced() {
        let env = env(
            vec![
                ScriptStep::Reply(tool_reply("call_1", "system.status", json!({}), 1, 1)),
                ScriptStep::Reply(tool_reply("call_2", "system.status", json!({}), 1, 1)),
            ],
            1,
        );
        let profile = env.create_profile("Mamá", &["system.status"]);
        env.create_provider("DeepSeek", Some("sk-test-secret"));
        let error = env
            .service
            .chat(user_request(profile.id, "loop forever"))
            .await
            .expect_err("tool step limit must be enforced");
        assert!(
            matches!(&error, AgentError::Provider(message) if message.contains("maximum")),
            "unexpected error: {error:?}"
        );
        assert_eq!(env.system_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn active_provider_selection_prefers_the_selected_provider() {
        let env = env(
            vec![
                ScriptStep::Reply(text_reply("ok", 1, 1)),
                ScriptStep::Reply(text_reply("ok again", 1, 1)),
            ],
            4,
        );
        let profile = env.create_profile("Mamá", &[]);
        let first = env.create_provider("Primary", Some("sk-first"));
        let second = env.create_provider("Secondary", Some("sk-second"));

        assert!(env.service.active_provider().await.is_none());
        env.service
            .set_active_provider(second.id)
            .await
            .expect("provider exists");
        assert_eq!(env.service.active_provider().await, Some(second.id));
        env.service
            .chat(user_request(profile.id, "hola"))
            .await
            .expect("chat should succeed");
        assert_eq!(
            env.factory.created_for.lock().expect("lock").as_slice(),
            &[second.id]
        );

        env.service
            .delete_provider(second.id)
            .await
            .expect("delete should succeed");
        assert!(env.service.active_provider().await.is_none());
        env.service
            .chat(user_request(profile.id, "hola"))
            .await
            .expect("chat should fall back to the first enabled provider");
        assert_eq!(
            env.factory.created_for.lock().expect("lock").as_slice(),
            &[second.id, first.id]
        );
    }

    #[tokio::test]
    async fn provider_crud_keeps_the_api_key_in_the_secret_store_only() {
        let env = env(Vec::new(), 4);
        let record = env.create_provider("DeepSeek", Some("sk-super-secret"));
        assert!(record.api_key_secret.starts_with("llm.provider."));
        assert!(record.api_key_secret.ends_with(".api_key"));

        let persisted: ProviderRecord = env
            .db
            .get(Table::Providers, &record.id.to_string())
            .expect("database read")
            .expect("provider must be persisted");
        let serialized = serde_json::to_string(&persisted).expect("serialize provider");
        assert!(!serialized.contains("sk-super-secret"));
        assert_eq!(
            env.secrets
                .get(&record.api_key_secret)
                .expect("secret read")
                .expect("secret must exist")
                .as_str(),
            "sk-super-secret"
        );

        // Upserting the same label updates the same record and rotates the key.
        let updated = env
            .service
            .upsert_provider(UpsertProviderRequest {
                kind: LlmProviderKind::DeepSeek,
                label: "DeepSeek".to_owned(),
                model: "deepseek-reasoner".to_owned(),
                base_url: None,
                api_key: Some("sk-rotated".to_owned()),
                enabled: true,
            })
            .expect("upsert should succeed");
        assert_eq!(updated.id, record.id);
        assert_eq!(updated.model, "deepseek-reasoner");
        assert_eq!(env.service.list_providers().expect("list").len(), 1);
        assert_eq!(
            env.secrets
                .get(&record.api_key_secret)
                .expect("secret read")
                .expect("secret must exist")
                .as_str(),
            "sk-rotated"
        );

        // Upserting without a key keeps the stored key.
        env.service
            .upsert_provider(UpsertProviderRequest {
                kind: LlmProviderKind::DeepSeek,
                label: "DeepSeek".to_owned(),
                model: "deepseek-chat".to_owned(),
                base_url: None,
                api_key: None,
                enabled: true,
            })
            .expect("upsert should succeed");
        assert!(env
            .secrets
            .get(&record.api_key_secret)
            .expect("secret read")
            .is_some());

        assert!(env
            .service
            .delete_provider(record.id)
            .await
            .expect("delete should succeed"));
        assert!(env.service.list_providers().expect("list").is_empty());
        assert!(!env
            .secrets
            .contains(&record.api_key_secret)
            .expect("secret read"));
        assert!(!env
            .service
            .delete_provider(record.id)
            .await
            .expect("second delete should succeed"));
    }

    #[tokio::test]
    async fn upsert_provider_validates_fields() {
        let env = env(Vec::new(), 4);
        let base = UpsertProviderRequest {
            kind: LlmProviderKind::DeepSeek,
            label: "Valid".to_owned(),
            model: "deepseek-chat".to_owned(),
            base_url: None,
            api_key: Some("sk-key".to_owned()),
            enabled: true,
        };

        let mut request = base.clone();
        request.label = "  ".to_owned();
        assert!(matches!(
            env.service
                .upsert_provider(request)
                .expect_err("empty label"),
            AgentError::InvalidRequest(_)
        ));

        let mut request = base.clone();
        request.model = String::new();
        assert!(env.service.upsert_provider(request).is_err());

        let mut request = base.clone();
        request.base_url = Some("not-a-url".to_owned());
        assert!(env.service.upsert_provider(request).is_err());

        let mut request = base.clone();
        request.api_key = Some("   ".to_owned());
        assert!(env.service.upsert_provider(request).is_err());

        let mut request = base.clone();
        request.kind = LlmProviderKind::Custom;
        assert!(env.service.upsert_provider(request).is_err());
        let mut request = base;
        request.kind = LlmProviderKind::Custom;
        request.base_url = Some("https://llm.example.test/v1".to_owned());
        assert!(env.service.upsert_provider(request).is_ok());
    }
}
