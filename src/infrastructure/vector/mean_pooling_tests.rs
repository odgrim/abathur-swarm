//! Additional unit tests for mean pooling implementation
//!
//! These tests validate the sentence-transformers mean pooling specification with known good outputs.

#[cfg(test)]
mod tests {
    use crate::domain::models::EmbeddingModel;
    use crate::infrastructure::vector::BertEmbeddingModel;
    use candle_core::Tensor;

    /// Test mean pooling with all padding (edge case)
    ///
    /// This test validates that mean pooling handles sequences that are entirely padding.
    /// With a mask of all zeros, the clamping to 1e-9 prevents division by zero.
    #[test]
    fn test_mean_pool_all_padding() {
        let model = BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create model");

        // Create test case: 1 sequence, 3 tokens, 4 dimensions
        // All tokens are padding (mask = [0, 0, 0])
        let hidden_states = vec![
            10.0, 20.0, 30.0, 40.0,  // Token 1 (padding)
            50.0, 60.0, 70.0, 80.0,  // Token 2 (padding)
            90.0, 100.0, 110.0, 120.0,  // Token 3 (padding)
        ];

        let attention_mask = vec![
            0.0, 0.0, 0.0,  // All tokens masked (padding)
        ];

        let hidden_tensor = Tensor::from_vec(
            hidden_states,
            (1, 3, 4),  // 1 sequence, 3 tokens, 4 dimensions
            &model.device,
        ).expect("Failed to create hidden states");

        let mask_tensor = Tensor::from_vec(
            attention_mask,
            (1, 3),  // 1 sequence, 3 tokens
            &model.device,
        ).expect("Failed to create attention mask");

        let result = model.mean_pool(&hidden_tensor, &mask_tensor);

        // Should handle gracefully without division by zero
        assert!(result.is_ok(), "Mean pooling should handle all-padding sequences");

        let pooled = result.unwrap();
        let vec: Vec<f32> = pooled
            .flatten_all().expect("Failed to flatten")
            .to_vec1().expect("Failed to convert");

        // With all padding, weighted sum is 0, divided by 1e-9 (clamped), result should be close to 0
        for &val in &vec {
            assert!(!val.is_nan(), "Should not produce NaN");
            assert!(!val.is_infinite(), "Should not produce Inf");
        }
    }

    /// Test mean pooling matches sentence-transformers specification
    ///
    /// This test validates the exact algorithm from sentence-transformers:
    /// ```python
    /// # Python reference implementation
    /// input_mask_expanded = attention_mask.unsqueeze(-1).expand(token_embeddings.size()).float()
    /// sum_embeddings = torch.sum(token_embeddings * input_mask_expanded, 1)
    /// sum_mask = torch.clamp(input_mask_expanded.sum(1), min=1e-9)
    /// embeddings = sum_embeddings / sum_mask
    /// ```
    #[test]
    fn test_mean_pool_sentence_transformers_spec() {
        let model = BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create model");

        // Real-world example: short text with padding
        // Sequence: "Hello world" = 4 tokens (including special tokens [CLS], [SEP])
        // Followed by 2 padding tokens
        // Simulated embeddings for each token (3 dimensions for simplicity)
        let hidden_states = vec![
            // Token 0: [CLS] token
            0.5, -0.3, 0.8,
            // Token 1: "Hello"
            0.2, 0.4, -0.1,
            // Token 2: "world"
            -0.3, 0.7, 0.2,
            // Token 3: [SEP] token
            0.1, -0.2, 0.5,
            // Token 4: [PAD] (should be ignored)
            999.0, 999.0, 999.0,
            // Token 5: [PAD] (should be ignored)
            888.0, 888.0, 888.0,
        ];

        let attention_mask = vec![
            1.0, 1.0, 1.0, 1.0, 0.0, 0.0,  // First 4 tokens active, last 2 padding
        ];

        let hidden_tensor = Tensor::from_vec(
            hidden_states.clone(),
            (1, 6, 3),  // 1 sequence, 6 tokens, 3 dimensions
            &model.device,
        ).expect("Failed to create hidden states");

        let mask_tensor = Tensor::from_vec(
            attention_mask.clone(),
            (1, 6),  // 1 sequence, 6 tokens
            &model.device,
        ).expect("Failed to create attention mask");

        let pooled = model.mean_pool(&hidden_tensor, &mask_tensor)
            .expect("Mean pooling failed");

        // Manual calculation following sentence-transformers spec:
        // Step 1: Multiply embeddings by mask (zeros out padding)
        // Token 0: [0.5, -0.3, 0.8] * 1.0 = [0.5, -0.3, 0.8]
        // Token 1: [0.2, 0.4, -0.1] * 1.0 = [0.2, 0.4, -0.1]
        // Token 2: [-0.3, 0.7, 0.2] * 1.0 = [-0.3, 0.7, 0.2]
        // Token 3: [0.1, -0.2, 0.5] * 1.0 = [0.1, -0.2, 0.5]
        // Token 4: [999.0, 999.0, 999.0] * 0.0 = [0.0, 0.0, 0.0]
        // Token 5: [888.0, 888.0, 888.0] * 0.0 = [0.0, 0.0, 0.0]

        // Step 2: Sum along token dimension
        // Sum = [0.5 + 0.2 + (-0.3) + 0.1, -0.3 + 0.4 + 0.7 + (-0.2), 0.8 + (-0.1) + 0.2 + 0.5]
        // Sum = [0.5, 0.6, 1.4]

        // Step 3: Count active tokens = 4 (mask sum = 1+1+1+1+0+0 = 4)

        // Step 4: Mean = Sum / Count = [0.5/4, 0.6/4, 1.4/4] = [0.125, 0.15, 0.35]

        let expected = vec![0.125, 0.15, 0.35];

        let result: Vec<f32> = pooled
            .flatten_all().expect("Failed to flatten")
            .to_vec1().expect("Failed to convert");

        assert_eq!(result.len(), 3, "Should have 3 dimensions");

        for (i, (actual, expected)) in result.iter().zip(expected.iter()).enumerate() {
            assert!(
                (actual - expected).abs() < 1e-6,
                "Dimension {}: expected {}, got {} (diff: {})",
                i, expected, actual, (actual - expected).abs()
            );
        }
    }

    /// Test mean pooling with single token (no padding)
    ///
    /// Edge case: sequence with only 1 real token (e.g., just [CLS])
    #[test]
    fn test_mean_pool_single_token() {
        let model = BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create model");

        let hidden_states = vec![
            0.5, -0.3, 0.8,  // Single token
        ];

        let attention_mask = vec![
            1.0,  // Single active token
        ];

        let hidden_tensor = Tensor::from_vec(
            hidden_states.clone(),
            (1, 1, 3),  // 1 sequence, 1 token, 3 dimensions
            &model.device,
        ).expect("Failed to create hidden states");

        let mask_tensor = Tensor::from_vec(
            attention_mask,
            (1, 1),  // 1 sequence, 1 token
            &model.device,
        ).expect("Failed to create attention mask");

        let pooled = model.mean_pool(&hidden_tensor, &mask_tensor)
            .expect("Mean pooling failed");

        let result: Vec<f32> = pooled
            .flatten_all().expect("Failed to flatten")
            .to_vec1().expect("Failed to convert");

        // Mean of single token should be the token itself
        let expected = vec![0.5, -0.3, 0.8];

        for (i, (actual, expected)) in result.iter().zip(expected.iter()).enumerate() {
            assert!(
                (actual - expected).abs() < 1e-6,
                "Dimension {}: expected {}, got {}",
                i, expected, actual
            );
        }
    }

    /// Test mean pooling with mixed batch (some sequences fully padded, some not)
    ///
    /// Tests batch processing with heterogeneous sequences
    #[test]
    fn test_mean_pool_mixed_batch() {
        let model = BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create model");

        // Batch of 3 sequences:
        // Sequence 0: 2 real tokens, 1 padding
        // Sequence 1: 3 real tokens, 0 padding
        // Sequence 2: 1 real token, 2 padding
        let hidden_states = vec![
            // Sequence 0
            1.0, 2.0,  // Token 0
            3.0, 4.0,  // Token 1
            999.0, 999.0,  // Token 2 (padding)
            // Sequence 1
            5.0, 6.0,  // Token 0
            7.0, 8.0,  // Token 1
            9.0, 10.0,  // Token 2
            // Sequence 2
            11.0, 12.0,  // Token 0
            888.0, 888.0,  // Token 1 (padding)
            777.0, 777.0,  // Token 2 (padding)
        ];

        let attention_mask = vec![
            1.0, 1.0, 0.0,  // Sequence 0: 2 real, 1 padding
            1.0, 1.0, 1.0,  // Sequence 1: 3 real
            1.0, 0.0, 0.0,  // Sequence 2: 1 real, 2 padding
        ];

        let hidden_tensor = Tensor::from_vec(
            hidden_states,
            (3, 3, 2),  // 3 sequences, 3 tokens, 2 dimensions
            &model.device,
        ).expect("Failed to create hidden states");

        let mask_tensor = Tensor::from_vec(
            attention_mask,
            (3, 3),  // 3 sequences, 3 tokens
            &model.device,
        ).expect("Failed to create attention mask");

        let pooled = model.mean_pool(&hidden_tensor, &mask_tensor)
            .expect("Mean pooling failed");

        let result: Vec<f32> = pooled
            .flatten_all().expect("Failed to flatten")
            .to_vec1().expect("Failed to convert");

        // Expected results:
        // Sequence 0: mean([1,2], [3,4]) = [(1+3)/2, (2+4)/2] = [2.0, 3.0]
        // Sequence 1: mean([5,6], [7,8], [9,10]) = [(5+7+9)/3, (6+8+10)/3] = [7.0, 8.0]
        // Sequence 2: mean([11,12]) = [11.0, 12.0] (single token)
        let expected = vec![
            2.0, 3.0,    // Sequence 0
            7.0, 8.0,    // Sequence 1
            11.0, 12.0,  // Sequence 2
        ];

        assert_eq!(result.len(), 6, "Should have 3 sequences * 2 dimensions = 6 values");

        for (i, (actual, expected)) in result.iter().zip(expected.iter()).enumerate() {
            assert!(
                (actual - expected).abs() < 1e-5,
                "Element {}: expected {}, got {}",
                i, expected, actual
            );
        }
    }

    /// Test mean pooling numerical stability with very large and very small values
    #[test]
    fn test_mean_pool_numerical_stability() {
        let model = BertEmbeddingModel::new(EmbeddingModel::LocalMiniLM)
            .expect("Failed to create model");

        // Test with extreme values
        let hidden_states = vec![
            1e6, -1e6, 1e-6,   // Very large positive, very large negative, very small
            2e6, -2e6, 2e-6,
            3e6, -3e6, 3e-6,
        ];

        let attention_mask = vec![
            1.0, 1.0, 1.0,  // All tokens active
        ];

        let hidden_tensor = Tensor::from_vec(
            hidden_states,
            (1, 3, 3),  // 1 sequence, 3 tokens, 3 dimensions
            &model.device,
        ).expect("Failed to create hidden states");

        let mask_tensor = Tensor::from_vec(
            attention_mask,
            (1, 3),  // 1 sequence, 3 tokens
            &model.device,
        ).expect("Failed to create attention mask");

        let pooled = model.mean_pool(&hidden_tensor, &mask_tensor)
            .expect("Mean pooling failed");

        let result: Vec<f32> = pooled
            .flatten_all().expect("Failed to flatten")
            .to_vec1().expect("Failed to convert");

        // Expected: mean of each dimension
        // Dim 0: (1e6 + 2e6 + 3e6) / 3 = 2e6
        // Dim 1: (-1e6 + -2e6 + -3e6) / 3 = -2e6
        // Dim 2: (1e-6 + 2e-6 + 3e-6) / 3 = 2e-6
        let expected = vec![2e6, -2e6, 2e-6];

        for (i, (actual, expected)) in result.iter().zip(expected.iter()).enumerate() {
            // Should not have NaN or Inf
            assert!(!actual.is_nan(), "Dimension {} should not be NaN", i);
            assert!(!actual.is_infinite(), "Dimension {} should not be Inf", i);

            // Check relative error (more robust for extreme values)
            let rel_error = if expected.abs() > 1e-10 {
                ((actual - expected) / expected).abs()
            } else {
                (actual - expected).abs()
            };

            assert!(
                rel_error < 1e-5,
                "Dimension {}: expected {}, got {}, relative error: {}",
                i, expected, actual, rel_error
            );
        }
    }
}
