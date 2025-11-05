-- Add vector memory tables for RAG (Retrieval-Augmented Generation)
--
-- This migration adds support for vector embeddings and semantic search
-- using sqlite-vec extension. Vector memories can be used for semantic
-- search over agent memories, documentation, code, and other content.

-- Create virtual table for vector storage using sqlite-vec
-- Note: This will be initialized by the VectorStore implementation
-- The vec0 virtual table provides efficient vector similarity search
CREATE TABLE IF NOT EXISTS vec_memory (
    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    embedding BLOB NOT NULL  -- Stores f32 vector as bytes (dimensions defined in code)
);

-- Create metadata table for vector memories
-- This table stores the actual content and metadata associated with each vector
CREATE TABLE IF NOT EXISTS vector_memory (
    id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    content TEXT NOT NULL,
    metadata TEXT,  -- JSON
    source_citation TEXT,  -- JSON with citation info
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    created_by TEXT NOT NULL DEFAULT 'system',
    vector_rowid INTEGER NOT NULL,  -- References vec_memory.rowid
    FOREIGN KEY (vector_rowid) REFERENCES vec_memory(rowid) ON DELETE CASCADE
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_vector_memory_namespace ON vector_memory(namespace);
CREATE INDEX IF NOT EXISTS idx_vector_memory_created ON vector_memory(created_at);
CREATE INDEX IF NOT EXISTS idx_vector_memory_vector_rowid ON vector_memory(vector_rowid);

-- Create table for tracking embedding model versions and configurations
CREATE TABLE IF NOT EXISTS embedding_models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model_name TEXT NOT NULL UNIQUE,  -- e.g., "all-MiniLM-L6-v2"
    model_type TEXT NOT NULL,  -- "local_minilm", "local_mpnet", "openai_ada002"
    dimensions INTEGER NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,  -- Boolean: currently active model
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Insert default embedding model
INSERT OR IGNORE INTO embedding_models (model_name, model_type, dimensions, is_active)
VALUES ('all-MiniLM-L6-v2', 'local_minilm', 384, 1);

-- Create table for chunking operations tracking
-- Useful for understanding how documents were split for vectorization
CREATE TABLE IF NOT EXISTS document_chunks (
    id TEXT PRIMARY KEY,
    parent_document_id TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    start_offset INTEGER,
    end_offset INTEGER,
    vector_memory_id TEXT,  -- References vector_memory.id
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (vector_memory_id) REFERENCES vector_memory(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_document_chunks_parent ON document_chunks(parent_document_id);
CREATE INDEX IF NOT EXISTS idx_document_chunks_vector ON document_chunks(vector_memory_id);

-- Create view for easy access to vector memories with their embeddings
CREATE VIEW IF NOT EXISTS vector_memory_full AS
SELECT
    vm.id,
    vm.namespace,
    vm.content,
    vm.metadata,
    vm.source_citation,
    vm.created_at,
    vm.updated_at,
    vm.created_by,
    vm.vector_rowid,
    v.embedding
FROM vector_memory vm
JOIN vec_memory v ON vm.vector_rowid = v.rowid;

-- Create view for namespace statistics
CREATE VIEW IF NOT EXISTS vector_namespace_stats AS
SELECT
    namespace,
    COUNT(*) as memory_count,
    MIN(created_at) as first_created,
    MAX(created_at) as last_created
FROM vector_memory
GROUP BY namespace
ORDER BY memory_count DESC;
