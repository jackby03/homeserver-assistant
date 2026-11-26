//! Tool trait, descriptors and the permission-enforcing registry.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_trait::async_trait;
use hs_core::models::Profile;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ToolError, ToolResult};
use crate::ports::ToolPorts;
use crate::tools;

/// How dangerous a tool is when the model decides to call it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRisk {
    /// Read-only or trivially reversible local action.
    Read,
    /// Creates or changes state that the user can undo.
    Write,
    /// Sends data outside the appliance or cannot be undone.
    Destructive,
}

impl ToolRisk {
    /// Whether the agent must ask the user before invoking the tool.
    #[must_use]
    pub fn requires_confirmation(self) -> bool {
        matches!(self, Self::Write | Self::Destructive)
    }
}

/// A capability the assistant may invoke.
///
/// Implementations only talk to the outside world through [`ToolPorts`]; they
/// must not check profile permissions themselves because the registry already
/// does that before calling [`Tool::execute`].
#[async_trait]
pub trait Tool: Send + Sync {
    /// Stable dotted tool name, e.g. `music.play`.
    fn name(&self) -> &'static str;

    /// One-line description shown to the language model.
    fn description(&self) -> &'static str;

    /// JSON-schema-style parameter object with `type`, `properties` and
    /// `required`.
    fn parameters(&self) -> Value;

    /// Risk classification used by the agent confirmation gate.
    fn risk(&self) -> ToolRisk {
        ToolRisk::Read
    }

    /// Execute the tool with arguments that passed schema validation.
    ///
    /// # Errors
    /// Returns [`ToolError`] when the arguments are invalid or the adapter
    /// fails.
    async fn execute(&self, ports: &ToolPorts, arguments: Value) -> ToolResult<Value>;

    /// Execute with the active profile. Most tools ignore it; profile-aware
    /// tools such as memory override this hook instead of `execute`.
    ///
    /// # Errors
    /// Returns [`ToolError`] under the same conditions as [`Tool::execute`].
    async fn execute_for_profile(
        &self,
        ports: &ToolPorts,
        arguments: Value,
        _profile: &Profile,
    ) -> ToolResult<Value> {
        self.execute(ports, arguments).await
    }
}

/// Serialized description of a tool, used to build LLM tool specifications.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON-schema-style parameter object.
    pub parameters: Value,
    /// Risk classification shown to the admin UI.
    pub risk: ToolRisk,
}

/// Result of one successful tool invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    /// Tool name.
    pub tool: String,
    /// Structured output returned to the caller.
    pub output: Value,
    /// Wall-clock execution time in milliseconds.
    pub duration_ms: u64,
}

/// Registry of tools with per-profile permission enforcement.
///
/// The registry owns the outbound ports and is constructed with them by the
/// composition root: [`ToolRegistry::builtin`]. Tests and future plugins may
/// build a custom registry with [`ToolRegistry::from_tools`] and attach ports
/// later through [`ToolRegistry::set_ports`].
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    ports: RwLock<Option<ToolPorts>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ToolRegistry {
    /// Build the registry with the ten built-in tools and their outbound ports.
    ///
    /// This is the constructor used by `hs-daemon` at the composition root.
    #[must_use]
    pub fn builtin(ports: ToolPorts) -> Arc<Self> {
        let registry = Self::from_tools(tools::builtin_tools());
        registry.set_ports(ports);
        Arc::new(registry)
    }

    /// Build the registry with the built-in tools and ports already attached.
    ///
    /// Alias of [`ToolRegistry::builtin`] kept for call sites that read better
    /// with `with_ports`.
    #[must_use]
    pub fn with_ports(ports: ToolPorts) -> Arc<Self> {
        Self::builtin(ports)
    }

    /// Build a registry from an explicit list of tools without ports.
    ///
    /// If two tools share a name the last one wins; built-in tools never
    /// collide. Call [`ToolRegistry::set_ports`] before invoking a tool.
    #[must_use]
    pub fn from_tools(tools: Vec<Arc<dyn Tool>>) -> Self {
        let tools = tools
            .into_iter()
            .map(|tool| (tool.name().to_owned(), tool))
            .collect();
        Self {
            tools,
            ports: RwLock::new(None),
        }
    }

    /// Attach or replace the outbound adapters used to execute tools.
    pub fn set_ports(&self, ports: ToolPorts) {
        let mut slot = match self.ports.write() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        *slot = Some(ports);
    }

    /// Whether outbound adapters have been attached.
    #[must_use]
    pub fn has_ports(&self) -> bool {
        match self.ports.read() {
            Ok(slot) => slot.is_some(),
            Err(poisoned) => poisoned.into_inner().is_some(),
        }
    }

    /// List every registered tool, sorted by name.
    #[must_use]
    pub fn list(&self) -> Vec<ToolDescriptor> {
        self.tools
            .values()
            .map(|tool| ToolDescriptor {
                name: tool.name().to_owned(),
                description: tool.description().to_owned(),
                parameters: tool.parameters(),
                risk: tool.risk(),
            })
            .collect()
    }

    /// Look up one tool by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|tool| tool.as_ref())
    }

    /// Risk classification for a registered tool.
    #[must_use]
    pub fn risk(&self, tool: &str) -> Option<ToolRisk> {
        self.tools.get(tool).map(|entry| entry.risk())
    }

    /// Invoke `tool` on behalf of `profile`.
    ///
    /// Security: the profile allow-list is checked **first**, before the tool
    /// lookup and before any port is touched. A denied call therefore never
    /// reaches an adapter, and denied attempts are logged without arguments.
    ///
    /// # Errors
    /// - [`ToolError::Forbidden`] when the profile may not use the tool.
    /// - [`ToolError::NotFound`] when the tool is not registered.
    /// - [`ToolError::Execution`] when no ports are attached.
    /// - Any error returned by the tool itself.
    pub async fn invoke(
        &self,
        profile: &Profile,
        tool: &str,
        args: Value,
    ) -> ToolResult<ToolOutcome> {
        if !profile.may_use_tool(tool) {
            tracing::warn!(
                profile_id = %profile.id,
                profile_name = %profile.name,
                tool = %tool,
                "tool call denied by the active profile permissions"
            );
            return Err(ToolError::Forbidden {
                tool: tool.to_owned(),
            });
        }

        let Some(entry) = self.tools.get(tool) else {
            return Err(ToolError::NotFound(tool.to_owned()));
        };
        let ports = self.active_ports()?;

        let started = Instant::now();
        let output = entry.execute_for_profile(&ports, args, profile).await?;
        let elapsed = started.elapsed().as_millis();
        let duration_ms = u64::try_from(elapsed).unwrap_or(u64::MAX);
        Ok(ToolOutcome {
            tool: tool.to_owned(),
            output,
            duration_ms,
        })
    }

    fn active_ports(&self) -> ToolResult<ToolPorts> {
        let slot = match self.ports.read() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        slot.clone()
            .ok_or_else(|| ToolError::Execution("tool ports are not configured".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fake_ports, profile_with_tools, CallCounts};
    use serde_json::json;

    fn registry() -> (Arc<ToolRegistry>, Arc<CallCounts>) {
        let (ports, counts, _webhook_payload) = fake_ports();
        (ToolRegistry::with_ports(ports), counts)
    }

    #[tokio::test]
    async fn builtin_registry_exposes_all_tools() {
        let (ports, _counts, _webhook_payload) = fake_ports();
        let registry = ToolRegistry::builtin(ports);
        let descriptors = registry.list();
        assert_eq!(descriptors.len(), 20);
        assert!(registry.get("system.status").is_some());
        assert!(registry.get("music.play").is_some());
        assert!(registry.get("does.not.exist").is_none());
        let names: Vec<&str> = descriptors.iter().map(|tool| tool.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "descriptors must be sorted by name");
    }

    #[tokio::test]
    async fn denied_invocation_never_touches_a_port() {
        let (registry, counts) = registry();
        let profile = profile_with_tools(&[] as &[&str]);

        let error = registry
            .invoke(&profile, "system.status", json!({}))
            .await
            .expect_err("empty allow-list must deny every tool");
        assert!(
            matches!(&error, ToolError::Forbidden { tool } if tool == "system.status"),
            "unexpected error: {error:?}"
        );
        let [runtime, domotics, audio, system, webhooks] = counts.snapshot();
        assert_eq!(
            [runtime, domotics, audio, system, webhooks],
            [0, 0, 0, 0, 0]
        );

        // The permission check runs before the lookup: even an unknown tool is
        // reported as forbidden, never as found.
        let error = registry
            .invoke(&profile, "ghost.tool", json!({}))
            .await
            .expect_err("unknown tool must still be denied first");
        assert!(matches!(error, ToolError::Forbidden { .. }), "{error:?}");
    }

    #[tokio::test]
    async fn allowed_invocation_reaches_the_adapter_once() {
        let (registry, counts) = registry();
        let profile = profile_with_tools(&["system.status"]);
        let outcome = registry
            .invoke(&profile, "system.status", json!({}))
            .await
            .expect("allowed tool must execute");
        assert_eq!(outcome.tool, "system.status");
        assert_eq!(outcome.output["status"], "ok");
        assert_eq!(counts.system.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(counts.runtime.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unknown_tool_is_not_found_for_a_permitted_profile() {
        let (registry, _counts) = registry();
        let profile = profile_with_tools(&["ghost.tool"]);
        let error = registry
            .invoke(&profile, "ghost.tool", json!({}))
            .await
            .expect_err("missing tool must be reported");
        assert!(matches!(error, ToolError::NotFound(name) if name == "ghost.tool"));
    }

    #[tokio::test]
    async fn invocation_without_ports_is_an_execution_error() {
        let registry = ToolRegistry::from_tools(crate::tools::builtin_tools());
        assert!(!registry.has_ports());
        let profile = profile_with_tools(&["system.status"]);
        let error = registry
            .invoke(&profile, "system.status", json!({}))
            .await
            .expect_err("ports are required");
        assert!(matches!(error, ToolError::Execution(_)), "{error:?}");
    }

    #[tokio::test]
    async fn ports_can_be_attached_after_construction() {
        let registry = ToolRegistry::from_tools(crate::tools::builtin_tools());
        let (ports, counts, _webhook_payload) = fake_ports();
        registry.set_ports(ports);
        assert!(registry.has_ports());
        let profile = profile_with_tools(&["music.play"]);
        let outcome = registry
            .invoke(&profile, "music.play", json!({}))
            .await
            .expect("tool must execute after ports are attached");
        assert_eq!(outcome.output["status"], "playing");
        assert_eq!(counts.audio.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
