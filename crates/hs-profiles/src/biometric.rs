//! Local biometric embedding helpers.
//!
//! Face embeddings are fixed-length `f32` vectors produced by the local vision
//! pipeline. This module owns the invariants around them (minimum dimension,
//! finiteness), the encrypted secret naming scheme and the cosine similarity
//! used to recognize a profile.

use uuid::Uuid;

use crate::error::{ProfileError, ProfileResult};

/// Minimum number of dimensions accepted for a face embedding.
///
/// Real embedders produce 128 to 512 dimensions; the floor only rejects
/// accidentally truncated payloads.
pub const MIN_EMBEDDING_LEN: usize = 8;

/// Prefix of the secret name used to store a profile embedding.
pub const SECRET_PREFIX: &str = "biometric.profile.";

/// Secret name that stores the encrypted embedding of `profile_id`.
#[must_use]
pub fn secret_name(profile_id: Uuid) -> String {
    format!("{SECRET_PREFIX}{profile_id}")
}

/// Validate an embedding before it is enrolled or used for matching.
///
/// # Errors
/// Returns [`ProfileError::Validation`] when the embedding has fewer than
/// [`MIN_EMBEDDING_LEN`] dimensions or contains a non-finite value.
pub fn validate_embedding(embedding: &[f32]) -> ProfileResult<()> {
    if embedding.len() < MIN_EMBEDDING_LEN {
        return Err(ProfileError::Validation(format!(
            "embedding must have at least {MIN_EMBEDDING_LEN} dimensions, got {}",
            embedding.len()
        )));
    }
    if embedding.iter().any(|value| !value.is_finite()) {
        return Err(ProfileError::Validation(
            "embedding must contain only finite values".to_owned(),
        ));
    }
    Ok(())
}

/// Encode an embedding as the JSON payload stored in the secret vault.
///
/// # Errors
/// Returns [`ProfileError::Validation`] for invalid embeddings and
/// [`ProfileError::Storage`] when the payload cannot be serialized.
pub fn encode_embedding(embedding: &[f32]) -> ProfileResult<String> {
    validate_embedding(embedding)?;
    serde_json::to_string(embedding)
        .map_err(|error| ProfileError::Storage(format!("failed to encode embedding: {error}")))
}

/// Decode an embedding previously produced by [`encode_embedding`].
///
/// # Errors
/// Returns [`ProfileError::Storage`] for corrupted payloads and
/// [`ProfileError::Validation`] for payloads that are no longer valid.
pub fn decode_embedding(payload: &str) -> ProfileResult<Vec<f32>> {
    let embedding: Vec<f32> = serde_json::from_str(payload).map_err(|error| {
        ProfileError::Storage(format!("stored embedding is corrupted: {error}"))
    })?;
    validate_embedding(&embedding)?;
    Ok(embedding)
}

/// Cosine similarity between two embeddings.
///
/// Returns `None` when the vectors have different dimensions, are empty or one
/// of them is the zero vector (similarity is undefined in that case). The
/// result is clamped to `[-1.0, 1.0]` to absorb floating-point noise.
#[must_use]
pub fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (a, b) in left.iter().zip(right.iter()) {
        let a = f64::from(*a);
        let b = f64::from(*b);
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm <= f64::EPSILON || right_norm <= f64::EPSILON {
        return None;
    }
    let similarity = dot / (left_norm.sqrt() * right_norm.sqrt());
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value is clamped to the f32-safe range [-1.0, 1.0]"
    )]
    Some(similarity.clamp(-1.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_embeddings() {
        let embedding = vec![0.1_f32, -0.5, 0.25, 0.0, 1.0, -1.0, 0.75, 0.125];
        let payload = encode_embedding(&embedding).expect("embedding should encode");
        let decoded = decode_embedding(&payload).expect("embedding should decode");
        assert_eq!(decoded, embedding);
    }

    #[test]
    fn rejects_invalid_embeddings() {
        assert!(validate_embedding(&[0.1; 7]).is_err());
        assert!(validate_embedding(&[f32::NAN; 8]).is_err());
        assert!(validate_embedding(&[f32::INFINITY; 8]).is_err());
        assert!(validate_embedding(&[0.0; 8]).is_ok());
    }

    #[test]
    fn cosine_similarity_bounds_and_dimensions() {
        let unit_x = [1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let unit_y = [0.0_f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let minus_x = [-1.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

        let identical = cosine_similarity(&unit_x, &unit_x).expect("same vector");
        assert!((identical - 1.0).abs() < 1e-6);
        let orthogonal = cosine_similarity(&unit_x, &unit_y).expect("valid vectors");
        assert!(orthogonal.abs() < 1e-6);
        let opposite = cosine_similarity(&unit_x, &minus_x).expect("valid vectors");
        assert!((opposite + 1.0).abs() < 1e-6);

        assert!(cosine_similarity(&unit_x, &[1.0_f32; 7]).is_none());
        assert!(cosine_similarity(&[], &[]).is_none());
        assert!(cosine_similarity(&unit_x, &[0.0_f32; 8]).is_none());
    }

    #[test]
    fn secret_name_is_a_valid_vault_name() {
        let name = secret_name(Uuid::new_v4());
        assert!(name.starts_with(SECRET_PREFIX));
        assert!(hs_core::validation::secret_name(&name).is_ok());
    }
}
