//! FinBERT ONNX sentiment scorer — feature-gated behind `sentiment`.
//!
//! Uses `ort` (ONNX Runtime) for inference and `tokenizers` for WordPiece tokenization.
//! The model + vocab are loaded lazily on first use via `OnceLock`.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::analysis::screener::SentimentScorer;
use crate::analysis::sentiment::{
    aggregate_sentiments, article_sentiment_from_probs, ArticleSentiment,
};
use crate::error::AnalysisError;

/// Maximum batch size for ONNX inference.
const MAX_BATCH_SIZE: usize = 32;

/// FinBERT-based sentiment scorer using ONNX Runtime.
///
/// Wraps a pre-trained FinBERT model (3-class: positive, negative, neutral)
/// and a WordPiece tokenizer. Both are loaded lazily on first inference call.
pub struct FinBertScorer {
    session: OnceLock<Result<Mutex<Session>, String>>,
    tokenizer: OnceLock<Result<Tokenizer, String>>,
    pub model_path: String,
    pub half_life_days: f64,
}

// Safety: Session and Tokenizer are Send+Sync in practice
unsafe impl Send for FinBertScorer {}
unsafe impl Sync for FinBertScorer {}

impl FinBertScorer {
    /// Create a new scorer pointing at the given model directory.
    ///
    /// The directory should contain:
    /// - `model.onnx` — the FinBERT ONNX model
    /// - `tokenizer.json` — the HuggingFace tokenizer config
    pub fn new(model_path: String, half_life_days: f64) -> Self {
        Self {
            session: OnceLock::new(),
            tokenizer: OnceLock::new(),
            model_path,
            half_life_days,
        }
    }

    /// Lazily initialize the ONNX session.
    fn get_session(&self) -> Result<&Mutex<Session>, AnalysisError> {
        let result = self.session.get_or_init(|| {
            let model_file = Path::new(&self.model_path).join("model.onnx");
            Session::builder()
                .and_then(|b| b.with_optimization_level(GraphOptimizationLevel::Level3))
                .and_then(|b| b.commit_from_file(&model_file))
                .map(Mutex::new)
                .map_err(|e| format!("Failed to load model from {}: {}", model_file.display(), e))
        });
        result
            .as_ref()
            .map_err(|e| AnalysisError::OnnxInference(e.clone()))
    }

    /// Lazily initialize the tokenizer.
    fn get_tokenizer(&self) -> Result<&Tokenizer, AnalysisError> {
        let result = self.tokenizer.get_or_init(|| {
            let tokenizer_file = Path::new(&self.model_path).join("tokenizer.json");
            Tokenizer::from_file(&tokenizer_file).map_err(|e| {
                format!(
                    "Failed to load tokenizer from {}: {}",
                    tokenizer_file.display(),
                    e
                )
            })
        });
        result
            .as_ref()
            .map_err(|e| AnalysisError::Tokenizer(e.clone()))
    }

    /// Run inference on a batch of headlines, returning softmax probabilities.
    ///
    /// Returns `Vec<[f64; 3]>` where each element is `[positive, negative, neutral]`.
    pub fn infer_batch(&self, headlines: &[&str]) -> Result<Vec<[f64; 3]>, AnalysisError> {
        if headlines.is_empty() {
            return Ok(Vec::new());
        }

        let session_mutex = self.get_session()?;
        let mut session = session_mutex
            .lock()
            .map_err(|e| AnalysisError::OnnxInference(format!("Session lock poisoned: {}", e)))?;
        let tokenizer = self.get_tokenizer()?;

        let mut all_probs = Vec::with_capacity(headlines.len());

        // Process in chunks of MAX_BATCH_SIZE
        for chunk in headlines.chunks(MAX_BATCH_SIZE) {
            let encodings = tokenizer
                .encode_batch(chunk.to_vec(), true)
                .map_err(|e| AnalysisError::Tokenizer(e.to_string()))?;

            let seq_len = encodings
                .iter()
                .map(|e| e.get_ids().len())
                .max()
                .unwrap_or(0);
            let batch_size = encodings.len();

            // Build flattened input tensors
            let mut input_ids: Vec<i64> = Vec::with_capacity(batch_size * seq_len);
            let mut attention_mask: Vec<i64> = Vec::with_capacity(batch_size * seq_len);
            let mut token_type_ids: Vec<i64> = Vec::with_capacity(batch_size * seq_len);

            for encoding in &encodings {
                let ids = encoding.get_ids();
                let mask = encoding.get_attention_mask();
                let types = encoding.get_type_ids();

                for i in 0..seq_len {
                    input_ids.push(if i < ids.len() { ids[i] as i64 } else { 0 });
                    attention_mask.push(if i < mask.len() { mask[i] as i64 } else { 0 });
                    token_type_ids.push(if i < types.len() { types[i] as i64 } else { 0 });
                }
            }

            let ids_tensor = Tensor::<i64>::from_array(([batch_size, seq_len], input_ids))
                .map_err(|e| AnalysisError::OnnxInference(e.to_string()))?;

            let mask_tensor = Tensor::<i64>::from_array(([batch_size, seq_len], attention_mask))
                .map_err(|e| AnalysisError::OnnxInference(e.to_string()))?;

            let type_tensor = Tensor::<i64>::from_array(([batch_size, seq_len], token_type_ids))
                .map_err(|e| AnalysisError::OnnxInference(e.to_string()))?;

            let outputs = session
                .run(ort::inputs![ids_tensor, mask_tensor, type_tensor])
                .map_err(|e| AnalysisError::OnnxInference(e.to_string()))?;

            // Extract logits and apply softmax
            let (shape, data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| AnalysisError::OnnxInference(e.to_string()))?;

            // shape is [batch_size, 3] — iterate over each sample
            let num_classes = if shape.len() >= 2 {
                shape[shape.len() - 1] as usize
            } else {
                3
            };
            for sample_idx in 0..batch_size {
                let offset = sample_idx * num_classes;
                if offset + 3 <= data.len() {
                    let probs = softmax(&[
                        data[offset] as f64,
                        data[offset + 1] as f64,
                        data[offset + 2] as f64,
                    ]);
                    all_probs.push(probs);
                }
            }
        }

        Ok(all_probs)
    }
}

/// Softmax over 3 logits → probabilities that sum to 1.
fn softmax(logits: &[f64; 3]) -> [f64; 3] {
    let max = logits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    [exps[0] / sum, exps[1] / sum, exps[2] / sum]
}

impl SentimentScorer for FinBertScorer {
    fn score(&self, headlines: &[&str]) -> Result<f64, AnalysisError> {
        if headlines.is_empty() {
            return Ok(0.0);
        }

        let probs = self.infer_batch(headlines)?;
        let now = chrono::Utc::now();

        let articles: Vec<ArticleSentiment> = probs
            .iter()
            .map(|p| article_sentiment_from_probs(*p, now))
            .collect();

        let result = aggregate_sentiments(&articles, now, self.half_life_days);
        Ok(result.compound)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────
//
// These tests are feature-gated and only run with `--features sentiment`.
// Most tests validate pure functions (softmax, batching logic) rather than
// requiring an actual model file.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_sums_to_one() {
        let probs = softmax(&[2.0, 1.0, 0.5]);
        let sum: f64 = probs.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "Softmax should sum to 1.0, got {}",
            sum
        );
    }

    #[test]
    fn softmax_ordering_preserved() {
        let probs = softmax(&[3.0, 1.0, 0.5]);
        assert!(
            probs[0] > probs[1],
            "Largest logit should have largest prob"
        );
        assert!(probs[1] > probs[2], "Second logit should have second prob");
    }

    #[test]
    fn softmax_equal_inputs() {
        let probs = softmax(&[1.0, 1.0, 1.0]);
        for p in &probs {
            assert!(
                (*p - 1.0 / 3.0).abs() < 1e-6,
                "Equal inputs should give uniform probs"
            );
        }
    }

    #[test]
    fn softmax_large_values_no_overflow() {
        let probs = softmax(&[1000.0, 999.0, 998.0]);
        let sum: f64 = probs.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "Softmax should handle large values"
        );
        assert!(probs[0] > probs[1]);
    }

    #[test]
    fn scorer_empty_headlines_returns_zero() {
        let scorer = FinBertScorer::new("/nonexistent".to_string(), 3.5);
        // Empty headlines should short-circuit without loading model
        let result = scorer.score(&[]).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn scorer_new_creates_instance() {
        let scorer = FinBertScorer::new("/models/finbert".to_string(), 3.5);
        assert_eq!(scorer.model_path, "/models/finbert");
        assert!((scorer.half_life_days - 3.5).abs() < 1e-6);
    }

    #[test]
    fn model_load_failure_returns_error() {
        // This test exercises the ONNX session init path. When the ONNX Runtime
        // shared library isn't installed (load-dynamic mode), ort panics before
        // we reach our error handling. Catch that case gracefully.
        let result = std::panic::catch_unwind(|| {
            let scorer = FinBertScorer::new("/nonexistent/path".to_string(), 3.5);
            scorer.infer_batch(&["test headline"])
        });
        match result {
            Ok(inner) => assert!(inner.is_err(), "Should fail with nonexistent model path"),
            Err(_) => {} // ONNX Runtime dylib not installed — expected in dev/CI
        }
    }

    #[test]
    fn max_batch_size_constant() {
        assert_eq!(MAX_BATCH_SIZE, 32);
    }
}
