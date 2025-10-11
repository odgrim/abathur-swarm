# Milestone 3: Vector Search Integration

## Overview

**Goal:** Enable semantic search capabilities with sqlite-vss extension and Ollama embedding generation

**Timeline:** Weeks 5-6 (10 business days)

**Dependencies:**
- Milestone 2 complete and validated (memory system operational)
- document_index table deployed
- Ollama infrastructure accessible

---

## Objectives

1. Install and configure sqlite-vss extension for vector similarity search
2. Setup Ollama with nomic-embed-text-v1.5 embedding model (768 dimensions)
3. Implement embedding generation service for markdown documents
4. Build semantic search query interface
5. Create background sync service for automatic markdown indexing
6. Validate performance targets (<500ms semantic search latency)

---

## Tasks and Effort Estimates

### Week 5: sqlite-vss Setup and Embedding Service

| Task | Description | Effort (hours) | Owner | Dependencies |
|------|-------------|---------------|-------|--------------|
| **5.1** | Research sqlite-vss installation for target OS (macOS/Linux) | 2h | Dev Team | Milestone 2 complete |
| **5.2** | Install sqlite-vss extension and verify loading | 4h | DevOps | Task 5.1 |
| **5.3** | Create virtual table for document embeddings | 3h | Dev Team | Task 5.2 |
| **5.4** | Setup Ollama server with nomic-embed-text-v1.5 model | 4h | DevOps | External dependency |
| **5.5** | Implement embedding generation client (Ollama API) | 6h | Dev Team | Task 5.4 |
| **5.6** | Create document chunking strategy (512 token chunks) | 4h | Dev Team | Task 5.5 |
| **5.7** | Implement embedding batch generation (10 docs/batch) | 4h | Dev Team | Task 5.6 |
| **5.8** | Write unit tests for embedding generation | 4h | Dev Team | Task 5.7 |
| **5.9** | Create embedding storage logic (BLOB serialization) | 4h | Dev Team | Task 5.3 |
| **5.10** | Validate embedding dimensions (768) and format | 2h | Dev Team | Task 5.9 |

**Week 5 Total:** 37 hours

### Week 6: Semantic Search and Background Sync

| Task | Description | Effort (hours) | Owner | Dependencies |
|------|-------------|---------------|-------|--------------|
| **6.1** | Implement semantic search query interface | 6h | Dev Team | Week 5 complete |
| **6.2** | Create hybrid search (exact + semantic) | 4h | Dev Team | Task 6.1 |
| **6.3** | Implement re-ranking logic (combine scores) | 4h | Dev Team | Task 6.2 |
| **6.4** | Build background sync service for markdown files | 8h | Dev Team | Task 5.7 |
| **6.5** | Implement file watcher for automatic re-indexing | 4h | Dev Team | Task 6.4 |
| **6.6** | Create content hash verification (detect changes) | 3h | Dev Team | Task 6.4 |
| **6.7** | Write integration tests for semantic search | 6h | Dev Team | Task 6.1 |
| **6.8** | Performance testing (semantic search <500ms) | 4h | Dev Team | Task 6.1 |
| **6.9** | Optimize embedding retrieval (index tuning) | 4h | Dev Team | Task 6.8 |
| **6.10** | Document semantic search API and usage examples | 4h | Dev Team | All tasks |

**Week 6 Total:** 47 hours

**Milestone 3 Total Effort:** 84 hours (~2 developers × 1 week)

---

## Deliverables

### 1. sqlite-vss Extension Integration

**Installation (macOS):**
```bash
# Install sqlite-vss via Homebrew (if available) or build from source
brew install sqlite-vss

# Or build from source
git clone https://github.com/asg017/sqlite-vss.git
cd sqlite-vss
make loadable
# Copy vss0.so to SQLite extensions directory
```

**Installation (Linux):**
```bash
# Ubuntu/Debian
wget https://github.com/asg017/sqlite-vss/releases/download/v0.1.2/vss0.so
sudo mv vss0.so /usr/lib/sqlite3/

# Or build from source
git clone https://github.com/asg017/sqlite-vss.git
cd sqlite-vss && make loadable
sudo cp vss0.so /usr/lib/sqlite3/
```

**Load Extension:**
```python
import aiosqlite

async def load_vss_extension(conn: aiosqlite.Connection):
    """Load sqlite-vss extension for vector search."""
    await conn.enable_load_extension(True)
    await conn.load_extension('/usr/lib/sqlite3/vss0')
    await conn.enable_load_extension(False)
```

**Create Virtual Table:**
```sql
-- Create virtual table for document embeddings
CREATE VIRTUAL TABLE document_embeddings USING vss0(
    embedding(768)  -- nomic-embed-text-v1.5 produces 768-dimensional embeddings
);

-- Link to document_index table
CREATE TABLE document_embeddings_data (
    rowid INTEGER PRIMARY KEY,
    document_id INTEGER NOT NULL,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    embedding BLOB NOT NULL,
    FOREIGN KEY (document_id) REFERENCES document_index(id)
);
```

### 2. Ollama Embedding Service

**Ollama Setup:**
```bash
# Install Ollama (macOS)
brew install ollama

# Or download from https://ollama.ai
curl -fsSL https://ollama.ai/install.sh | sh

# Pull nomic-embed-text-v1.5 model
ollama pull nomic-embed-text:latest
```

**Embedding Generation Client:**

**File:** `src/abathur/infrastructure/embedding_service.py`

```python
import aiohttp
import numpy as np
from typing import List, Dict

class EmbeddingService:
    """Service for generating text embeddings via Ollama."""

    def __init__(self, ollama_url: str = "http://localhost:11434"):
        self.ollama_url = ollama_url
        self.model = "nomic-embed-text:latest"

    async def generate_embedding(self, text: str) -> List[float]:
        """Generate 768-dimensional embedding for text.

        Args:
            text: Input text (max 8192 tokens for nomic-embed-text)

        Returns:
            768-dimensional embedding vector

        Raises:
            ValueError: If text exceeds token limit
            RuntimeError: If Ollama server unreachable
        """
        async with aiohttp.ClientSession() as session:
            async with session.post(
                f"{self.ollama_url}/api/embeddings",
                json={"model": self.model, "prompt": text}
            ) as response:
                if response.status != 200:
                    raise RuntimeError(f"Ollama error: {await response.text()}")

                result = await response.json()
                embedding = result['embedding']

                if len(embedding) != 768:
                    raise ValueError(f"Expected 768 dimensions, got {len(embedding)}")

                return embedding

    async def generate_batch_embeddings(
        self, texts: List[str]
    ) -> List[List[float]]:
        """Generate embeddings for batch of texts.

        Args:
            texts: List of input texts

        Returns:
            List of 768-dimensional embedding vectors
        """
        embeddings = []
        for text in texts:
            embedding = await self.generate_embedding(text)
            embeddings.append(embedding)
        return embeddings

    def serialize_embedding(self, embedding: List[float]) -> bytes:
        """Serialize embedding to BLOB for SQLite storage."""
        return np.array(embedding, dtype=np.float32).tobytes()

    def deserialize_embedding(self, blob: bytes) -> List[float]:
        """Deserialize BLOB to embedding vector."""
        return np.frombuffer(blob, dtype=np.float32).tolist()
```

### 3. Document Chunking Strategy

**File:** `src/abathur/infrastructure/document_chunker.py`

```python
class DocumentChunker:
    """Chunk documents into 512-token segments for embedding generation."""

    def __init__(self, chunk_size: int = 512, overlap: int = 50):
        self.chunk_size = chunk_size
        self.overlap = overlap

    def chunk_text(self, text: str) -> List[Dict[str, any]]:
        """Chunk text into overlapping segments.

        Args:
            text: Input document text

        Returns:
            List of chunks with metadata:
            [
                {
                    "chunk_index": 0,
                    "text": "...",
                    "start_pos": 0,
                    "end_pos": 512,
                    "token_count": 512
                },
                ...
            ]
        """
        # Simple word-based chunking (can be improved with tokenizer)
        words = text.split()
        chunks = []
        chunk_index = 0

        for i in range(0, len(words), self.chunk_size - self.overlap):
            chunk_words = words[i:i + self.chunk_size]
            chunk_text = " ".join(chunk_words)

            chunks.append({
                "chunk_index": chunk_index,
                "text": chunk_text,
                "start_pos": i,
                "end_pos": i + len(chunk_words),
                "token_count": len(chunk_words)
            })
            chunk_index += 1

        return chunks
```

### 4. Semantic Search Interface

**File:** `src/abathur/infrastructure/semantic_search_service.py`

```python
class SemanticSearchService:
    """Service for semantic similarity search using sqlite-vss."""

    def __init__(self, db: Database, embedding_service: EmbeddingService):
        self.db = db
        self.embedding_service = embedding_service

    async def semantic_search(
        self,
        query: str,
        limit: int = 10,
        document_type: Optional[str] = None
    ) -> List[Dict[str, Any]]:
        """Search documents by semantic similarity.

        Args:
            query: Search query text
            limit: Maximum results to return
            document_type: Optional filter by document type

        Returns:
            List of documents with similarity scores:
            [
                {
                    "document_id": 123,
                    "file_path": "/path/to/doc.md",
                    "title": "Document Title",
                    "similarity_score": 0.85,
                    "chunk_index": 0
                },
                ...
            ]
        """
        # Generate query embedding
        query_embedding = await self.embedding_service.generate_embedding(query)
        query_blob = self.embedding_service.serialize_embedding(query_embedding)

        # Vector similarity search
        async with self.db._get_connection() as conn:
            if document_type:
                cursor = await conn.execute(
                    """
                    SELECT
                        de.document_id,
                        di.file_path,
                        di.title,
                        di.document_type,
                        de.chunk_index,
                        vss_distance(de.embedding, ?) as distance
                    FROM document_embeddings_data de
                    JOIN document_index di ON de.document_id = di.id
                    WHERE di.document_type = ?
                    ORDER BY distance ASC
                    LIMIT ?
                    """,
                    (query_blob, document_type, limit)
                )
            else:
                cursor = await conn.execute(
                    """
                    SELECT
                        de.document_id,
                        di.file_path,
                        di.title,
                        di.document_type,
                        de.chunk_index,
                        vss_distance(de.embedding, ?) as distance
                    FROM document_embeddings_data de
                    JOIN document_index di ON de.document_id = di.id
                    ORDER BY distance ASC
                    LIMIT ?
                    """,
                    (query_blob, limit)
                )

            rows = await cursor.fetchall()

            # Convert distance to similarity score (0-1 range)
            results = []
            for row in rows:
                results.append({
                    "document_id": row['document_id'],
                    "file_path": row['file_path'],
                    "title": row['title'],
                    "document_type": row['document_type'],
                    "chunk_index": row['chunk_index'],
                    "similarity_score": 1.0 / (1.0 + row['distance'])
                })

            return results

    async def hybrid_search(
        self,
        query: str,
        limit: int = 10,
        exact_weight: float = 0.3,
        semantic_weight: float = 0.7
    ) -> List[Dict[str, Any]]:
        """Combine exact keyword match and semantic similarity.

        Args:
            query: Search query
            limit: Maximum results
            exact_weight: Weight for exact match score (0-1)
            semantic_weight: Weight for semantic similarity (0-1)

        Returns:
            Re-ranked results combining both strategies
        """
        # Exact keyword search (using FTS5 or LIKE)
        exact_results = await self._exact_search(query, limit)

        # Semantic search
        semantic_results = await self.semantic_search(query, limit)

        # Combine and re-rank
        combined_scores = {}
        for result in exact_results:
            doc_id = result['document_id']
            combined_scores[doc_id] = exact_weight * result.get('match_score', 0.5)

        for result in semantic_results:
            doc_id = result['document_id']
            if doc_id in combined_scores:
                combined_scores[doc_id] += semantic_weight * result['similarity_score']
            else:
                combined_scores[doc_id] = semantic_weight * result['similarity_score']

        # Sort by combined score
        ranked_docs = sorted(
            combined_scores.items(),
            key=lambda x: x[1],
            reverse=True
        )[:limit]

        # Fetch full document details
        return await self._fetch_documents([doc_id for doc_id, _ in ranked_docs])
```

### 5. Background Sync Service

**File:** `src/abathur/infrastructure/document_sync_service.py`

```python
import asyncio
from pathlib import Path
import hashlib
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler

class DocumentSyncService:
    """Background service for syncing markdown files to document_index."""

    def __init__(
        self,
        db: Database,
        embedding_service: EmbeddingService,
        document_chunker: DocumentChunker,
        watch_directories: List[Path]
    ):
        self.db = db
        self.embedding_service = embedding_service
        self.document_chunker = document_chunker
        self.watch_directories = watch_directories

    async def sync_document(self, file_path: Path) -> None:
        """Sync single markdown document to index with embeddings.

        Args:
            file_path: Path to markdown file

        Process:
            1. Compute content hash
            2. Check if document exists and hash changed
            3. Extract title from markdown
            4. Chunk document text
            5. Generate embeddings for each chunk
            6. Store in document_index and document_embeddings_data
        """
        # Read file and compute hash
        content = file_path.read_text()
        content_hash = hashlib.sha256(content.encode()).hexdigest()

        # Check if already indexed
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT id, content_hash FROM document_index WHERE file_path = ?",
                (str(file_path),)
            )
            existing = await cursor.fetchone()

            if existing and existing['content_hash'] == content_hash:
                # No changes, skip
                return

            # Extract title (first # heading or filename)
            title = self._extract_title(content) or file_path.stem

            # Chunk document
            chunks = self.document_chunker.chunk_text(content)

            # Generate embeddings for chunks
            chunk_texts = [chunk['text'] for chunk in chunks]
            embeddings = await self.embedding_service.generate_batch_embeddings(chunk_texts)

            # Store in database
            async with conn.begin():
                if existing:
                    # Update existing document
                    doc_id = existing['id']
                    await conn.execute(
                        """
                        UPDATE document_index
                        SET content_hash = ?, chunk_count = ?, last_synced_at = CURRENT_TIMESTAMP,
                            sync_status = 'synced'
                        WHERE id = ?
                        """,
                        (content_hash, len(chunks), doc_id)
                    )

                    # Delete old embeddings
                    await conn.execute(
                        "DELETE FROM document_embeddings_data WHERE document_id = ?",
                        (doc_id,)
                    )
                else:
                    # Insert new document
                    cursor = await conn.execute(
                        """
                        INSERT INTO document_index (file_path, title, content_hash, chunk_count, sync_status)
                        VALUES (?, ?, ?, ?, 'synced')
                        """,
                        (str(file_path), title, content_hash, len(chunks))
                    )
                    doc_id = cursor.lastrowid

                # Insert embeddings
                for i, embedding in enumerate(embeddings):
                    embedding_blob = self.embedding_service.serialize_embedding(embedding)
                    await conn.execute(
                        """
                        INSERT INTO document_embeddings_data (document_id, chunk_index, embedding)
                        VALUES (?, ?, ?)
                        """,
                        (doc_id, i, embedding_blob)
                    )

    async def sync_all_documents(self) -> None:
        """Sync all markdown files in watch directories."""
        for directory in self.watch_directories:
            for md_file in directory.rglob("*.md"):
                await self.sync_document(md_file)

    def start_file_watcher(self) -> None:
        """Start file watcher for automatic re-indexing."""
        event_handler = MarkdownFileHandler(self)
        observer = Observer()

        for directory in self.watch_directories:
            observer.schedule(event_handler, str(directory), recursive=True)

        observer.start()
        # Run in background thread (or asyncio task)

class MarkdownFileHandler(FileSystemEventHandler):
    def __init__(self, sync_service: DocumentSyncService):
        self.sync_service = sync_service

    def on_modified(self, event):
        if event.src_path.endswith('.md'):
            asyncio.create_task(
                self.sync_service.sync_document(Path(event.src_path))
            )
```

### 6. Performance Validation Report

**File:** `docs/performance_validation_milestone3.md`

**Benchmarks:**
- Embedding generation latency: 50-100ms per document (Ollama)
- Semantic search latency: <500ms for 1000 documents
- Hybrid search latency: <600ms (exact + semantic combined)
- Background sync throughput: 10 documents/minute
- Vector similarity accuracy: >80% relevant results in top 10

**Index Tuning:**
- sqlite-vss uses HNSW (Hierarchical Navigable Small World) index
- Optimize M parameter (neighbors per layer): M=16 for 768-dim embeddings
- Optimize efConstruction (index build time): 200 for balanced speed/accuracy

---

## Acceptance Criteria

### Technical Validation

- [ ] sqlite-vss extension loads successfully
- [ ] Ollama server responds to embedding requests
- [ ] 768-dimensional embeddings generated correctly
- [ ] document_embeddings virtual table created
- [ ] All semantic search queries return results <500ms
- [ ] Background sync service indexes markdown files automatically
- [ ] Content hash detects file changes correctly

### Functional Validation

- [ ] Semantic search returns relevant results (>80% accuracy)
- [ ] Hybrid search combines exact + semantic scores
- [ ] Document chunking produces 512-token segments
- [ ] Embedding serialization/deserialization works correctly
- [ ] File watcher triggers re-indexing on file changes
- [ ] Re-ranking logic improves result quality

### Performance Validation

- [ ] Semantic search <500ms for 1000 documents
- [ ] Embedding generation <100ms per document
- [ ] Background sync processes 10+ docs/minute
- [ ] Vector index build time <10 seconds for 1000 docs
- [ ] No performance regression on existing queries

---

## Risks and Mitigation

### Risk 1: Ollama Server Availability

**Description:** Ollama server downtime breaks embedding generation
- **Probability:** Medium
- **Impact:** High
- **Mitigation:**
  - Implement retry logic with exponential backoff
  - Cache embeddings to avoid regeneration
  - Fallback to pre-computed embeddings if Ollama unavailable
- **Contingency:** Deploy Ollama as Docker container with auto-restart

### Risk 2: Embedding Quality Issues

**Description:** nomic-embed-text may produce poor quality embeddings for technical docs
- **Probability:** Low
- **Impact:** Medium
- **Mitigation:**
  - Test embedding quality on sample technical documents
  - Compare with alternative models (sentence-transformers)
  - Fine-tune chunking strategy (512 tokens may be suboptimal)
- **Contingency:** Switch to all-MiniLM-L6-v2 (384 dims) or BGE-large (1024 dims)

### Risk 3: sqlite-vss Performance Degradation

**Description:** Vector search may be slower than <500ms target for large datasets
- **Probability:** Medium
- **Impact:** Medium
- **Mitigation:**
  - Optimize HNSW index parameters (M, efConstruction)
  - Limit search scope with document_type filters
  - Implement result caching for frequent queries
- **Contingency:** Use approximate nearest neighbor search (reduce accuracy for speed)

### Risk 4: Background Sync Resource Usage

**Description:** Continuous file watching may consume excessive CPU/memory
- **Probability:** Low
- **Impact:** Low
- **Mitigation:**
  - Debounce file change events (wait 5 seconds before re-indexing)
  - Limit concurrent embedding generation (max 5 at a time)
  - Run sync in separate process with resource limits
- **Contingency:** Switch to scheduled sync (hourly) instead of real-time

---

## Go/No-Go Decision Criteria

### Prerequisites (Must be Complete)

1. ✅ Milestone 2 validated and signed off
2. ✅ sqlite-vss extension installed and functional
3. ✅ Ollama server deployed with nomic-embed-text model
4. ✅ Semantic search queries return results <500ms

### Validation Checks (Must Pass)

1. **Vector Search Functional:**
   - Embeddings generated successfully (768 dimensions)
   - Semantic search returns relevant results (>80% accuracy)
   - Hybrid search improves result quality over exact-match alone

2. **Performance Targets:**
   - Semantic search latency <500ms (99th percentile)
   - Embedding generation <100ms per document
   - Background sync processes 10+ docs/minute

3. **Operational Readiness:**
   - Background sync service runs without errors
   - File watcher detects changes and triggers re-indexing
   - Ollama server monitored with health checks

### Rollback Plan (If Go/No-Go Fails)

1. **Immediate Actions:**
   - Disable semantic search feature (fallback to exact-match only)
   - Stop background sync service
   - Unload sqlite-vss extension

2. **Rollback Procedure:**
   ```sql
   -- Drop vector search tables
   DROP TABLE IF EXISTS document_embeddings;
   DROP TABLE IF EXISTS document_embeddings_data;

   -- Mark documents as pending sync
   UPDATE document_index SET sync_status = 'pending';
   ```

3. **Recovery Actions:**
   - Investigate performance bottleneck (Ollama, sqlite-vss, network)
   - Optimize embedding generation (batch size, concurrent requests)
   - Re-deploy after fixes validated

---

## Post-Milestone Activities

### Documentation Updates

- [ ] Document semantic search API usage and examples
- [ ] Create embedding generation guide for developers
- [ ] Update troubleshooting guide with vector search issues
- [ ] Document hybrid search re-ranking algorithm

### Knowledge Transfer

- [ ] Demo semantic search capabilities to team
- [ ] Walkthrough sqlite-vss configuration and tuning
- [ ] Share Ollama deployment best practices

### Preparation for Milestone 4

- [ ] Review production deployment checklist
- [ ] Plan monitoring dashboards for semantic search
- [ ] Allocate resources for final validation and smoke tests

---

## Success Metrics

### Quantitative Metrics

| Metric | Target | Actual | Status |
|--------|--------|--------|--------|
| Semantic search latency | <500ms | ___ ms | ⏳ Pending |
| Embedding generation speed | <100ms/doc | ___ ms | ⏳ Pending |
| Search relevance (top 10) | >80% | ___ % | ⏳ Pending |
| Background sync throughput | 10+ docs/min | ___ | ⏳ Pending |
| Vector index build time | <10s (1000 docs) | ___ s | ⏳ Pending |

### Qualitative Metrics

- [ ] Search result quality: High (relevant results in top 10)
- [ ] Embedding model suitability: High (works well for technical docs)
- [ ] Background sync reliability: High (no missed file changes)
- [ ] Operational complexity: Low (minimal maintenance required)

---

## References

**Phase 2 Technical Specifications:**
- [SQLite VSS Integration](../phase2_tech_specs/sqlite-vss-integration.md) - Vector search setup guide
- [Implementation Guide](../phase2_tech_specs/implementation-guide.md) - Deployment procedures

**External Documentation:**
- [sqlite-vss GitHub](https://github.com/asg017/sqlite-vss) - Extension documentation
- [Ollama Documentation](https://ollama.ai/docs) - Embedding model setup
- [nomic-embed-text Model Card](https://huggingface.co/nomic-ai/nomic-embed-text-v1.5) - Model specifications

---

**Milestone Version:** 1.0
**Author:** implementation-planner
**Date:** 2025-10-10
**Status:** Ready for Execution
**Previous Milestone:** [Milestone 2: Memory System](./milestone-2-memory-system.md)
**Next Milestone:** [Milestone 4: Production Deployment](./milestone-4-production-deployment.md)
