//! Persistence: configuration, secrets and the embedded database.
//!
//! The daemon owns one [`ConfigManager`], one [`Database`] and one
//! [`SecretStore`] for its whole lifetime. Tests use
//! [`Database::open_in_memory`] so they never touch the developer's disk.

pub mod config_store;
pub mod db;
pub mod fsutil;
pub mod vault;

pub use config_store::ConfigManager;
pub use db::{Database, Table};
pub use vault::{EncryptedSecret, SecretRecord, SecretStore, SecretVault};
