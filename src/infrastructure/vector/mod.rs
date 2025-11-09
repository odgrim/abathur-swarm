//! Vector infrastructure components
//!
//! Provides implementations for embedding generation, text chunking,
//! and vector storage for semantic search (RAG).

pub mod bert_error;
pub mod bert_model;
pub mod chunker;
pub mod embedding_service;
pub mod model_cache;
pub mod vector_store;

pub use bert_error::{BertError, BertResult};
pub use bert_model::BertEmbeddingModel;
pub use chunker::Chunker;
pub use embedding_service::LocalEmbeddingService;
pub use model_cache::{ModelCache, ModelPaths, RetryPolicy};
pub use vector_store::{VectorImplementation, VectorStore};
