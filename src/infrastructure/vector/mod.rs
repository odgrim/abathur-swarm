//! Vector infrastructure components
//!
//! Provides implementations for embedding generation, text chunking,
//! and vector storage for semantic search (RAG).

pub mod chunker;
pub mod embedding_service;
pub mod model_cache;
pub mod vector_store;

pub use chunker::Chunker;
pub use embedding_service::LocalEmbeddingService;
pub use model_cache::{CacheStats, ModelCache};
pub use vector_store::VectorStore;
