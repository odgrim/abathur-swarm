# SQLite-VSS Integration Guide - Vector Semantic Search

## Overview

This guide provides step-by-step instructions for integrating vector semantic search using sqlite-vss extension with nomic-embed-text-v1.5 embeddings generated via Ollama.

**Phase:** Post-schema deployment (Phase 2+)

**Performance Target:** <500ms for semantic search across 1,000+ documents

**Model:** nomic-embed-text-v1.5 (768 dimensions, 8K context, Apache 2.0 license)

---

## 1. Prerequisites

### 1.1 Install Ollama

**macOS:**
```bash
brew install ollama

# Start Ollama service
ollama serve
```

**Linux:**
```bash
curl -fsSL https://ollama.ai/install.sh | sh

# Start service
sudo systemctl start ollama
```

**Verify Installation:**
```bash
ollama --version
# Expected: ollama version 0.1.x
```

### 1.2 Pull nomic-embed-text-v1.5 Model

```bash
ollama pull nomic-embed-text:latest

# Verify model
ollama list
# Should show: nomic-embed-text:latest
```

**Model Specifications:**
- Dimensions: 768
- Context Length: 8192 tokens
- License: Apache 2.0
- Size: ~274 MB

---

## 2. Install sqlite-vss Extension

### 2.1 macOS Installation

**Option A: Build from Source**
```bash
# Clone repository
git clone https://github.com/asg017/sqlite-vss.git
cd sqlite-vss

# Build extension
make loadable

# Copy to extensions directory
mkdir -p ~/.sqlite-extensions
cp dist/vss0.dylib ~/.sqlite-extensions/
```

**Option B: Download Pre-built Binary**
```bash
# Download from releases
curl -L https://github.com/asg017/sqlite-vss/releases/latest/download/vss0-darwin-x86_64.dylib \
  -o ~/.sqlite-extensions/vss0.dylib
```

### 2.2 Linux Installation

```bash
# Download pre-built extension
curl -L https://github.com/asg017/sqlite-vss/releases/latest/download/vss0-linux-x86_64.so \
  -o ~/.sqlite-extensions/vss0.so
```

### 2.3 Verify Installation

```bash
sqlite3
.load ~/.sqlite-extensions/vss0
.quit
# No errors = successful installation
```

---

## 3. Python Integration

### 3.1 Install Dependencies

```bash
pip install ollama aiosqlite numpy
```

### 3.2 Embedding Generation Service

```python
import ollama
import numpy as np
from typing import List, Dict, Any

class EmbeddingService:
    """Generate embeddings using Ollama + nomic-embed-text-v1.5."""

    def __init__(self, model: str = "nomic-embed-text:latest") -> None:
        self.model = model

    async def generate_embedding(self, text: str) -> List[float]:
        """Generate embedding for text.

        Args:
            text: Input text (max 8192 tokens)

        Returns:
            768-dimensional embedding vector

        Example:
            >>> service = EmbeddingService()
            >>> embedding = await service.generate_embedding("Memory architecture design")
            >>> len(embedding)  # 768
        """
        response = ollama.embed(
            model=self.model,
            input=text
        )
        return response['embeddings'][0]

    async def batch_generate_embeddings(
        self,
        texts: List[str]
    ) -> List[List[float]]:
        """Generate embeddings for multiple texts.

        Args:
            texts: List of input texts

        Returns:
            List of embedding vectors
        """
        embeddings = []
        for text in texts:
            embedding = await self.generate_embedding(text)
            embeddings.append(embedding)
        return embeddings

    @staticmethod
    def serialize_embedding(embedding: List[float]) -> bytes:
        """Convert embedding to binary format for BLOB storage.

        Args:
            embedding: 768-dimensional vector

        Returns:
            Binary representation (768 * 4 = 3072 bytes)
        """
        array = np.array(embedding, dtype=np.float32)
        return array.tobytes()

    @staticmethod
    def deserialize_embedding(blob: bytes) -> List[float]:
        """Convert binary BLOB back to embedding vector.

        Args:
            blob: Binary embedding data

        Returns:
            768-dimensional vector
        """
        array = np.frombuffer(blob, dtype=np.float32)
        return array.tolist()
```

### 3.3 Document Sync Service

```python
import hashlib
import aiosqlite
from pathlib import Path

class DocumentSyncService:
    """Sync markdown files to document_index with embeddings."""

    def __init__(self, db: "Database", embedding_service: EmbeddingService) -> None:
        self.db = db
        self.embedding_service = embedding_service

    async def sync_document(self, file_path: Path) -> int:
        """Index or update document with embeddings.

        Args:
            file_path: Path to markdown file

        Returns:
            Document index ID

        Example:
            >>> service = DocumentSyncService(db, embedding_service)
            >>> doc_id = await service.sync_document(Path("design_docs/phase1_design/memory-architecture.md"))
        """
        # Read file
        content = file_path.read_text(encoding='utf-8')

        # Calculate content hash
        content_hash = hashlib.sha256(content.encode()).hexdigest()

        # Extract title (first # heading)
        title = self._extract_title(content)

        # Check if document exists
        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT id, content_hash FROM document_index WHERE file_path = ?",
                (str(file_path),)
            )
            existing = await cursor.fetchone()

            # If exists and hash unchanged, skip
            if existing and existing['content_hash'] == content_hash:
                return existing['id']

            # Generate embedding
            embedding_vector = await self.embedding_service.generate_embedding(content[:8000])  # Truncate to context limit
            embedding_blob = EmbeddingService.serialize_embedding(embedding_vector)

            # Determine document type
            document_type = self._determine_type(file_path)

            # Metadata
            metadata = {
                "author": "auto-sync",
                "word_count": len(content.split()),
                "chunk_count": 1
            }

            if existing:
                # Update existing document
                async with conn.begin():
                    await conn.execute(
                        """
                        UPDATE document_index
                        SET content_hash = ?, embedding_blob = ?, embedding_model = 'nomic-embed-text-v1.5',
                            sync_status = 'synced', last_synced_at = CURRENT_TIMESTAMP,
                            metadata = ?, updated_at = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (content_hash, embedding_blob, json.dumps(metadata), existing['id'])
                    )
                return existing['id']
            else:
                # Insert new document
                async with conn.begin():
                    cursor = await conn.execute(
                        """
                        INSERT INTO document_index (
                            file_path, title, document_type, content_hash,
                            embedding_blob, embedding_model, metadata, sync_status, last_synced_at
                        )
                        VALUES (?, ?, ?, ?, ?, 'nomic-embed-text-v1.5', ?, 'synced', CURRENT_TIMESTAMP)
                        """,
                        (str(file_path), title, document_type, content_hash, embedding_blob, json.dumps(metadata))
                    )
                    return cursor.lastrowid

    def _extract_title(self, content: str) -> str:
        """Extract title from first # heading."""
        for line in content.split('\n'):
            if line.startswith('# '):
                return line[2:].strip()
        return "Untitled"

    def _determine_type(self, file_path: Path) -> str:
        """Determine document type from path."""
        path_str = str(file_path)
        if 'phase1_design' in path_str:
            return 'design'
        elif 'phase2_tech_specs' in path_str:
            return 'specification'
        elif 'plan' in path_str.lower():
            return 'plan'
        else:
            return 'document'
```

---

## 4. Vector Similarity Search

### 4.1 Load sqlite-vss Extension

```python
async def enable_vss(conn: aiosqlite.Connection) -> None:
    """Load sqlite-vss extension."""
    await conn.enable_load_extension(True)
    await conn.load_extension("~/.sqlite-extensions/vss0")
    await conn.enable_load_extension(False)
```

### 4.2 Semantic Search Query

```python
class SemanticSearchService:
    """Perform vector similarity search on documents."""

    def __init__(self, db: "Database", embedding_service: EmbeddingService) -> None:
        self.db = db
        self.embedding_service = embedding_service

    async def search(
        self,
        query: str,
        limit: int = 10,
        threshold: float = 0.7
    ) -> List[Dict[str, Any]]:
        """Search documents by semantic similarity.

        Args:
            query: Search query text
            limit: Maximum results to return
            threshold: Minimum similarity score (0.0 - 1.0)

        Returns:
            List of documents with similarity scores

        Example:
            >>> service = SemanticSearchService(db, embedding_service)
            >>> results = await service.search("memory architecture design", limit=5)
            >>> for doc in results:
            ...     print(f"{doc['title']}: {doc['similarity']:.3f}")
        """
        # Generate query embedding
        query_embedding = await self.embedding_service.generate_embedding(query)
        query_blob = EmbeddingService.serialize_embedding(query_embedding)

        # Perform similarity search
        async with self.db._get_connection() as conn:
            await enable_vss(conn)

            cursor = await conn.execute(
                """
                SELECT
                    id,
                    file_path,
                    title,
                    document_type,
                    metadata,
                    vss_cosine_similarity(embedding_blob, ?) as similarity
                FROM document_index
                WHERE sync_status = 'synced'
                  AND vss_cosine_similarity(embedding_blob, ?) >= ?
                ORDER BY similarity DESC
                LIMIT ?
                """,
                (query_blob, query_blob, threshold, limit)
            )

            results = []
            async for row in cursor:
                doc = dict(row)
                doc['metadata'] = json.loads(doc.get('metadata', '{}'))
                results.append(doc)

            return results

    async def hybrid_search(
        self,
        query: str,
        exact_match_keywords: List[str],
        limit: int = 10
    ) -> List[Dict[str, Any]]:
        """Hybrid search: exact keyword match + semantic similarity.

        Args:
            query: Semantic search query
            exact_match_keywords: Keywords for exact title/metadata match
            limit: Maximum results

        Returns:
            Combined results (exact matches ranked higher)
        """
        # Exact match query
        async with self.db._get_connection() as conn:
            keyword_pattern = '%' + '%'.join(exact_match_keywords) + '%'
            cursor = await conn.execute(
                """
                SELECT *, 1.0 as similarity
                FROM document_index
                WHERE title LIKE ? OR metadata LIKE ?
                LIMIT ?
                """,
                (keyword_pattern, keyword_pattern, limit // 2)
            )
            exact_results = [dict(row) async for row in cursor]

        # Semantic search
        semantic_results = await self.search(query, limit=limit // 2)

        # Combine and deduplicate
        seen_ids = set()
        combined = []

        for result in exact_results + semantic_results:
            if result['id'] not in seen_ids:
                seen_ids.add(result['id'])
                combined.append(result)

        return combined[:limit]
```

---

## 5. Background Sync Service

### 5.1 File Watcher Implementation

```python
import asyncio
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler

class MarkdownFileWatcher(FileSystemEventHandler):
    """Watch design_docs/ for markdown file changes."""

    def __init__(self, sync_service: DocumentSyncService) -> None:
        self.sync_service = sync_service
        self.queue = asyncio.Queue()

    def on_modified(self, event):
        """Handle file modification events."""
        if event.src_path.endswith('.md'):
            asyncio.create_task(
                self.queue.put(Path(event.src_path))
            )

    async def process_queue(self):
        """Process queued file sync operations."""
        while True:
            file_path = await self.queue.get()
            try:
                await self.sync_service.sync_document(file_path)
                print(f"Synced: {file_path}")
            except Exception as e:
                print(f"Error syncing {file_path}: {e}")

async def start_file_watcher(sync_service: DocumentSyncService, watch_dir: Path):
    """Start background file watcher."""
    handler = MarkdownFileWatcher(sync_service)
    observer = Observer()
    observer.schedule(handler, str(watch_dir), recursive=True)
    observer.start()

    # Process queue
    await handler.process_queue()
```

---

## 6. Performance Optimization

### 6.1 Batch Embedding Generation

```python
async def batch_sync_documents(
    sync_service: DocumentSyncService,
    file_paths: List[Path],
    batch_size: int = 10
) -> None:
    """Sync multiple documents in batches."""
    for i in range(0, len(file_paths), batch_size):
        batch = file_paths[i:i+batch_size]
        tasks = [sync_service.sync_document(fp) for fp in batch]
        await asyncio.gather(*tasks)
        print(f"Synced batch {i//batch_size + 1}/{(len(file_paths)-1)//batch_size + 1}")
```

### 6.2 Caching Embeddings

```python
from functools import lru_cache

class CachedEmbeddingService(EmbeddingService):
    """Embedding service with LRU cache."""

    @lru_cache(maxsize=1000)
    def _cached_embed(self, text_hash: str, text: str) -> List[float]:
        """Cache embeddings by text hash."""
        response = ollama.embed(model=self.model, input=text)
        return response['embeddings'][0]

    async def generate_embedding(self, text: str) -> List[float]:
        """Generate or retrieve cached embedding."""
        text_hash = hashlib.sha256(text.encode()).hexdigest()
        return self._cached_embed(text_hash, text)
```

---

## 7. Deployment Checklist

- [ ] Ollama installed and service running
- [ ] nomic-embed-text:latest model pulled
- [ ] sqlite-vss extension compiled/downloaded
- [ ] Extension loaded successfully in SQLite
- [ ] Python dependencies installed (ollama, numpy)
- [ ] Initial document sync completed
- [ ] File watcher service configured (optional)
- [ ] Semantic search tested with sample queries
- [ ] Performance benchmarks validated (<500ms)

---

**Document Version:** 1.0
**Author:** technical-specifications-writer
**Date:** 2025-10-10
**Related Files:** `implementation-guide.md`, `api-specifications.md`
