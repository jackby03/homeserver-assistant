//! Encryption at rest for API keys, tokens and biometric data.
//!
//! A random 256-bit master key is generated on first boot and stored with
//! `0600` permissions in `<data_dir>/master.key`. Secrets are encrypted with
//! ChaCha20-Poly1305 (AEAD) using a fresh 96-bit nonce per value and stored as
//! base64 in the database. Plaintext never touches disk.

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::error::{CoreError, CoreResult};
use crate::state::db::{Database, Table};
use crate::state::fsutil;

/// Length of the master key in bytes.
pub const MASTER_KEY_LEN: usize = 32;
/// Current secret envelope version.
pub const SECRET_ENVELOPE_VERSION: u8 = 1;
/// Default master key file name inside the data directory.
pub const MASTER_KEY_FILE: &str = "master.key";

/// Encrypted value envelope stored in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSecret {
    /// Envelope version (for future key rotation).
    pub version: u8,
    /// Base64 encoded 96-bit nonce.
    pub nonce: String,
    /// Base64 encoded ciphertext (includes the Poly1305 tag).
    pub ciphertext: String,
}

/// Master-key holder. Cloning shares the same key material.
#[derive(Clone)]
pub struct SecretVault {
    key: [u8; MASTER_KEY_LEN],
}

impl Zeroize for SecretVault {
    fn zeroize(&mut self) {
        self.key.zeroize();
    }
}

impl std::fmt::Debug for SecretVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretVault")
            .field("key", &"[redacted]")
            .finish()
    }
}

impl SecretVault {
    /// Create a vault with a cryptographically random key.
    #[must_use]
    pub fn generate() -> Self {
        let mut key = [0u8; MASTER_KEY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self { key }
    }

    /// Load the master key from `path`, creating it when missing.
    ///
    /// # Errors
    /// Returns [`CoreError::Crypto`] when the file exists but is malformed, and
    /// [`CoreError::Io`] when it cannot be read or written.
    pub fn load_or_create(path: &Path) -> CoreResult<Self> {
        if path.exists() {
            let bytes = std::fs::read(path)?;
            return Self::from_bytes(&bytes);
        }
        let vault = Self::generate();
        fsutil::write_private(path, &vault.key)?;
        tracing::info!(
            path = %path.display(),
            "generated new master key for the secret vault"
        );
        Ok(vault)
    }

    /// Build a vault from raw key bytes (32 bytes) or a 64-character hex key.
    ///
    /// # Errors
    /// Returns [`CoreError::Crypto`] when the input is not a valid key.
    pub fn from_bytes(bytes: &[u8]) -> CoreResult<Self> {
        if bytes.len() == MASTER_KEY_LEN {
            let mut key = [0u8; MASTER_KEY_LEN];
            key.copy_from_slice(bytes);
            return Ok(Self { key });
        }
        if bytes.len() == MASTER_KEY_LEN * 2 {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| CoreError::Crypto("master key is not valid UTF-8".to_owned()))?;
            return Self::from_hex(text.trim());
        }
        Err(CoreError::Crypto(format!(
            "master key must be {MASTER_KEY_LEN} raw bytes or {} hex characters",
            MASTER_KEY_LEN * 2
        )))
    }

    /// Build a vault from a hex-encoded key.
    ///
    /// # Errors
    /// Returns [`CoreError::Crypto`] when the key is malformed.
    pub fn from_hex(hex_key: &str) -> CoreResult<Self> {
        let bytes = hex::decode(hex_key)
            .map_err(|_| CoreError::Crypto("master key is not valid hex".to_owned()))?;
        if bytes.len() != MASTER_KEY_LEN {
            return Err(CoreError::Crypto(format!(
                "master key must be {MASTER_KEY_LEN} bytes"
            )));
        }
        let mut key = [0u8; MASTER_KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(Self { key })
    }

    /// Encrypt a UTF-8 secret.
    ///
    /// # Errors
    /// Returns [`CoreError::Crypto`] when encryption fails.
    pub fn encrypt(&self, plaintext: &[u8]) -> CoreResult<EncryptedSecret> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));

        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| CoreError::Crypto("failed to encrypt secret".to_owned()))?;

        Ok(EncryptedSecret {
            version: SECRET_ENVELOPE_VERSION,
            nonce: base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
            ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
        })
    }

    /// Decrypt a secret envelope.
    ///
    /// # Errors
    /// Returns [`CoreError::Crypto`] when the envelope is malformed or
    /// authentication fails (tampering or wrong key).
    pub fn decrypt(&self, secret: &EncryptedSecret) -> CoreResult<Zeroizing<Vec<u8>>> {
        if secret.version != SECRET_ENVELOPE_VERSION {
            return Err(CoreError::Crypto(format!(
                "unsupported secret envelope version {}",
                secret.version
            )));
        }
        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(secret.nonce.as_bytes())
            .map_err(|_| CoreError::Crypto("secret nonce is not valid base64".to_owned()))?;
        if nonce_bytes.len() != 12 {
            return Err(CoreError::Crypto(
                "secret nonce must be 12 bytes".to_owned(),
            ));
        }
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(secret.ciphertext.as_bytes())
            .map_err(|_| CoreError::Crypto("secret ciphertext is not valid base64".to_owned()))?;

        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| CoreError::Crypto("failed to decrypt secret".to_owned()))?;
        Ok(Zeroizing::new(plaintext))
    }
}

/// Metadata stored next to an encrypted secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRecord {
    /// Secret name (`llm.openai.api_key`).
    pub name: String,
    /// Encrypted payload.
    pub encrypted: EncryptedSecret,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// High-level secret storage backed by [`Database`] and [`SecretVault`].
#[derive(Debug)]
pub struct SecretStore {
    db: Arc<Database>,
    vault: SecretVault,
}

impl SecretStore {
    /// Create a secret store.
    #[must_use]
    pub fn new(db: Arc<Database>, vault: SecretVault) -> Self {
        Self { db, vault }
    }

    /// Create or update a secret.
    ///
    /// # Errors
    /// Returns an error when the name is invalid or encryption/storage fails.
    pub fn set(&self, name: &str, plaintext: &str) -> CoreResult<()> {
        crate::validation::secret_name(name)?;
        let now = Utc::now();
        let created_at = self
            .db
            .get::<SecretRecord>(Table::Secrets, name)?
            .map_or(now, |record| record.created_at);
        let record = SecretRecord {
            name: name.to_owned(),
            encrypted: self.vault.encrypt(plaintext.as_bytes())?,
            created_at,
            updated_at: now,
        };
        self.db.put(Table::Secrets, name, &record)
    }

    /// Load and decrypt a secret.
    ///
    /// # Errors
    /// Returns an error when decryption or storage fails.
    pub fn get(&self, name: &str) -> CoreResult<Option<Zeroizing<String>>> {
        let Some(record) = self.db.get::<SecretRecord>(Table::Secrets, name)? else {
            return Ok(None);
        };
        let plaintext = self.vault.decrypt(&record.encrypted)?;
        let text = String::from_utf8(plaintext.to_vec())
            .map_err(|_| CoreError::Crypto("secret is not valid UTF-8".to_owned()))?;
        Ok(Some(Zeroizing::new(text)))
    }

    /// Remove a secret.
    ///
    /// # Errors
    /// Returns an error when storage fails.
    pub fn delete(&self, name: &str) -> CoreResult<bool> {
        self.db.delete(Table::Secrets, name)
    }

    /// Whether a secret exists.
    ///
    /// # Errors
    /// Returns an error when storage fails.
    pub fn contains(&self, name: &str) -> CoreResult<bool> {
        Ok(self.db.get::<SecretRecord>(Table::Secrets, name)?.is_some())
    }

    /// List secret names (never values).
    ///
    /// # Errors
    /// Returns an error when storage fails.
    pub fn list_names(&self) -> CoreResult<Vec<String>> {
        Ok(self
            .db
            .list::<SecretRecord>(Table::Secrets)?
            .into_iter()
            .map(|record| record.name)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let vault = SecretVault::generate();
        let encrypted = vault.encrypt(b"sk-super-secret").unwrap();
        assert_ne!(encrypted.ciphertext, "sk-super-secret");
        let decrypted = vault.decrypt(&encrypted).unwrap();
        assert_eq!(std::str::from_utf8(&decrypted).unwrap(), "sk-super-secret");
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let vault = SecretVault::generate();
        let other = SecretVault::generate();
        let encrypted = vault.encrypt(b"payload").unwrap();
        assert!(other.decrypt(&encrypted).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let vault = SecretVault::generate();
        let mut encrypted = vault.encrypt(b"payload").unwrap();
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(encrypted.ciphertext.as_bytes())
            .unwrap();
        bytes[0] ^= 0xff;
        encrypted.ciphertext = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert!(vault.decrypt(&encrypted).is_err());
    }

    #[test]
    fn nonces_are_unique() {
        let vault = SecretVault::generate();
        let a = vault.encrypt(b"same").unwrap();
        let b = vault.encrypt(b"same").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn master_key_file_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MASTER_KEY_FILE);
        let vault = SecretVault::load_or_create(&path).unwrap();
        let encrypted = vault.encrypt(b"value").unwrap();
        assert_eq!(
            std::str::from_utf8(&vault.decrypt(&encrypted).unwrap()).unwrap(),
            "value"
        );

        // Reloading uses the same key.
        let reloaded = SecretVault::load_or_create(&path).unwrap();
        assert_eq!(
            std::str::from_utf8(&reloaded.decrypt(&encrypted).unwrap()).unwrap(),
            "value"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn secret_store_roundtrip() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SecretStore::new(db, SecretVault::generate());
        store.set("llm.openai.api_key", "sk-123").unwrap();
        assert!(store.contains("llm.openai.api_key").unwrap());
        assert_eq!(
            store.get("llm.openai.api_key").unwrap().unwrap().as_str(),
            "sk-123"
        );
        assert_eq!(store.list_names().unwrap(), vec!["llm.openai.api_key"]);
        assert!(store.delete("llm.openai.api_key").unwrap());
        assert!(store.get("llm.openai.api_key").unwrap().is_none());
    }

    #[test]
    fn rejects_invalid_secret_names() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let store = SecretStore::new(db, SecretVault::generate());
        assert!(store.set("UPPER", "x").is_err());
        assert!(store.set("", "x").is_err());
    }
}
