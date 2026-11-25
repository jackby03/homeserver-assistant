//! Minimal, dependency-free HS256 JWT implementation.
//!
//! Only HS256 is supported: the token is signed with the daemon's random
//! per-install secret, so asymmetric algorithms would add complexity without a
//! security benefit. The implementation validates the algorithm header
//! explicitly to prevent `alg: none` / algorithm-confusion attacks.

use base64::Engine as _;
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::{AuthError, AuthResult};

type HmacSha256 = Hmac<Sha256>;

const TOKEN_TTL_LEEWAY_SECONDS: i64 = 30;

/// Claims embedded in the session JWT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// Subject: user id (UUID string).
    pub sub: String,
    /// Session id (UUID string) used for server-side revocation.
    pub sid: String,
    /// Role snapshot; authorization always re-checks the live user record.
    pub role: String,
    /// Issued-at (Unix seconds).
    pub iat: i64,
    /// Expiry (Unix seconds).
    pub exp: i64,
    /// Issuer (`homecore-daemon`).
    pub iss: String,
}

impl Claims {
    /// Whether the token is expired at `now`.
    #[must_use]
    pub fn is_expired(&self, now: i64) -> bool {
        self.exp + TOKEN_TTL_LEEWAY_SECONDS <= now
    }
}

/// HS256 JWT encoder/decoder.
#[derive(Clone)]
pub struct JwtService {
    secret: [u8; 32],
    issuer: String,
}

impl std::fmt::Debug for JwtService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtService")
            .field("secret", &"[redacted]")
            .field("issuer", &self.issuer)
            .finish()
    }
}

impl JwtService {
    /// Build a service from a 32-byte secret.
    #[must_use]
    pub fn new(secret: [u8; 32], issuer: impl Into<String>) -> Self {
        Self {
            secret,
            issuer: issuer.into(),
        }
    }

    /// Generate a random secret (used on first boot).
    #[must_use]
    pub fn generate_secret() -> [u8; 32] {
        use rand::RngCore;
        let mut secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        secret
    }

    /// Issuer embedded in generated tokens.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Encode claims into a compact JWT.
    ///
    /// # Errors
    /// Returns [`AuthError::Token`] when serialization fails.
    pub fn encode(&self, claims: &Claims) -> AuthResult<String> {
        let header = serde_json::json!({"alg": "HS256", "typ": "JWT"});
        let header = base64_url_encode(
            serde_json::to_vec(&header).map_err(|err| AuthError::Token(err.to_string()))?,
        );
        let payload = base64_url_encode(
            serde_json::to_vec(claims).map_err(|err| AuthError::Token(err.to_string()))?,
        );
        let signing_input = format!("{header}.{payload}");
        let signature = self.sign(signing_input.as_bytes());
        Ok(format!("{signing_input}.{signature}"))
    }

    /// Decode and validate a compact JWT.
    ///
    /// # Errors
    /// Returns [`AuthError::Unauthorized`] for malformed, badly signed,
    /// expired or foreign tokens.
    pub fn decode(&self, token: &str) -> AuthResult<Claims> {
        let mut parts = token.split('.');
        let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(AuthError::Unauthorized("malformed token".to_owned()));
        };

        let header_bytes = base64_url_decode(header_b64)?;
        let header: serde_json::Value = serde_json::from_slice(&header_bytes)
            .map_err(|_| AuthError::Unauthorized("malformed token header".to_owned()))?;
        if header.get("alg").and_then(serde_json::Value::as_str) != Some("HS256") {
            return Err(AuthError::Unauthorized(
                "unsupported token algorithm".to_owned(),
            ));
        }

        let expected_signature = self.sign(format!("{header_b64}.{payload_b64}").as_bytes());
        let provided_signature = signature_b64.to_owned();
        if !constant_time_eq(expected_signature.as_bytes(), provided_signature.as_bytes()) {
            return Err(AuthError::Unauthorized(
                "invalid token signature".to_owned(),
            ));
        }

        let payload_bytes = base64_url_decode(payload_b64)?;
        let claims: Claims = serde_json::from_slice(&payload_bytes)
            .map_err(|_| AuthError::Unauthorized("malformed token payload".to_owned()))?;

        if claims.iss != self.issuer {
            return Err(AuthError::Unauthorized(
                "unexpected token issuer".to_owned(),
            ));
        }
        let now = Utc::now().timestamp();
        if claims.is_expired(now) {
            return Err(AuthError::Unauthorized("token expired".to_owned()));
        }
        if claims.iat > now + TOKEN_TTL_LEEWAY_SECONDS {
            return Err(AuthError::Unauthorized(
                "token issued in the future".to_owned(),
            ));
        }
        Ok(claims)
    }

    /// Convenience helper: create claims for a session.
    #[must_use]
    pub fn claims_for(
        &self,
        user_id: uuid::Uuid,
        session_id: uuid::Uuid,
        role: &str,
        ttl: Duration,
    ) -> Claims {
        let now = Utc::now();
        Claims {
            sub: user_id.to_string(),
            sid: session_id.to_string(),
            role: role.to_owned(),
            iat: now.timestamp(),
            exp: (now + ttl).timestamp(),
            iss: self.issuer.clone(),
        }
    }

    fn sign(&self, input: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key size");
        mac.update(input);
        base64_url_encode(mac.finalize().into_bytes().to_vec())
    }

    /// Derive the CSRF token bound to a session.
    ///
    /// The token is deterministic per session and signed with the same secret,
    /// so the server can verify it without storing it.
    #[must_use]
    pub fn csrf_token(&self, session_id: uuid::Uuid) -> String {
        self.sign(format!("csrf:{session_id}").as_bytes())
    }

    /// Verify a CSRF token for a session in constant time.
    #[must_use]
    pub fn verify_csrf(&self, session_id: uuid::Uuid, token: &str) -> bool {
        constant_time_eq(
            self.csrf_token(session_id).as_bytes(),
            token.trim().as_bytes(),
        )
    }
}

fn base64_url_encode(data: Vec<u8>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn base64_url_decode(data: &str) -> AuthResult<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data.as_bytes())
        .map_err(|_| AuthError::Unauthorized("malformed token encoding".to_owned()))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn service() -> JwtService {
        JwtService::new([7u8; 32], "homecore-daemon")
    }

    fn claims() -> Claims {
        service().claims_for(Uuid::new_v4(), Uuid::new_v4(), "admin", Duration::hours(1))
    }

    #[test]
    fn encodes_and_decodes() {
        let jwt = service();
        let claims = claims();
        let token = jwt.encode(&claims).unwrap();
        assert_eq!(token.split('.').count(), 3);
        assert_eq!(jwt.decode(&token).unwrap(), claims);
    }

    #[test]
    fn rejects_tampered_payload() {
        let jwt = service();
        let claims = claims();
        let token = jwt.encode(&claims).unwrap();
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged_payload = base64_url_encode(br#"{"sub":"evil"}"#.to_vec());
        parts[1] = &forged_payload;
        let forged = parts.join(".");
        assert!(jwt.decode(&forged).is_err());
    }

    #[test]
    fn rejects_alg_none() {
        let jwt = service();
        let header = base64_url_encode(br#"{"alg":"none","typ":"JWT"}"#.to_vec());
        let payload = base64_url_encode(serde_json::to_vec(&claims()).unwrap());
        let token = format!("{header}.{payload}.");
        assert!(jwt.decode(&token).is_err());
    }

    #[test]
    fn rejects_wrong_secret() {
        let token = service().encode(&claims()).unwrap();
        let other = JwtService::new([9u8; 32], "homecore-daemon");
        assert!(other.decode(&token).is_err());
    }

    #[test]
    fn rejects_expired_tokens() {
        let jwt = service();
        let mut claims = claims();
        claims.exp = Utc::now().timestamp() - 120;
        let token = jwt.encode(&claims).unwrap();
        assert!(jwt.decode(&token).is_err());
    }

    #[test]
    fn rejects_foreign_issuer() {
        let jwt = service();
        let mut claims = claims();
        claims.iss = "someone-else".to_owned();
        let token = jwt.encode(&claims).unwrap();
        assert!(jwt.decode(&token).is_err());
    }

    #[test]
    fn rejects_malformed_tokens() {
        let jwt = service();
        for token in ["", "abc", "a.b", "a.b.c.d", "...."] {
            assert!(jwt.decode(token).is_err(), "{token} should be rejected");
        }
    }
}
