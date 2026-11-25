//! `hs-core` — domain types shared by every HomeServer Assistant component.
//!
//! This crate is a leaf: it does not depend on any other workspace crate. It
//! owns the configuration schema, domain models, the embedded database, the
//! encrypted secret vault and the event types exchanged with the wall kiosk.
//!
//! # Layers
//! - [`config`]: versioned appliance configuration (JSON on disk).
//! - [`models`]: users, profiles, runtimes, apps, discovery, audit, agent and
//!   workflow domain objects.
//! - [`state`]: redb database, secret vault and configuration manager.
//! - [`events`]: real-time kiosk events.
//! - [`api`]: HTTP error envelope shared with the UI.
//! - [`validation`]: centralized input validation.
//!
//! # Example
//! ```
//! use hs_core::config::SystemConfig;
//! use hs_core::state::Database;
//!
//! let config = SystemConfig::bootstrap();
//! config.validate().expect("bootstrap configuration must be valid");
//! let db = Database::open_in_memory().expect("in-memory database");
//! assert!(db.list_users().expect("users").is_empty());
//! ```

pub mod api;
pub mod config;
pub mod dns;
pub mod error;
pub mod events;
pub mod models;
pub mod state;
pub mod validation;
pub mod voice;

pub use dns::{DnsProvider, DnsProviderSpec, DNS_PROVIDERS};
pub use error::{CoreError, CoreResult};
pub use state::{ConfigManager, Database, SecretStore, SecretVault};
pub use voice::{WakeMatch, WakeWordMatcher};

/// Default data directory used by the packaged appliance.
pub const DEFAULT_DATA_DIR: &str = "/var/lib/homecore";

/// Environment variable that overrides the data directory (development/tests).
pub const DATA_DIR_ENV: &str = "HS_DATA_DIR";
