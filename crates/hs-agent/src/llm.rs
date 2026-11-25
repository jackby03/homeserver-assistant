//! Provider-agnostic LLM abstraction used by the agent loop.
//!
//! Adapters (OpenAI-compatible and Anthropic) implement [`LlmClient`]; the
//! daemon injects a [`LlmClientFactory`] so the agent loop can be tested with
//! scripted clients and never performs network I/O in unit tests.

use std::sync::Arc;

use async_trait::async_trait;
use hs_core::models::{ChatMessage, ProviderRecord, TokenUsage, ToolCall};
use serde::{Deserialize, Serialize};

use crate::error::AgentResult;

/// Tool specification sent to the language model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmToolSpec {
    /// Registered tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON-schema-style parameter object.
    pub parameters: serde_json::Value,
}

/// One completion request sent to a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    /// Model identifier.
    pub model: String,
    /// System prompt.
    pub system: String,
    /// Conversation messages, oldest first.
    pub messages: Vec<ChatMessage>,
    /// Tools the model may call.
    pub tools: Vec<LlmToolSpec>,
    /// Sampling temperature (`0.0..=2.0`).
    pub temperature: f32,
    /// Maximum number of tokens to generate.
    pub max_tokens: u32,
    /// Stable per-conversation id for gateways that require session affinity
    /// (OpenCode Zen Go sends it as `x-opencode-session`). `None` for providers
    /// that do not use it.
    pub session_id: Option<String>,
}

/// One completion response returned by a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmResponse {
    /// Text content, if the model produced any.
    pub content: Option<String>,
    /// Tool calls requested by the model.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage reported by the provider.
    pub usage: TokenUsage,
    /// Model that produced the response.
    pub model: String,
}

/// Provider-specific completion client.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// Perform one completion request.
    ///
    /// # Errors
    /// Returns [`crate::AgentError::Provider`] or
    /// [`crate::AgentError::Unauthorized`] when the provider call fails.
    async fn complete(&self, request: LlmRequest) -> AgentResult<LlmResponse>;
}

/// Factory that turns a stored provider record into a concrete client.
///
/// The compose root passes [`crate::providers::DefaultFactory`]; tests inject a
/// scripted factory.
#[async_trait]
pub trait LlmClientFactory: Send + Sync {
    /// Build a client for `provider`, authenticating with `api_key`.
    ///
    /// # Errors
    /// Returns [`crate::AgentError::Provider`] when the provider configuration
    /// is incomplete or the transport cannot be created.
    async fn create(
        &self,
        provider: &ProviderRecord,
        api_key: &str,
    ) -> AgentResult<Arc<dyn LlmClient>>;
}
