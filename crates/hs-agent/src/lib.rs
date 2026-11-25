//! LLM provider adapters and the agent tool-calling loop.
//!
//! # Layers
//! - [`llm`]: provider-agnostic request/response types plus the [`LlmClient`]
//!   and [`LlmClientFactory`] traits that the daemon injects.
//! - [`providers`]: OpenAI-compatible (`openai`, `deepseek`, `opencode_go`,
//!   `custom`) and Anthropic adapters with deterministic request builders and
//!   response parsers.
//! - [`prompt`]: profile-aware system prompt construction.
//! - [`service`]: [`AgentService`] provider CRUD and the agent loop.
//!
//! # Security
//! API keys are only persisted through `hs_core::state::SecretStore` under
//! `llm.provider.<provider-id>.api_key`. Provider records, logs and traces never
//! contain the key itself, and every tool call inside a turn is re-checked
//! against the active profile permissions by `hs-tools`.
//!
//! # Example
//! ```
//! use hs_agent::build_system_prompt;
//! use hs_core::models::{Avatar, Profile, ProfileRole, Tone};
//! use uuid::Uuid;
//!
//! let profile = Profile {
//!     id: Uuid::new_v4(),
//!     name: "Mamá".to_owned(),
//!     role: ProfileRole::Adult,
//!     avatar: Avatar::default(),
//!     language: "es".to_owned(),
//!     tone: Tone::Friendly,
//!     allowed_tools: Vec::new(),
//!     face_enrolled: false,
//!     created_at: chrono::Utc::now(),
//!     updated_at: chrono::Utc::now(),
//! };
//! assert!(build_system_prompt(&profile).contains("Mamá"));
//! ```

#![deny(missing_docs)]

pub mod error;
pub mod llm;
pub mod prompt;
pub mod providers;
pub mod service;
pub mod soul;

pub use error::{AgentError, AgentResult};
pub use llm::{LlmClient, LlmClientFactory, LlmRequest, LlmResponse, LlmToolSpec};
pub use prompt::{build_system_prompt, build_system_prompt_with_soul};
pub use providers::{normalize_model, DefaultFactory, DEFAULT_OPENCODE_GO_BASE_URL};
pub use service::{AgentService, ProviderTest, UpsertProviderRequest};
pub use soul::DEFAULT_SOUL;
