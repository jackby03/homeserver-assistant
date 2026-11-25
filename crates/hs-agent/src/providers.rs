//! Concrete LLM provider adapters.
//!
//! Two wire protocols are supported:
//!
//! - **OpenAI-compatible** (`openai`, `deepseek`, `opencode_go`, `custom`):
//!   `POST {base}/chat/completions`.
//! - **Anthropic Messages** (`anthropic`): `POST {base}/v1/messages` with an
//!   `x-api-key` header and the `2023-06-01` API version.
//!
//! Request building and response parsing are pure functions so they can be
//! unit tested with fixture JSON, without network access. [`DefaultFactory`]
//! creates the real HTTP clients; tests inject a scripted
//! [`crate::llm::LlmClientFactory`] instead.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use hs_core::config::LlmProviderKind;
use hs_core::models::{ChatMessage, ChatRole, ProviderRecord, TokenUsage, ToolCall};
use serde_json::{json, Value};

use crate::error::{AgentError, AgentResult};
use crate::llm::{LlmClient, LlmClientFactory, LlmRequest, LlmResponse};

/// Default base URL for OpenAI.
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
/// Default base URL for DeepSeek.
pub const DEFAULT_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";
/// Default base URL for the Anthropic Messages API.
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
/// Default base URL for the OpenCode Zen "Go" OpenAI-compatible gateway.
pub const DEFAULT_OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go/v1";
/// `anthropic-version` header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Default completion token budget used by [`crate::AgentService`].
pub const DEFAULT_MAX_TOKENS: u32 = 1024;
/// Default HTTP timeout for provider requests.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Production factory that builds `reqwest`-backed provider clients.
///
/// The HTTP client is created lazily on first use, which keeps
/// [`DefaultFactory::default`] infallible and lets dependency-free unit tests
/// construct the factory without touching TLS initialization.
pub struct DefaultFactory {
    http: OnceLock<reqwest::Client>,
}

impl Default for DefaultFactory {
    fn default() -> Self {
        Self {
            http: OnceLock::new(),
        }
    }
}

impl std::fmt::Debug for DefaultFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DefaultFactory")
            .field("http_initialized", &self.http.get().is_some())
            .finish_non_exhaustive()
    }
}

impl DefaultFactory {
    /// Create a factory with a lazily initialized HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a factory that reuses an existing `reqwest` client.
    ///
    /// Useful when the daemon wants to share a connection pool, proxy settings
    /// or a custom TLS configuration across integrations.
    #[must_use]
    pub fn with_client(client: reqwest::Client) -> Self {
        let factory = Self::default();
        let _ = factory.http.set(client);
        factory
    }

    fn client(&self) -> AgentResult<reqwest::Client> {
        if let Some(client) = self.http.get() {
            return Ok(client.clone());
        }
        let client = reqwest::Client::builder()
            .user_agent(concat!("HomeServerAssistant/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
            .build()
            .map_err(|error| {
                AgentError::Provider(format!("failed to build the HTTP client: {error}"))
            })?;
        let _ = self.http.set(client);
        self.http
            .get()
            .cloned()
            .ok_or_else(|| AgentError::Provider("failed to initialize the HTTP client".to_owned()))
    }
}

#[async_trait]
impl LlmClientFactory for DefaultFactory {
    async fn create(
        &self,
        provider: &ProviderRecord,
        api_key: &str,
    ) -> AgentResult<Arc<dyn LlmClient>> {
        let http = self.client()?;
        if provider.kind.is_openai_compatible() {
            let base = openai_base_url(provider.kind, provider.base_url.as_deref())?;
            Ok(Arc::new(OpenAiCompatibleClient {
                http,
                url: openai_chat_completions_url(&base),
                api_key: api_key.to_owned(),
                opencode_go: provider.kind == LlmProviderKind::OpencodeGo,
            }))
        } else {
            let base = anthropic_base_url(provider.base_url.as_deref())?;
            Ok(Arc::new(AnthropicClient {
                http,
                url: anthropic_messages_url(&base),
                api_key: api_key.to_owned(),
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

/// Resolve the OpenAI-compatible base URL for `kind`.
///
/// `custom` and `opencode_go` have no default and require an explicit base URL.
///
/// # Errors
/// Returns [`AgentError::Provider`] when the base URL is missing or invalid.
pub fn openai_base_url(kind: LlmProviderKind, base_url: Option<&str>) -> AgentResult<String> {
    let base = match kind {
        LlmProviderKind::OpenAi => base_url.unwrap_or(DEFAULT_OPENAI_BASE_URL),
        LlmProviderKind::DeepSeek => base_url.unwrap_or(DEFAULT_DEEPSEEK_BASE_URL),
        LlmProviderKind::OpencodeGo => base_url.unwrap_or(DEFAULT_OPENCODE_GO_BASE_URL),
        LlmProviderKind::Custom => base_url.ok_or_else(|| {
            AgentError::Provider(format!(
                "the `{}` provider requires an explicit base URL",
                kind.id()
            ))
        })?,
        LlmProviderKind::Anthropic => {
            return Err(AgentError::Provider(
                "the `anthropic` provider uses the Messages API, not the OpenAI protocol"
                    .to_owned(),
            ));
        }
    };
    let base = normalize_openai_base(kind, base);
    validated_base_url(&base)
}

/// Normalize common provider-base mistakes before adding `/chat/completions`.
///
/// - OpenCode Zen documents `https://opencode.ai/zen/go/v1`; pasting the
///   console/UI URL without `/v1` is a common mistake, so it is appended.
/// - A full endpoint pasted by mistake (`.../chat/completions`) is trimmed.
fn normalize_openai_base(kind: LlmProviderKind, base: &str) -> String {
    let mut base = base.trim().trim_end_matches('/').to_owned();
    if let Some(stripped) = base.strip_suffix("/chat/completions") {
        base = stripped.trim_end_matches('/').to_owned();
    }
    if kind == LlmProviderKind::OpencodeGo {
        if base.ends_with("/zen/go") {
            base.push_str("/v1");
        } else if let Some(idx) = base.find("/zen/go/") {
            let tail = &base[idx + "/zen/go/".len()..];
            if !tail.starts_with("v1") {
                base = format!("{}/v1", base.trim_end_matches('/'));
            }
        }
    }
    base
}

/// Normalize the model id for a provider.
///
/// OpenCode Zen's model list exposes raw ids (`deepseek-v4.1-flash`); the
/// OpenCode CLI may show them prefixed (`opencode-go/deepseek-v4.1-flash`).
/// The gateway expects the raw id, so known UI prefixes are stripped.
#[must_use]
pub fn normalize_model(kind: LlmProviderKind, model: &str) -> String {
    let model = model.trim();
    if kind == LlmProviderKind::OpencodeGo {
        for prefix in ["opencode-go/", "opencode/"] {
            if let Some(stripped) = model.strip_prefix(prefix) {
                return stripped.trim().to_owned();
            }
        }
    }
    model.to_owned()
}

/// Resolve the Anthropic API root for a provider record.
///
/// # Errors
/// Returns [`AgentError::Provider`] when the base URL is invalid.
pub fn anthropic_base_url(base_url: Option<&str>) -> AgentResult<String> {
    validated_base_url(base_url.unwrap_or(DEFAULT_ANTHROPIC_BASE_URL))
}

/// Full OpenAI-compatible chat completions URL.
#[must_use]
pub fn openai_chat_completions_url(base: &str) -> String {
    format!("{}/chat/completions", base.trim_end_matches('/'))
}

/// Full Anthropic Messages API URL.
#[must_use]
pub fn anthropic_messages_url(base: &str) -> String {
    format!("{}/v1/messages", base.trim_end_matches('/'))
}

fn validated_base_url(base: &str) -> AgentResult<String> {
    hs_core::validation::http_url(base)
        .map_err(|error| AgentError::Provider(format!("invalid provider base URL: {error}")))?;
    Ok(base.trim_end_matches('/').to_owned())
}

/// Stable secret name that stores the API key of `provider_id`.
#[must_use]
pub fn provider_secret_name(provider_id: uuid::Uuid) -> String {
    format!("llm.provider.{provider_id}.api_key")
}

// ---------------------------------------------------------------------------
// OpenAI-compatible adapters
// ---------------------------------------------------------------------------

struct OpenAiCompatibleClient {
    http: reqwest::Client,
    url: String,
    api_key: String,
    /// OpenCode Zen Go requires a stable per-conversation session header.
    opencode_go: bool,
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn complete(&self, request: LlmRequest) -> AgentResult<LlmResponse> {
        let body = openai_request_body(&request);
        let mut builder = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .json(&body);
        if self.opencode_go {
            let session = request
                .session_id
                .as_deref()
                .filter(|session| !session.trim().is_empty())
                .unwrap_or("homeserver-assistant");
            builder = builder.header("x-opencode-session", session);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| AgentError::Provider(format!("LLM request failed: {error}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            AgentError::Provider(format!("failed to read the LLM response: {error}"))
        })?;
        ensure_success(status, &body)?;
        parse_openai_response(&body)
    }
}

struct AnthropicClient {
    http: reqwest::Client,
    url: String,
    api_key: String,
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, request: LlmRequest) -> AgentResult<LlmResponse> {
        let body = anthropic_request_body(&request);
        let response = self
            .http
            .post(&self.url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|error| AgentError::Provider(format!("LLM request failed: {error}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|error| {
            AgentError::Provider(format!("failed to read the LLM response: {error}"))
        })?;
        ensure_success(status, &body)?;
        parse_anthropic_response(&body)
    }
}

fn ensure_success(status: reqwest::StatusCode, body: &str) -> AgentResult<()> {
    if status.is_success() {
        return Ok(());
    }
    // Include a short, sanitized excerpt of the provider error so the admin
    // "test provider" action can explain what is wrong (for example an invalid
    // model id or a wrong base URL). The request body/API key are never echoed.
    let detail = body
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let detail = if detail.chars().count() > 400 {
        format!("{}…", detail.chars().take(400).collect::<String>())
    } else {
        detail
    };
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(AgentError::Unauthorized(format!(
            "the provider rejected the API key (HTTP {status}){}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }
    Err(AgentError::Provider(format!(
        "the provider returned HTTP {status}{}",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    )))
}

// ---------------------------------------------------------------------------
// OpenAI-compatible request/response mapping
// ---------------------------------------------------------------------------

/// Build the JSON body of an OpenAI-compatible `/chat/completions` request.
#[must_use]
pub fn openai_request_body(request: &LlmRequest) -> Value {
    let mut body = json!({
        "model": request.model.clone(),
        "messages": openai_messages(&request.system, &request.messages),
        "temperature": request.temperature,
        "max_tokens": request.max_tokens,
    });
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name.clone(),
                        "description": tool.description.clone(),
                        "parameters": tool.parameters.clone()
                    }
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = json!("auto");
    }
    body
}

fn openai_messages(system: &str, messages: &[ChatMessage]) -> Vec<Value> {
    let mut output = Vec::with_capacity(messages.len() + 1);
    output.push(json!({ "role": "system", "content": system }));
    for message in messages {
        match message.role {
            ChatRole::System => {
                output.push(json!({ "role": "system", "content": message.content.clone() }));
            }
            ChatRole::User => {
                output.push(json!({ "role": "user", "content": message.content.clone() }));
            }
            ChatRole::Assistant => {
                let content = if message.content.is_empty() {
                    Value::Null
                } else {
                    Value::String(message.content.clone())
                };
                if message.tool_calls.is_empty() {
                    output.push(json!({ "role": "assistant", "content": content }));
                } else {
                    let calls: Vec<Value> = message
                        .tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id.clone(),
                                "type": "function",
                                "function": {
                                    "name": call.name.clone(),
                                    "arguments": call.arguments.to_string()
                                }
                            })
                        })
                        .collect();
                    output.push(json!({
                        "role": "assistant",
                        "content": content,
                        "tool_calls": calls
                    }));
                }
            }
            ChatRole::Tool => {
                output.push(json!({
                    "role": "tool",
                    "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content.clone()
                }));
            }
        }
    }
    output
}

/// Parse an OpenAI-compatible `/chat/completions` response body.
///
/// # Errors
/// Returns [`AgentError::Provider`] when the body is not a valid response or a
/// tool call carries malformed JSON arguments.
pub fn parse_openai_response(body: &str) -> AgentResult<LlmResponse> {
    let value = parse_json(body, "OpenAI-compatible")?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| {
            AgentError::Provider("OpenAI-compatible response has no choices".to_owned())
        })?;
    let message = choice.get("message").ok_or_else(|| {
        AgentError::Provider("OpenAI-compatible response has no message".to_owned())
    })?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            tool_calls.push(parse_openai_tool_call(call)?);
        }
    }

    Ok(LlmResponse {
        content,
        tool_calls,
        usage: openai_usage(value.get("usage")),
        model,
    })
}

fn parse_openai_tool_call(call: &Value) -> AgentResult<ToolCall> {
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentError::Provider("OpenAI-compatible tool call has no `id`".to_owned()))?
        .to_owned();
    let function = call.get("function").ok_or_else(|| {
        AgentError::Provider(format!(
            "OpenAI-compatible tool call `{id}` has no function"
        ))
    })?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AgentError::Provider(format!("OpenAI-compatible tool call `{id}` has no name"))
        })?
        .to_owned();
    let raw_arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("null");
    let arguments = if raw_arguments.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(raw_arguments).map_err(|error| {
            AgentError::Provider(format!(
                "tool call `{name}` has invalid JSON arguments: {error}"
            ))
        })?
    };
    Ok(ToolCall {
        id,
        name,
        arguments,
    })
}

fn openai_usage(usage: Option<&Value>) -> TokenUsage {
    TokenUsage {
        prompt_tokens: usage
            .and_then(|usage| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: usage
            .and_then(|usage| usage.get("completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

// ---------------------------------------------------------------------------
// Anthropic request/response mapping
// ---------------------------------------------------------------------------

/// Build the JSON body of an Anthropic `/v1/messages` request.
#[must_use]
pub fn anthropic_request_body(request: &LlmRequest) -> Value {
    let mut body = json!({
        "model": request.model.clone(),
        "system": request.system.clone(),
        "messages": anthropic_messages(&request.messages),
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
    });
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name.clone(),
                    "description": tool.description.clone(),
                    "input_schema": tool.parameters.clone()
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    body
}

fn anthropic_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let mut output: Vec<Value> = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let message = &messages[index];
        match message.role {
            ChatRole::System => {
                index += 1;
            }
            ChatRole::User => {
                output.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": message.content.clone() }]
                }));
                index += 1;
            }
            ChatRole::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if !message.content.is_empty() {
                    blocks.push(json!({ "type": "text", "text": message.content.clone() }));
                }
                for call in &message.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id.clone(),
                        "name": call.name.clone(),
                        "input": call.arguments.clone()
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                output.push(json!({ "role": "assistant", "content": blocks }));
                index += 1;
            }
            ChatRole::Tool => {
                let mut blocks: Vec<Value> = Vec::new();
                while index < messages.len() && messages[index].role == ChatRole::Tool {
                    let tool_message = &messages[index];
                    blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_message.tool_call_id.clone().unwrap_or_default(),
                        "content": tool_message.content.clone()
                    }));
                    index += 1;
                }
                output.push(json!({ "role": "user", "content": blocks }));
            }
        }
    }
    output
}

/// Parse an Anthropic `/v1/messages` response body.
///
/// # Errors
/// Returns [`AgentError::Provider`] when the body is not a valid response or a
/// `tool_use` block is incomplete.
pub fn parse_anthropic_response(body: &str) -> AgentResult<LlmResponse> {
    let value = parse_json(body, "Anthropic")?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AgentError::Provider("Anthropic response has no content blocks".to_owned())
        })?;

    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    text_parts.push(text.to_owned());
                }
            }
            Some("tool_use") => {
                let id = block.get("id").and_then(Value::as_str).ok_or_else(|| {
                    AgentError::Provider("Anthropic tool_use block has no `id`".to_owned())
                })?;
                let name = block.get("name").and_then(Value::as_str).ok_or_else(|| {
                    AgentError::Provider(format!("Anthropic tool_use block `{id}` has no name"))
                })?;
                let arguments = block.get("input").cloned().unwrap_or_else(|| json!({}));
                tool_calls.push(ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments,
                });
            }
            _ => {}
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };
    let usage = TokenUsage {
        prompt_tokens: value
            .get("usage")
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: value
            .get("usage")
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };
    Ok(LlmResponse {
        content,
        tool_calls,
        usage,
        model,
    })
}

fn parse_json(body: &str, provider: &str) -> AgentResult<Value> {
    serde_json::from_str(body)
        .map_err(|error| AgentError::Provider(format!("invalid {provider} response: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hs_core::models::ToolCall;
    use serde_json::json;
    use uuid::Uuid;

    const OPENAI_TEXT_FIXTURE: &str = r#"{
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "model": "gpt-4.1-mini",
        "choices": [
            {
                "index": 0,
                "message": { "role": "assistant", "content": "Hola, soy tu asistente." },
                "finish_reason": "stop"
            }
        ],
        "usage": { "prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17 }
    }"#;

    const OPENAI_TOOL_FIXTURE: &str = r#"{
        "id": "chatcmpl-2",
        "model": "deepseek-chat",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "music.set_volume",
                                "arguments": "{\"percent\": 35}"
                            }
                        }
                    ]
                },
                "finish_reason": "tool_calls"
            }
        ],
        "usage": { "prompt_tokens": 20, "completion_tokens": 7 }
    }"#;

    const ANTHROPIC_FIXTURE: &str = r#"{
        "id": "msg_01",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4",
        "content": [
            { "type": "text", "text": "Voy a comprobarlo." },
            {
                "type": "tool_use",
                "id": "toolu_01",
                "name": "system.status",
                "input": { "detail": "full" }
            }
        ],
        "stop_reason": "tool_use",
        "usage": { "input_tokens": 30, "output_tokens": 9 }
    }"#;

    fn request() -> LlmRequest {
        LlmRequest {
            model: "gpt-4.1-mini".to_owned(),
            session_id: None,
            system: "You are helpful.".to_owned(),
            messages: vec![
                ChatMessage {
                    role: ChatRole::User,
                    content: "Hola".to_owned(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_1".to_owned(),
                        name: "music.play".to_owned(),
                        arguments: json!({}),
                    }],
                    tool_call_id: None,
                },
                ChatMessage {
                    role: ChatRole::Tool,
                    content: "{\"status\":\"playing\"}".to_owned(),
                    tool_calls: Vec::new(),
                    tool_call_id: Some("call_1".to_owned()),
                },
            ],
            tools: vec![crate::llm::LlmToolSpec {
                name: "music.play".to_owned(),
                description: "Play music".to_owned(),
                parameters: json!({ "type": "object", "properties": {} }),
            }],
            temperature: 0.5,
            max_tokens: 256,
        }
    }

    fn provider(kind: LlmProviderKind, base_url: Option<&str>) -> ProviderRecord {
        ProviderRecord {
            id: Uuid::new_v4(),
            kind,
            label: "Test".to_owned(),
            model: "test-model".to_owned(),
            base_url: base_url.map(str::to_owned),
            api_key_secret: provider_secret_name(Uuid::new_v4()),
            enabled: true,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn parses_openai_text_response() {
        let response = parse_openai_response(OPENAI_TEXT_FIXTURE).expect("valid fixture");
        assert_eq!(response.content.as_deref(), Some("Hola, soy tu asistente."));
        assert!(response.tool_calls.is_empty());
        assert_eq!(response.model, "gpt-4.1-mini");
        assert_eq!(response.usage.prompt_tokens, 12);
        assert_eq!(response.usage.completion_tokens, 5);
    }

    #[test]
    fn parses_openai_tool_calls_with_json_arguments() {
        let response = parse_openai_response(OPENAI_TOOL_FIXTURE).expect("valid fixture");
        assert!(response.content.is_none());
        assert_eq!(response.tool_calls.len(), 1);
        let call = &response.tool_calls[0];
        assert_eq!(call.id, "call_abc");
        assert_eq!(call.name, "music.set_volume");
        assert_eq!(call.arguments, json!({ "percent": 35 }));
        assert_eq!(response.usage.prompt_tokens, 20);
    }

    #[test]
    fn rejects_malformed_openai_responses() {
        assert!(parse_openai_response("not json").is_err());
        assert!(parse_openai_response("{}").is_err());
        let invalid_arguments = r#"{
            "choices": [{ "message": { "tool_calls": [{
                "id": "call_1",
                "function": { "name": "music.play", "arguments": "{not-json" }
            }] } }]
        }"#;
        let error = parse_openai_response(invalid_arguments).expect_err("must fail");
        assert!(matches!(error, AgentError::Provider(_)), "{error:?}");
    }

    #[test]
    fn openai_request_body_uses_the_function_tool_format() {
        let body = openai_request_body(&request());
        assert_eq!(body["model"], "gpt-4.1-mini");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are helpful.");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["messages"][2]["content"], Value::Null);
        assert_eq!(
            body["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "music.play");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["max_tokens"], 256);
    }

    #[test]
    fn parses_anthropic_text_and_tool_use_blocks() {
        let response = parse_anthropic_response(ANTHROPIC_FIXTURE).expect("valid fixture");
        assert_eq!(response.content.as_deref(), Some("Voy a comprobarlo."));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "toolu_01");
        assert_eq!(response.tool_calls[0].name, "system.status");
        assert_eq!(
            response.tool_calls[0].arguments,
            json!({ "detail": "full" })
        );
        assert_eq!(response.model, "claude-sonnet-4");
        assert_eq!(response.usage.prompt_tokens, 30);
        assert_eq!(response.usage.completion_tokens, 9);
    }

    #[test]
    fn rejects_malformed_anthropic_responses() {
        assert!(parse_anthropic_response("[]").is_err());
        assert!(parse_anthropic_response("{}").is_err());
        let incomplete = r#"{ "content": [{ "type": "tool_use", "id": "toolu_x" }] }"#;
        assert!(parse_anthropic_response(incomplete).is_err());
    }

    #[test]
    fn anthropic_request_body_uses_messages_and_tool_results() {
        let body = anthropic_request_body(&request());
        assert_eq!(body["model"], "gpt-4.1-mini");
        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["type"], "text");
        assert_eq!(body["messages"][1]["role"], "assistant");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["name"], "music.play");
        assert_eq!(body["messages"][2]["role"], "user");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call_1");
        assert_eq!(body["tools"][0]["name"], "music.play");
        assert!(body["tools"][0]["input_schema"].is_object());
    }

    #[test]
    fn base_url_resolution_rules() {
        assert_eq!(
            openai_base_url(LlmProviderKind::OpenAi, None).expect("default"),
            DEFAULT_OPENAI_BASE_URL
        );
        assert_eq!(
            openai_base_url(LlmProviderKind::DeepSeek, None).expect("default"),
            DEFAULT_DEEPSEEK_BASE_URL
        );
        assert_eq!(
            openai_base_url(
                LlmProviderKind::Custom,
                Some("https://llm.example.test/v1/")
            )
            .expect("explicit base"),
            "https://llm.example.test/v1"
        );
        assert!(openai_base_url(LlmProviderKind::Custom, None).is_err());
        assert_eq!(
            openai_base_url(LlmProviderKind::OpencodeGo, None).expect("opencode default"),
            DEFAULT_OPENCODE_GO_BASE_URL
        );
        assert_eq!(
            openai_base_url(
                LlmProviderKind::OpencodeGo,
                Some("https://opencode.ai/zen/go")
            )
            .expect("opencode v1 suffix"),
            "https://opencode.ai/zen/go/v1"
        );
        assert_eq!(
            openai_base_url(
                LlmProviderKind::OpencodeGo,
                Some("https://opencode.ai/zen/go/v1/chat/completions")
            )
            .expect("full endpoint trimmed"),
            "https://opencode.ai/zen/go/v1"
        );
        assert_eq!(
            normalize_model(
                LlmProviderKind::OpencodeGo,
                "opencode-go/deepseek-v4.1-flash"
            ),
            "deepseek-v4.1-flash"
        );
        assert_eq!(
            normalize_model(LlmProviderKind::OpencodeGo, "deepseek-v4.1-flash"),
            "deepseek-v4.1-flash"
        );
        assert_eq!(
            normalize_model(LlmProviderKind::OpenAi, "opencode-go/model"),
            "opencode-go/model"
        );
        assert!(openai_base_url(LlmProviderKind::Anthropic, None).is_err());
        assert!(openai_base_url(LlmProviderKind::Custom, Some("not-a-url")).is_err());
        assert_eq!(
            anthropic_base_url(None).expect("default"),
            DEFAULT_ANTHROPIC_BASE_URL
        );
        assert_eq!(
            openai_chat_completions_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com/"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn provider_secret_names_are_vault_compatible() {
        let id = Uuid::new_v4();
        let name = provider_secret_name(id);
        assert!(name.starts_with("llm.provider."));
        assert!(name.ends_with(".api_key"));
        assert!(hs_core::validation::secret_name(&name).is_ok());
    }

    #[tokio::test]
    async fn factory_requires_a_base_url_for_custom_providers() {
        let factory = DefaultFactory::default();
        let custom = provider(LlmProviderKind::Custom, None);
        let error = match factory.create(&custom, "sk-test").await {
            Ok(_) => panic!("custom providers need a base URL"),
            Err(error) => error,
        };
        assert!(matches!(error, AgentError::Provider(_)), "{error:?}");

        let opencode = provider(LlmProviderKind::OpencodeGo, None);
        assert!(
            factory.create(&opencode, "sk-test").await.is_ok(),
            "opencode-go now has a documented default base URL"
        );
    }

    #[tokio::test]
    async fn factory_builds_clients_for_known_providers() {
        let factory = DefaultFactory::default();
        for kind in [
            LlmProviderKind::OpenAi,
            LlmProviderKind::DeepSeek,
            LlmProviderKind::Anthropic,
        ] {
            let record = provider(kind, None);
            let _client = factory
                .create(&record, "sk-test")
                .await
                .unwrap_or_else(|error| panic!("{kind:?} should build a client: {error}"));
        }
    }
}
