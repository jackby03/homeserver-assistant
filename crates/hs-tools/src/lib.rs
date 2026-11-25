//! Tool registry, permission enforcement and workflow engine.
//!
//! # Security invariants
//!
//! - [`ToolRegistry::invoke`] checks the active profile allow-list **before**
//!   any tool is looked up or any adapter is called. There is no bypass flag.
//! - Tools reach the outside world only through the [`ports`] traits;
//!   `hs-tools` never depends on the concrete runtime, Home Assistant, audio or
//!   network crates. The daemon implements the adapters.
//! - The workflow engine executes tool nodes through the same registry, so a
//!   workflow cannot call a tool that the active profile may not use.
//!
//! # Example
//! ```
//! let registry = hs_tools::ToolRegistry::from_tools(Vec::new());
//! assert!(registry.list().is_empty());
//! ```

#![deny(missing_docs)]

mod args;
pub mod error;
pub mod ports;
pub mod registry;
#[cfg(test)]
mod test_support;
mod tools;
pub mod workflow;

pub use error::{ToolError, ToolResult};
pub use hs_core::models::InstanceSummary;
pub use ports::{
    AudioControl, DomoticsControl, EntityLite, EphemeralMemoryStore, MemoryEntry, MemoryStore,
    RuntimeControl, SystemControl, ToolPorts, WebhookClient,
};
pub use registry::{Tool, ToolDescriptor, ToolOutcome, ToolRegistry};
pub use workflow::{WorkflowEngine, WorkflowRun, WorkflowStep};
