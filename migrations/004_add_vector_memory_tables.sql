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
