//! Symmetric encryption utilities for webhook secret storage.
//!
//! Uses AES-256-GCM authenticated encryption. Encrypted values are stored
//! with an `ENC:v1:` prefix followed by base64-encoded nonce+ciphertext.
//! Values without the prefix are treated as plaintext (backward compatible).

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine as _;
use rand::RngCore;

const ENCRYPTED_PREFIX: &str = "ENC:v1:";
const NONCE_SIZE: usize = 12; // 96-bit nonce for AES-GCM

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Invalid master key: expected 64 hex chars (32 bytes), got {0} chars")]
    InvalidMasterKey(usize),
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("Invalid encrypted data format")]
    InvalidFormat,
}

/// Holds the parsed AES-256-GCM key for encrypting/decrypting webhook secrets.
#[derive(Clone)]
pub struct SecretEncryptor {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for SecretEncryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretEncryptor")
            .field("cipher", &"<redacted>")
            .finish()
    }
}

impl SecretEncryptor {
    /// Create from hex-encoded 32-byte key (64 hex characters).
    pub fn from_hex_key(hex_key: &str) -> Result<Self, CryptoError> {
        let trimmed = hex_key.trim();
        let key_bytes =
            hex::decode(trimmed).map_err(|_| CryptoError::InvalidMasterKey(trimmed.len()))?;
        if key_bytes.len() != 32 {
            return Err(CryptoError::InvalidMasterKey(trimmed.len()));
        }
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;
        Ok(Self { cipher })
    }

    /// Encrypt a plaintext secret. Returns `ENC:v1:<base64(nonce + ciphertext)>`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, CryptoError> {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))?;

        let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        let encoded = base64::engine::general_purpose::STANDARD.encode(&combined);
        Ok(format!("{ENCRYPTED_PREFIX}{encoded}"))
    }

    /// Decrypt a stored secret. If not prefixed with `ENC:v1:`, returns as-is
    /// (plaintext passthrough for backward compatibility / migration).
    pub fn decrypt(&self, stored: &str) -> Result<String, CryptoError> {
        let Some(encoded) = stored.strip_prefix(ENCRYPTED_PREFIX) else {
            return Ok(stored.to_string()); // plaintext passthrough
        };

        let combined = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| CryptoError::InvalidFormat)?;

        if combined.len() < NONCE_SIZE {
            return Err(CryptoError::InvalidFormat);
        }

        let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))?;

        String::from_utf8(plaintext).map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
    }

    /// Returns true if the value has the encrypted prefix.
    pub fn is_encrypted(value: &str) -> bool {
        value.starts_with(ENCRYPTED_PREFIX)
    }
}

/// Load a [`SecretEncryptor`] from the `ABATHUR_MASTER_KEY` environment variable.
///
/// Returns `None` if the variable is unset, empty, or invalid (with a log warning/error).
pub fn load_encryptor_from_env() -> Option<SecretEncryptor> {
    match std::env::var("ABATHUR_MASTER_KEY") {
        Ok(key) if !key.is_empty() => match SecretEncryptor::from_hex_key(&key) {
            Ok(enc) => {
                tracing::info!("Webhook secret encryption enabled (ABATHUR_MASTER_KEY set)");
                Some(enc)
            }
            Err(e) => {
                tracing::error!(
                    "Invalid ABATHUR_MASTER_KEY: {e} — webhook secrets will be stored in plaintext"
                );
                None
            }
        },
        _ => {
            tracing::debug!("ABATHUR_MASTER_KEY not set — webhook secrets stored in plaintext");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 32-byte key as 64 hex characters
    const TEST_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn test_encryptor() -> SecretEncryptor {
        SecretEncryptor::from_hex_key(TEST_KEY).unwrap()
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let enc = test_encryptor();
        let original = "my-webhook-secret-123!@#";
        let encrypted = enc.encrypt(original).unwrap();
        let decrypted = enc.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_decrypt_plaintext_passthrough() {
        let enc = test_encryptor();
        let plaintext = "not-encrypted-at-all";
        let result = enc.decrypt(plaintext).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn test_encrypt_produces_prefixed_output() {
        let enc = test_encryptor();
        let encrypted = enc.encrypt("secret").unwrap();
        assert!(encrypted.starts_with("ENC:v1:"));
    }

    #[test]
    fn test_different_encryptions_produce_different_output() {
        let enc = test_encryptor();
        let a = enc.encrypt("same-input").unwrap();
        let b = enc.encrypt("same-input").unwrap();
        // Different nonces should produce different ciphertexts
        assert_ne!(a, b);
        // But both decrypt to the same value
        assert_eq!(enc.decrypt(&a).unwrap(), "same-input");
        assert_eq!(enc.decrypt(&b).unwrap(), "same-input");
    }

    #[test]
    fn test_invalid_master_key_wrong_length() {
        let result = SecretEncryptor::from_hex_key("0123456789abcdef"); // 16 chars = 8 bytes
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("16 chars"), "error was: {err}");
    }

    #[test]
    fn test_invalid_master_key_not_hex() {
        let result = SecretEncryptor::from_hex_key(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let enc = test_encryptor();
        let encrypted = enc.encrypt("secret").unwrap();
        // Tamper with the base64 payload
        let prefix = "ENC:v1:";
        let payload = &encrypted[prefix.len()..];
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        if let Some(last) = bytes.last_mut() {
            *last ^= 0xFF; // flip bits
        }
        let tampered = format!(
            "{prefix}{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        );
        assert!(enc.decrypt(&tampered).is_err());
    }

    #[test]
    fn test_is_encrypted() {
        assert!(SecretEncryptor::is_encrypted("ENC:v1:somedata"));
        assert!(!SecretEncryptor::is_encrypted("plaintext-secret"));
        assert!(!SecretEncryptor::is_encrypted(""));
    }
}
