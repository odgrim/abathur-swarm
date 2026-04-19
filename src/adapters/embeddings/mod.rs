//! Embedding provider adapters.
//!
//! All adapters in this module MUST return unit-length (L2-normalized)
//! vectors from [`EmbeddingProvider::embed`] and
//! [`EmbeddingProvider::embed_batch`]. Downstream consumers (gap fingerprint
//! merge, memory cosine similarity) are permitted to rely on that invariant.
//!
//! Use [`normalize_unit`] to enforce it on any newly returned vector.

pub mod openai;

pub use openai::{OpenAiEmbeddingConfig, OpenAiEmbeddingProvider};

/// Normalize `v` in place to unit L2 length.
///
/// If the L2 norm is zero, sub-normal, or non-finite, logs a warning and
/// leaves `v` untouched. This matches what OpenAI `text-embedding-3-small`
/// returns in practice and gives downstream cosine-similarity / centroid-
/// averaging code a clean invariant to depend on.
pub fn normalize_unit(v: &mut [f32]) {
    if v.is_empty() {
        return;
    }
    // Use f64 accumulation to avoid loss of precision in large dims.
    let norm_sq: f64 = v.iter().map(|x| (*x as f64) * (*x as f64)).sum();
    let norm = norm_sq.sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        tracing::warn!(
            dim = v.len(),
            norm = norm,
            "embedding has degenerate norm; leaving unnormalized"
        );
        return;
    }
    let inv = (1.0 / norm) as f32;
    for x in v.iter_mut() {
        *x *= inv;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unit_vector_stays_unit() {
        // Already unit-length: (1,0,0).
        let mut v = vec![1.0_f32, 0.0, 0.0];
        normalize_unit(&mut v);
        assert!((v[0] - 1.0).abs() < 1e-6);
        assert!(v[1].abs() < 1e-6);
        assert!(v[2].abs() < 1e-6);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_arbitrary_vector_gets_l2_normalized() {
        let mut v = vec![3.0_f32, 4.0]; // norm = 5
        normalize_unit(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_zero_vector_is_left_alone() {
        let mut v = vec![0.0_f32, 0.0, 0.0];
        normalize_unit(&mut v);
        // No panic and no NaNs introduced.
        for x in &v {
            assert_eq!(*x, 0.0);
        }
    }

    #[test]
    fn normalize_empty_vector_is_noop() {
        let mut v: Vec<f32> = Vec::new();
        normalize_unit(&mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn normalize_non_finite_vector_is_left_alone() {
        let mut v = vec![f32::NAN, 1.0];
        normalize_unit(&mut v);
        // First element stays NaN; second element unchanged from 1.0.
        assert!(v[0].is_nan());
        assert_eq!(v[1], 1.0);
    }
}
