//! Family profiles, avatars, permissions and local biometric matching.
//!
//! A [`Profile`] describes who is in front of the kiosk: display name, family
//! role, avatar customization, preferred language and conversational tone. It
//! also carries the allow-list of agentic tools the profile may use; that list
//! is enforced by `hs-tools` *before* any adapter is called.
//!
//! # Biometrics
//!
//! Face embeddings are produced locally by the vision pipeline and never leave
//! the appliance. [`ProfileService::enroll_face`] stores each embedding through
//! `hs_core::state::SecretStore` (ChaCha20-Poly1305, encrypted at rest) under
//! `biometric.profile.<profile-id>`. Matching uses cosine similarity and runs
//! entirely in memory; only the best match above the requested threshold is
//! returned.
//!
//! # Example
//! ```
//! use std::sync::Arc;
//!
//! use hs_core::state::{Database, SecretStore, SecretVault};
//! use hs_profiles::{CreateProfileRequest, ProfileService};
//!
//! let db = Arc::new(Database::open_in_memory().expect("in-memory database"));
//! let secrets = Arc::new(SecretStore::new(
//!     Arc::clone(&db),
//!     SecretVault::generate(),
//! ));
//! let service = ProfileService::new(db, secrets);
//! let request = ProfileService::default_profiles()
//!     .into_iter()
//!     .next()
//!     .expect("built-in profiles are available");
//! let profile = service.create(request).expect("profile should be created");
//! assert!(profile.may_use_tool("system.status"));
//! ```

#![deny(missing_docs)]

pub mod biometric;
mod defaults;
pub mod error;
pub mod service;

pub use error::{ProfileError, ProfileResult};
pub use hs_core::models::{Avatar, AvatarShape, Profile, ProfileRole, Tone};
pub use service::{CreateProfileRequest, ProfileService, UpdateProfileRequest};
