//! Domain models shared by the daemon, domain crates, persistence and the UI.
//!
//! All timestamps are UTC. All identifiers are UUID strings unless documented
//! otherwise. Models are persisted as JSON in redb and returned as JSON by the
//! HTTP API; renaming a field is a breaking API change.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::LlmProviderKind;

// ---------------------------------------------------------------------------
// Users, roles and sessions
// ---------------------------------------------------------------------------

/// Administrative role. Permissions are defined in `hs-auth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Full access, including user management and network changes.
    Admin,
    /// Day-to-day operations: instances, apps, profiles, agent.
    Operator,
    /// Read-only access to dashboards and instance state.
    Viewer,
}

impl Role {
    /// Stable identifier used by the API.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }
}

/// Admin portal user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// Unique identifier.
    pub id: Uuid,
    /// Lower-case login name.
    pub username: String,
    /// Display name shown in the portal.
    pub display_name: String,
    /// Argon2id PHC string. Never serialized by the API.
    pub password_hash: String,
    /// Role used for RBAC checks.
    pub role: Role,
    /// Disabled accounts cannot log in.
    pub disabled: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
    /// Last successful login, if any.
    pub last_login_at: Option<DateTime<Utc>>,
}

/// Server-side session record; JWT claims reference `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Session id (also the JWT `sid` claim).
    pub id: Uuid,
    /// Owner of the session.
    pub user_id: Uuid,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Revoked sessions remain stored until purged.
    pub revoked: bool,
    /// Client IP address (best effort).
    pub ip: Option<String>,
    /// Client user agent (best effort).
    pub user_agent: Option<String>,
}

impl SessionRecord {
    /// Whether the session is currently usable.
    #[must_use]
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        !self.revoked && self.expires_at > now
    }
}

// ---------------------------------------------------------------------------
// Profiles and avatars
// ---------------------------------------------------------------------------

/// Avatar silhouette shapes supported by the kiosk renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarShape {
    /// Rounded circle.
    Circle,
    /// iOS-like squircle.
    Squircle,
    /// Soft cloud shape.
    Cloud,
    /// Rounded cone.
    Cone,
    /// Water droplet.
    Droplet,
}

impl AvatarShape {
    /// All shapes, used by the wizard and admin UI.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Circle,
            Self::Squircle,
            Self::Cloud,
            Self::Cone,
            Self::Droplet,
        ]
    }

    /// Stable identifier used by the API.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Squircle => "squircle",
            Self::Cloud => "cloud",
            Self::Cone => "cone",
            Self::Droplet => "droplet",
        }
    }
}

/// Customizable avatar appearance stored per profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Avatar {
    /// Silhouette shape.
    pub shape: AvatarShape,
    /// Primary color, `#RRGGBB`.
    pub color: String,
    /// Eye color, `#RRGGBB`.
    pub eye_color: String,
    /// Eye tilt in degrees (-45..=45).
    pub eye_tilt_deg: f32,
}

impl Default for Avatar {
    fn default() -> Self {
        Self {
            shape: AvatarShape::Squircle,
            color: "#6ee7ff".to_owned(),
            eye_color: "#0b1220".to_owned(),
            eye_tilt_deg: 0.0,
        }
    }
}

impl Avatar {
    /// Validate colors and eye tilt.
    ///
    /// # Errors
    /// Returns [`crate::error::CoreError::Validation`] for invalid values.
    pub fn validate(&self) -> crate::error::CoreResult<()> {
        validate_hex_color(&self.color, "color")?;
        validate_hex_color(&self.eye_color, "eye_color")?;
        if !(-45.0..=45.0).contains(&self.eye_tilt_deg) {
            return Err(crate::error::CoreError::validation(
                "eye_tilt_deg",
                "must be between -45 and 45 degrees",
            ));
        }
        Ok(())
    }
}

fn validate_hex_color(value: &str, field: &'static str) -> crate::error::CoreResult<()> {
    let valid = value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(())
    } else {
        Err(crate::error::CoreError::validation(
            field,
            "must use #RRGGBB notation",
        ))
    }
}

/// Family role of a profile. Controls the default tool set and UI affordances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileRole {
    /// Household administrator profile (full tool set).
    Owner,
    /// Adult family member.
    Adult,
    /// Child profile (restricted tools).
    Child,
    /// Guest profile (minimal tools).
    Guest,
}

impl ProfileRole {
    /// Stable identifier used by the API.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Adult => "adult",
            Self::Child => "child",
            Self::Guest => "guest",
        }
    }
}

/// Conversation tone applied to the system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// Warm and helpful (default).
    Friendly,
    /// Short and neutral.
    Calm,
    /// Playful, suitable for children.
    Playful,
    /// Professional and concise.
    Formal,
}

/// Household profile: who is in front of the kiosk and what the agent may do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Unique identifier.
    pub id: Uuid,
    /// Display name, e.g. "Mamá".
    pub name: String,
    /// Family role.
    pub role: ProfileRole,
    /// Avatar customization.
    pub avatar: Avatar,
    /// Preferred language (`es` or `en`).
    pub language: String,
    /// Conversation tone.
    pub tone: Tone,
    /// Tool names this profile is allowed to use. Empty means no tools.
    pub allowed_tools: Vec<String>,
    /// Whether a local face embedding is enrolled.
    pub face_enrolled: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Profile {
    /// Whether the profile is allowed to invoke a tool.
    ///
    /// Matching is exact and case-sensitive on purpose: tool names are
    /// lowercase identifiers controlled by the server.
    #[must_use]
    pub fn may_use_tool(&self, tool: &str) -> bool {
        self.allowed_tools.iter().any(|allowed| allowed == tool)
    }
}

// ---------------------------------------------------------------------------
// Compute runtime
// ---------------------------------------------------------------------------

/// Kind of runtime workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// System container (LXC inside Incus).
    Container,
    /// QEMU/KVM virtual machine.
    Vm,
}

/// Runtime status normalized across providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    /// Workload is running.
    Running,
    /// Workload exists but is stopped.
    Stopped,
    /// Workload is starting.
    Starting,
    /// Workload is stopping.
    Stopping,
    /// Workload is frozen/paused.
    Frozen,
    /// Provider reported an error.
    Error,
    /// Status is unknown (provider unavailable).
    Unknown,
}

impl RuntimeStatus {
    /// Whether the workload is running.
    #[must_use]
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

/// Network segment where a workload lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkSegment {
    /// Trusted home LAN.
    Lan,
    /// Isolated DMZ.
    Dmz,
}

impl NetworkSegment {
    /// Stable identifier used by the API.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Lan => "lan",
            Self::Dmz => "dmz",
        }
    }
}

/// Resource limits for a workload. `None` means "provider default".
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU limit in cores (e.g. 0.5, 2.0).
    pub cpu_limit: Option<f32>,
    /// Memory limit in MiB.
    pub memory_mb: Option<u64>,
}

impl ResourceLimits {
    /// Validate limits.
    ///
    /// # Errors
    /// Returns [`crate::error::CoreError::Validation`] for invalid values.
    pub fn validate(&self) -> crate::error::CoreResult<()> {
        if let Some(cpu) = self.cpu_limit {
            if !cpu.is_finite() || cpu <= 0.0 || cpu > 128.0 {
                return Err(crate::error::CoreError::validation(
                    "cpu_limit",
                    "must be greater than 0 and at most 128",
                ));
            }
        }
        if let Some(memory) = self.memory_mb {
            if !(64..=1024 * 1024).contains(&memory) {
                return Err(crate::error::CoreError::validation(
                    "memory_mb",
                    "must be between 64 and 1048576",
                ));
            }
        }
        Ok(())
    }
}

/// Specification used to create a workload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceSpec {
    /// Instance name, unique in the provider.
    pub name: String,
    /// Container or VM.
    pub kind: RuntimeKind,
    /// Base image alias, e.g. `images:debian/13` or an OCI image.
    pub image: String,
    /// Optional network segment; `None` attaches no managed bridge.
    pub segment: Option<NetworkSegment>,
    /// Resource limits.
    #[serde(default)]
    pub limits: ResourceLimits,
    /// Environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Start the workload immediately after creation.
    #[serde(default = "default_true")]
    pub start: bool,
}

fn default_true() -> bool {
    true
}

impl InstanceSpec {
    /// Validate the specification.
    ///
    /// # Errors
    /// Returns [`crate::error::CoreError::Validation`] for invalid values.
    pub fn validate(&self) -> crate::error::CoreResult<()> {
        crate::validation::instance_name(&self.name)?;
        if self.image.trim().is_empty() {
            return Err(crate::error::CoreError::validation(
                "image",
                "must not be empty",
            ));
        }
        self.limits.validate()
    }
}

/// Summary of a workload returned by list/get operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceSummary {
    /// Instance name.
    pub name: String,
    /// Container or VM.
    pub kind: RuntimeKind,
    /// Normalized status.
    pub status: RuntimeStatus,
    /// Base image when known.
    pub image: String,
    /// IPv4 addresses assigned to the instance.
    pub ipv4_addresses: Vec<String>,
    /// Resource limits when known.
    pub limits: ResourceLimits,
    /// Creation timestamp when known.
    pub created_at: Option<DateTime<Utc>>,
    /// Number of snapshots.
    pub snapshot_count: u32,
}

/// Snapshot metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    /// Snapshot name.
    pub name: String,
    /// Creation timestamp when known.
    pub created_at: Option<DateTime<Utc>>,
    /// Size in bytes when known.
    pub size_bytes: u64,
}

/// Result of a one-shot command execution inside a workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutput {
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Exit code, when available.
    pub exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Applications
// ---------------------------------------------------------------------------

/// Application deployment state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    /// Record created, deployment not started.
    Pending,
    /// Deployment in progress.
    Deploying,
    /// Running and healthy.
    Running,
    /// Deployed but stopped.
    Stopped,
    /// Deployment failed.
    Failed,
    /// Removed from the provider.
    Removed,
}

/// Application catalog category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    /// Automation and IoT.
    Automation,
    /// Media and audio.
    Media,
    /// Networking and DNS.
    Networking,
    /// Productivity and files.
    Productivity,
    /// Storage and backup.
    Storage,
    /// Everything else.
    Other,
}

/// Environment variable exposed by an app template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTemplateEnv {
    /// Environment variable name.
    pub key: String,
    /// Label shown in the UI.
    pub label: String,
    /// Whether the variable must be provided.
    pub required: bool,
    /// Whether the value is a secret (stored encrypted, never returned).
    pub secret: bool,
    /// Optional default value.
    pub default: Option<String>,
}

/// Catalog entry for a deployable application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppTemplate {
    /// Stable template identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// OCI image reference.
    pub image: String,
    /// Port exposed by the container.
    pub default_port: u16,
    /// Category.
    pub category: AppCategory,
    /// Suggested network segment.
    pub segment: NetworkSegment,
    /// Environment variables.
    pub env: Vec<AppTemplateEnv>,
    /// Highlight in the App Store.
    pub featured: bool,
}

/// A deployed application instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRecord {
    /// Unique identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Underlying runtime instance name.
    pub instance_name: String,
    /// Template used, when deployed from the catalog.
    pub template_id: Option<String>,
    /// OCI image reference.
    pub image: String,
    /// Network segment.
    pub segment: NetworkSegment,
    /// Internal container port.
    pub internal_port: u16,
    /// Full public hostname when published through the proxy.
    pub hostname: Option<String>,
    /// Whether the app is published through the outbound tunnel.
    pub expose_externally: bool,
    /// Environment variables (non-secret only; secrets live in the vault).
    pub env: BTreeMap<String, String>,
    /// Deployment state.
    pub status: AppStatus,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Request to deploy an application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAppRequest {
    /// Display name.
    pub name: String,
    /// Template id, when deploying from the catalog.
    pub template_id: Option<String>,
    /// OCI image override (required when no template is used).
    pub image: Option<String>,
    /// Target network segment.
    pub segment: NetworkSegment,
    /// Internal port.
    pub internal_port: u16,
    /// Subdomain label for the reverse proxy (LAN apps only by default).
    pub subdomain: Option<String>,
    /// Publish through the outbound tunnel.
    #[serde(default)]
    pub expose_externally: bool,
    /// Environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Protocol that reported a discovered device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryProtocol {
    /// Multicast DNS / Avahi.
    Mdns,
    /// SSDP / UPnP.
    Ssdp,
    /// ARP table.
    Arp,
}

/// A device found on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    /// Stable id derived from MAC address (or IP when MAC is unknown).
    pub id: String,
    /// IPv4 address.
    pub ip: String,
    /// MAC address in lower-case colon notation.
    pub mac: Option<String>,
    /// Hostname advertised by the device.
    pub hostname: Option<String>,
    /// Vendor resolved through the local OUI table.
    pub vendor: Option<String>,
    /// Protocols that reported the device.
    pub protocols: Vec<DiscoveryProtocol>,
    /// Last time the device was seen.
    pub last_seen: DateTime<Utc>,
    /// Whether the device was adopted into Home Assistant.
    pub adopted: bool,
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

/// Outcome of an audited operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// Operation succeeded.
    Success,
    /// Operation was denied by authentication or RBAC.
    Denied,
    /// Operation failed for another reason.
    Failure,
}

/// Tamper-evident activity log entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Monotonic sequence number.
    pub seq: u64,
    /// Timestamp (UTC).
    pub timestamp: DateTime<Utc>,
    /// Username or `system`/`kiosk`.
    pub actor: String,
    /// Dotted action name, e.g. `instance.start`.
    pub action: String,
    /// Target identifier when applicable.
    pub target: Option<String>,
    /// Result of the operation.
    pub outcome: AuditOutcome,
    /// Client address when known.
    pub ip: Option<String>,
    /// Additional structured details. Never contains secrets.
    pub details: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// LLM providers and conversations
// ---------------------------------------------------------------------------

/// Configured LLM provider, persisted without the API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecord {
    /// Unique identifier.
    pub id: Uuid,
    /// Provider family.
    pub kind: LlmProviderKind,
    /// User-visible label.
    pub label: String,
    /// Model identifier.
    pub model: String,
    /// Base URL override for custom/OpenAI-compatible endpoints.
    pub base_url: Option<String>,
    /// Name of the secret in the vault that stores the API key.
    pub api_key_secret: String,
    /// Whether the provider is enabled.
    pub enabled: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Chat message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// System prompt.
    System,
    /// End-user message.
    User,
    /// Assistant response.
    Assistant,
    /// Tool result.
    Tool,
}

/// A message in an agent conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Role of the message author.
    pub role: ChatRole,
    /// Text content (empty for assistant tool-call-only messages).
    pub content: String,
    /// Tool calls requested by the assistant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Id of the tool call this message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned call id.
    pub id: String,
    /// Registered tool name.
    pub name: String,
    /// JSON arguments.
    pub arguments: serde_json::Value,
}

/// A single step of the agentic trace, exposed to the UI for transparency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolTraceEntry {
    /// Tool name.
    pub tool: String,
    /// Arguments passed to the tool.
    pub arguments: serde_json::Value,
    /// Whether the profile was allowed to call the tool.
    pub allowed: bool,
    /// Tool output (truncated) when executed.
    pub output: Option<String>,
    /// Error message when execution failed.
    pub error: Option<String>,
    /// Execution time in milliseconds.
    pub duration_ms: u64,
}

/// Token usage reported by the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Prompt tokens.
    pub prompt_tokens: u64,
    /// Completion tokens.
    pub completion_tokens: u64,
}

/// Request sent to the assistant from the kiosk or portal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentChatRequest {
    /// Active profile (determines permissions and persona).
    pub profile_id: Uuid,
    /// Conversation history, oldest first. The last message must be from the user.
    pub messages: Vec<ChatMessage>,
    /// Whether the agent may call tools.
    #[serde(default = "default_true")]
    pub use_tools: bool,
}

/// Assistant response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentChatResponse {
    /// Final textual reply.
    pub reply: String,
    /// Model that produced the reply.
    pub model: String,
    /// Tool calls executed during the turn.
    pub tool_trace: Vec<ToolTraceEntry>,
    /// Token usage.
    pub usage: TokenUsage,
}

// ---------------------------------------------------------------------------
// Workflows (node-based automation)
// ---------------------------------------------------------------------------

/// What starts a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowTrigger {
    /// Run on demand from the portal or kiosk.
    Manual,
    /// Run when an inbound webhook is received.
    Webhook {
        /// Secret path segment.
        path: String,
    },
    /// Run when a profile is activated.
    ProfileActivated,
}

/// Type of a workflow node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowNodeKind {
    /// Entry point.
    Trigger {
        /// Trigger definition.
        trigger: WorkflowTrigger,
    },
    /// Invoke an agentic tool.
    Tool {
        /// Registered tool name.
        tool: String,
        /// Static arguments; supports `{{input}}` and `{{previous.output}}`.
        #[serde(default)]
        arguments: serde_json::Value,
    },
    /// Continue only when the previous tool output matches `equals`.
    Condition {
        /// Dotted path into the previous output.
        field: String,
        /// Expected value.
        equals: serde_json::Value,
    },
    /// Pause for a number of seconds.
    Delay {
        /// Delay in seconds (max 3600).
        seconds: u64,
    },
}

/// A node in a workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNode {
    /// Node identifier, unique within the workflow.
    pub id: String,
    /// Node behavior.
    #[serde(flatten)]
    pub kind: WorkflowNodeKind,
}

/// Directed edge between two nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdge {
    /// Source node id.
    pub from: String,
    /// Destination node id.
    pub to: String,
}

/// Node-based automation executed by `hs-tools`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Whether the workflow can be triggered.
    pub enabled: bool,
    /// Nodes.
    pub nodes: Vec<WorkflowNode>,
    /// Edges.
    pub edges: Vec<WorkflowEdge>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last modification timestamp.
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_validation() {
        let avatar = Avatar::default();
        avatar.validate().unwrap();
        let bad = Avatar {
            color: "blue".to_owned(),
            ..Avatar::default()
        };
        assert!(bad.validate().is_err());
        let bad_tilt = Avatar {
            eye_tilt_deg: 90.0,
            ..Avatar::default()
        };
        assert!(bad_tilt.validate().is_err());
    }

    #[test]
    fn profile_permission_matching_is_exact() {
        let mut profile = Profile {
            id: Uuid::new_v4(),
            name: "Mamá".to_owned(),
            role: ProfileRole::Adult,
            avatar: Avatar::default(),
            language: "es".to_owned(),
            tone: Tone::Friendly,
            allowed_tools: vec!["music.play".to_owned()],
            face_enrolled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(profile.may_use_tool("music.play"));
        assert!(!profile.may_use_tool("music.play "));
        assert!(!profile.may_use_tool("runtime.delete"));
        profile.allowed_tools.clear();
        assert!(!profile.may_use_tool("music.play"));
    }

    #[test]
    fn session_expiry() {
        let now = Utc::now();
        let mut session = SessionRecord {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            revoked: false,
            ip: None,
            user_agent: None,
        };
        assert!(session.is_active(now));
        session.revoked = true;
        assert!(!session.is_active(now));
        session.revoked = false;
        session.expires_at = now - chrono::Duration::seconds(1);
        assert!(!session.is_active(now));
    }

    #[test]
    fn instance_spec_validation() {
        let mut spec = InstanceSpec {
            name: "home-assistant".to_owned(),
            kind: RuntimeKind::Container,
            image: "images:debian/13".to_owned(),
            segment: Some(NetworkSegment::Lan),
            limits: ResourceLimits {
                cpu_limit: Some(1.0),
                memory_mb: Some(1024),
            },
            env: BTreeMap::new(),
            start: true,
        };
        spec.validate().unwrap();
        spec.name = "Bad Name".to_owned();
        assert!(spec.validate().is_err());
        spec.name = "ok".to_owned();
        spec.limits.memory_mb = Some(8);
        assert!(spec.validate().is_err());
    }
}
