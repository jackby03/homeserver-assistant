//! Argon2id password hashing.
//!
//! Parameters follow the OWASP recommendation (Argon2id, 19 MiB, 2 iterations,
//! 1 lane). Hashes are PHC strings stored verbatim in the database, which makes
//! future parameter upgrades possible without a schema change.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::rngs::OsRng;

use crate::error::{AuthError, AuthResult};

/// Memory cost in KiB (19 MiB).
pub const ARGON2_MEMORY_KIB: u32 = 19_456;
/// Iterations.
pub const ARGON2_ITERATIONS: u32 = 2;
/// Parallel lanes.
pub const ARGON2_LANES: u32 = 1;

fn hasher() -> AuthResult<Argon2<'static>> {
    let params = Params::new(ARGON2_MEMORY_KIB, ARGON2_ITERATIONS, ARGON2_LANES, None)
        .map_err(|err| AuthError::Hashing(err.to_string()))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hash a password with a fresh random salt.
///
/// The plaintext is not zeroized here because the caller owns the `&str`
/// (normally deserialized from a request body and dropped right after).
///
/// # Errors
/// Returns [`AuthError::Hashing`] when the Argon2 parameters are invalid or
/// the salt generator fails.
pub fn hash_password(password: &str) -> AuthResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    hasher()?
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AuthError::Hashing(err.to_string()))
}

/// Verify a password against a PHC hash.
///
/// Returns `Ok(false)` for a mismatch and [`AuthError::Hashing`] only when the
/// stored hash itself is malformed.
///
/// # Errors
/// Returns [`AuthError::Hashing`] when the stored hash cannot be parsed.
pub fn verify_password(password: &str, phc_hash: &str) -> AuthResult<bool> {
    let parsed = PasswordHash::new(phc_hash).map_err(|err| AuthError::Hashing(err.to_string()))?;
    Ok(hasher()?
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_argon2id_and_verifies() {
        let hash = hash_password("Sup3rSecret!!").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("Sup3rSecret!!", &hash).unwrap());
        assert!(!verify_password("wrong password", &hash).unwrap());
    }

    #[test]
    fn salts_are_random() {
        let a = hash_password("same-password").unwrap();
        let b = hash_password("same-password").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn malformed_hash_is_an_error() {
        assert!(verify_password("x", "not-a-phc-string").is_err());
    }

    #[test]
    fn unicode_passwords_are_supported() {
        let hash = hash_password("contraseña-ñandú-123").unwrap();
        assert!(verify_password("contraseña-ñandú-123", &hash).unwrap());
    }
}
