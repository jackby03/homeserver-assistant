//! Home Assistant REST client and instance provisioning.
//!
//! The crate provides three layers:
//!
//! - [`transport`]: the HTTP port ([`transport::HaTransport`]) plus the
//!   production [`transport::ReqwestTransport`]. The port exists so the client
//!   can be unit tested without a network or a real Home Assistant instance.
//! - [`client`]: the typed REST client ([`client::HaClient`]) for the endpoints
//!   the appliance uses (`/api/`, `/api/states`, `/api/services/{domain}/{service}`,
//!   `/api/config`).
//! - [`service`]: a shared, reconfigurable handle ([`service::HaService`]) used
//!   by the daemon after the onboarding wizard stores the access token.
//! - [`provisioner`]: deploys Home Assistant as an OCI container through the
//!   [`hs_runtime::RuntimeProvider`] port.
//!
//! # Security
//! The long-lived access token is stored by the caller through
//! `hs_core::state::SecretVault` and only passed to
//! [`transport::ReqwestTransport::new`]; it is never logged or serialized.

#![deny(missing_docs)]

pub mod client;
pub mod error;
pub mod model;
pub mod provisioner;
pub mod service;
pub mod transport;

pub use client::HaClient;
pub use error::{HaError, HaResult};
pub use model::{HaEntity, HaStatus};
pub use provisioner::{HaProvisioner, ProvisionReport, ProvisionRequest};
pub use service::HaService;
pub use transport::{HaTransport, ReqwestTransport};
