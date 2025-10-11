-- ================================================================
-- DDL Script: Memory Management Tables
-- Purpose: New tables for session management, long-term memory, and document indexing
-- Phase: Phase 2 Technical Specifications
-- Author: technical-specifications-writer
-- Date: 2025-10-10
-- ================================================================

-- Prerequisites: SQLite 3.35+ with JSON functions support
-- Execution: Run BEFORE ddl-core-tables.sql (provides foreign key targets)
-- Dependencies: None (self-contained new tables)

-- ================================================================
-- TABLE: sessions
-- PURPOSE: Core session management with event tracking and state storage
-- DESCRIPTION: Stores conversation threads with chronological events and ephemeral state
-- RELATIONSHIPS: Referenced by tasks, agents, checkpoints, memory_entries
-- ================================================================

CREATE TABLE IF NOT EXISTS sessions (
    -- ===== Primary Identifiers =====
    id TEXT PRIMARY KEY,                          -- UUID v4: unique session identifier
    app_name TEXT NOT NULL,                       -- Application context (e.g., "abathur")
    user_id TEXT NOT NULL,                        -- User identifier
    project_id TEXT,                              -- Optional project association (for cross-agent collaboration)

    -- ===== Lifecycle Management =====
    status TEXT NOT NULL DEFAULT 'created',       -- Session lifecycle state

    -- ===== Session Data (JSON Columns) =====
    events TEXT NOT NULL DEFAULT '[]',            -- JSON array of Event objects (chronological message history)
    state TEXT NOT NULL DEFAULT '{}',             -- JSON dict of session state (key-value with namespace prefixes)

    -- ===== Metadata =====
    metadata TEXT DEFAULT '{}',                   -- JSON dict for extensibility (tags, labels, custom fields)

    -- ===== Timestamps =====
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_update_time TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    terminated_at TIMESTAMP,                      -- When session ended
    archived_at TIMESTAMP,                        -- When session moved to cold storage

    -- ===== Constraints =====
    CHECK(status IN ('created', 'active', 'paused', 'terminated', 'archived')),
    CHECK(json_valid(events)),                    -- Ensure events is valid JSON array
    CHECK(json_valid(state)),                     -- Ensure state is valid JSON object
    CHECK(json_valid(metadata))                   -- Ensure metadata is valid JSON object
);

-- ================================================================
-- SESSIONS TABLE USAGE NOTES
-- ================================================================

-- Events JSON Structure Example:
-- [
--   {
--     "event_id": "evt_001",
--     "timestamp": "2025-10-10T10:00:00Z",
--     "event_type": "message",
--     "actor": "user",
--     "content": {"message": "Design the memory schema"},
--     "state_delta": {"session:current_task": "memory_architecture"},
--     "is_final_response": false
--   },
--   {
--     "event_id": "evt_002",
--     "timestamp": "2025-10-10T10:01:00Z",
--     "event_type": "action",
--     "actor": "agent:memory-systems-architect",
--     "content": {"action": "analyze_chapter", "file": "Chapter 8.md"},
--     "state_delta": {"session:analysis_complete": true},
--     "is_final_response": false
--   }
-- ]

-- State JSON Structure Example:
-- {
--   "session:abc123:current_task": "schema_redesign",
--   "session:abc123:progress_steps": [1, 2, 3],
--   "temp:validation_needed": true,
--   "user:alice:last_interaction": "2025-10-10T09:00:00Z"
-- }

-- Lifecycle State Transitions:
-- CREATED → ACTIVE → (PAUSED ↔ ACTIVE)* → TERMINATED → ARCHIVED

-- ================================================================
-- TABLE: memory_entries
-- PURPOSE: Long-term persistent memory storage with hierarchical namespaces and versioning
-- DESCRIPTION: Stores semantic, episodic, and procedural memories with version history
-- RELATIONSHIPS: Referenced by audit table for memory operation tracking
-- ================================================================

CREATE TABLE IF NOT EXISTS memory_entries (
    -- ===== Primary Key =====
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- ===== Hierarchical Namespace =====
    namespace TEXT NOT NULL,                      -- Hierarchical path (e.g., "user:alice:preferences", "project:schema_redesign:status")
    key TEXT NOT NULL,                            -- Memory key (unique within namespace+version)

    -- ===== Memory Content =====
    value TEXT NOT NULL,                          -- JSON-serialized memory content
    memory_type TEXT NOT NULL,                    -- Memory classification

    -- ===== Versioning and Soft-Delete =====
    version INTEGER NOT NULL DEFAULT 1,           -- Version number (increments on update)
    is_deleted BOOLEAN NOT NULL DEFAULT 0,        -- Soft-delete flag (0=active, 1=deleted)

    -- ===== Metadata =====
    metadata TEXT DEFAULT '{}',                   -- JSON dict for extensibility (tags, importance_score, consolidation_notes)

    -- ===== Audit Trail =====
    created_by TEXT,                              -- Session or agent ID that created this entry
    updated_by TEXT,                              -- Session or agent ID that last updated this entry

    -- ===== Timestamps =====
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- ===== Constraints =====
    CHECK(memory_type IN ('semantic', 'episodic', 'procedural')),
    CHECK(json_valid(value)),                     -- Ensure value is valid JSON
    CHECK(json_valid(metadata)),                  -- Ensure metadata is valid JSON
    CHECK(version > 0),                           -- Version must be positive integer
    UNIQUE(namespace, key, version)               -- Enforce unique versions per key
);

-- ================================================================
-- MEMORY_ENTRIES TABLE USAGE NOTES
-- ================================================================

-- Memory Type Definitions:
-- - semantic: Facts, preferences, domain knowledge (permanent, manually managed)
-- - episodic: Past events, task history, experiences (30-90 day TTL, append-only)
-- - procedural: Rules, instructions, learned behaviors (permanent, versioned refinement)

-- Namespace Hierarchy Examples:
-- - project:schema_redesign:status             (project-wide shared state)
-- - user:alice:preferences                     (user-specific across sessions)
-- - app:abathur:agent_instructions             (application-wide shared config)
-- - session:abc123:draft                       (session-specific temporary, promoted to user: on publish)

-- Version Management Pattern:
-- - Create: INSERT with version=1
-- - Update: INSERT new row with version=previous_version+1
-- - Delete: UPDATE SET is_deleted=1 on current version (soft-delete)
-- - Retrieve Current: SELECT WHERE is_deleted=0 ORDER BY version DESC LIMIT 1
-- - Rollback: SET is_deleted=1 on current version, restore previous version

-- ================================================================
-- TABLE: document_index
-- PURPOSE: Index for markdown documents with embeddings for semantic search
-- DESCRIPTION: Hybrid storage model - markdown files as source, SQLite as search index
-- RELATIONSHIPS: Standalone (no foreign keys), links to files on disk
-- ================================================================

CREATE TABLE IF NOT EXISTS document_index (
    -- ===== Primary Key =====
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- ===== Document Identification =====
    file_path TEXT NOT NULL UNIQUE,               -- Absolute path to markdown file (source of truth)
    title TEXT NOT NULL,                          -- Document title (extracted from frontmatter or first # heading)
    document_type TEXT,                           -- Categorization: design|specification|plan|report

    -- ===== Content Tracking =====
    content_hash TEXT NOT NULL,                   -- SHA-256 hash of file content (detect changes for re-embedding)
    chunk_count INTEGER DEFAULT 1,                -- Number of chunks this document was split into

    -- ===== Embeddings (BLOB for sqlite-vss integration) =====
    embedding_model TEXT,                         -- Model used for embedding (e.g., "nomic-embed-text-v1.5")
    embedding_blob BLOB,                          -- Serialized embedding vector (JSON array or binary format)

    -- ===== Metadata =====
    metadata TEXT DEFAULT '{}',                   -- JSON dict: author, tags, phase, project_id, etc.

    -- ===== Sync Tracking =====
    last_synced_at TIMESTAMP,                     -- When embeddings were last generated/updated
    sync_status TEXT DEFAULT 'pending',           -- Embedding sync state

    -- ===== Timestamps =====
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- ===== Constraints =====
    CHECK(sync_status IN ('pending', 'synced', 'failed', 'stale')),
    CHECK(json_valid(metadata))                   -- Ensure metadata is valid JSON
);

-- ================================================================
-- DOCUMENT_INDEX TABLE USAGE NOTES
-- ================================================================

-- Document Storage Model (Hybrid):
-- - Source of Truth: Markdown files on disk (git tracked, human readable)
-- - Index: document_index table (embeddings, metadata, fast retrieval)
-- - Sync Workflow: File watcher detects changes → update content_hash → set sync_status='stale' → background service re-embeds

-- Embedding Storage Format (embedding_blob):
-- - JSON Array: [0.123, -0.456, 0.789, ...] (768 floats for nomic-embed-text-v1.5)
-- - Binary Format: Packed float32 array (more efficient, 768 * 4 = 3072 bytes)
-- - Chunk Handling: For multi-chunk documents, store separate rows with chunk_index in metadata

-- Sync Status Lifecycle:
-- - pending: New document, embeddings not yet generated
-- - synced: Embeddings up-to-date with file content
-- - failed: Embedding generation failed (error logged in metadata)
-- - stale: File content changed (content_hash mismatch), needs re-embedding

-- Metadata JSON Examples:
-- {
--   "author": "memory-systems-architect",
--   "phase": "phase1_design",
--   "project_id": "schema_redesign",
--   "tags": ["memory", "architecture", "design"],
--   "word_count": 15000,
--   "chunk_index": 0
-- }

-- ================================================================
-- EXECUTION VERIFICATION
-- ================================================================

-- After executing this script, verify table creation:
-- SELECT name FROM sqlite_master WHERE type='table' AND name IN ('sessions', 'memory_entries', 'document_index');

-- Check table schemas:
-- PRAGMA table_info(sessions);
-- PRAGMA table_info(memory_entries);
-- PRAGMA table_info(document_index);

-- Verify JSON validation constraints:
-- INSERT INTO sessions (id, app_name, user_id, events) VALUES ('test', 'app', 'user', 'invalid json');
-- Expected: Error: CHECK constraint failed: json_valid(events)

-- ================================================================
-- INTEGRATION NOTES
-- ================================================================

-- For fresh start projects:
-- 1. Run this script FIRST (provides foreign key targets for ddl-core-tables.sql)
-- 2. Then run ddl-core-tables.sql (adds session_id foreign keys)
-- 3. Finally run ddl-indexes.sql (creates all performance indexes)

-- For existing databases (migration scenario):
-- 1. Backup database: cp abathur.db abathur.db.backup
-- 2. Run this script to add new tables
-- 3. Run ddl-core-tables.sql for enhanced tables (ALTER TABLE commands)
-- 4. Run ddl-indexes.sql to create indexes

-- ================================================================
-- END OF SCRIPT
-- ================================================================
