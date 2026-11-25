//! `hs-auth` — administrator authentication and authorization.
//!
//! - Argon2id password hashing ([`password`]).
//! - HS256 JWT sessions with server-side revocation ([`jwt`], [`service`]).
//! - Role-based access control ([`rbac`]).
//! - Login throttling, CSRF tokens and the audit trail ([`service`]).
//!
//! The HTTP layer (`hs-daemon`) is responsible for cookies and headers; this
//! crate only issues and validates tokens.
//!
//! ```
//! use std::sync::Arc;
//! use hs_auth::{AuthConfig, AuthService, JwtService};
//! use hs_core::state::Database;
//!
//! let db = Arc::new(Database::open_in_memory().unwrap());
//! let jwt = JwtService::new(JwtService::generate_secret(), "homecore-daemon");
//! let auth = AuthService::new(db, jwt, AuthConfig::default());
//! auth.bootstrap_admin("admin", "Sup3rSecret!!", "Admin").unwrap();
//! let login = auth.login("admin", "Sup3rSecret!!", Some("127.0.0.1"), None).unwrap();
//! let context = auth.authenticate(&login.token).unwrap();
//! assert_eq!(context.user.username, "admin");
//! ```

#![deny(missing_docs)]

pub mod error;
pub mod jwt;
pub mod password;
pub mod rbac;
pub mod service;

pub use error::{AuthError, AuthResult};
pub use jwt::{Claims, JwtService};
pub use rbac::{role_can, Permission};
pub use service::{AuthConfig, AuthContext, AuthService, LoginResult};
