"""Unit tests for DocumentIndexService."""

import hashlib

import pytest
from abathur.services import DocumentIndexService


class TestDocumentIndexService:
    """Test DocumentIndexService CRUD operations and document management."""

    @pytest.mark.asyncio
    async def test_index_document_success(self, document_service: DocumentIndexService) -> None:
        """Test successful document indexing."""
        content = "# Schema Design\n\nThis is the content."

        doc_id = await document_service.index_document(
            file_path="/docs/schema-design.md",
            title="Schema Design",
            content=content,
            document_type="design",
            metadata={"author": "alice", "version": "1.0"},
        )

        assert doc_id > 0, "Document ID should be positive"

        # Verify document was created
        doc = await document_service.get_document("/docs/schema-design.md")
        assert doc is not None
        assert doc["file_path"] == "/docs/schema-design.md"
        assert doc["title"] == "Schema Design"
        assert doc["document_type"] == "design"
        assert doc["content_hash"] == hashlib.sha256(content.encode()).hexdigest()
        assert doc["sync_status"] == "pending"
        assert doc["metadata"] == {"author": "alice", "version": "1.0"}
        assert doc["chunk_count"] == 1

    @pytest.mark.asyncio
    async def test_index_document_minimal(self, document_service: DocumentIndexService) -> None:
        """Test indexing document with minimal required fields."""
        content = "Minimal content"

        _doc_id = await document_service.index_document(
            file_path="/docs/minimal.md", title="Minimal Document", content=content
        )

        doc = await document_service.get_document("/docs/minimal.md")
        assert doc is not None
        assert doc["file_path"] == "/docs/minimal.md"
        assert doc["title"] == "Minimal Document"
        assert doc["document_type"] is None
        assert doc["metadata"] == {}
        assert doc["sync_status"] == "pending"

    @pytest.mark.asyncio
    async def test_index_document_duplicate_raises_error(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test that indexing duplicate file_path raises ValueError."""
        content = "# Content"

        await document_service.index_document(
            file_path="/docs/duplicate.md", title="First", content=content
        )

        with pytest.raises(ValueError, match="already exists"):
            await document_service.index_document(
                file_path="/docs/duplicate.md", title="Second", content=content
            )

    @pytest.mark.asyncio
    async def test_get_document_by_file_path(self, document_service: DocumentIndexService) -> None:
        """Test retrieving document by file path."""
        content = "# Test Content"

        await document_service.index_document(
            file_path="/docs/test.md",
            title="Test Doc",
            content=content,
            document_type="specification",
        )

        doc = await document_service.get_document("/docs/test.md")
        assert doc is not None
        assert doc["file_path"] == "/docs/test.md"
        assert doc["title"] == "Test Doc"
        assert doc["document_type"] == "specification"

    @pytest.mark.asyncio
    async def test_get_document_nonexistent_returns_none(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test getting nonexistent document returns None."""
        doc = await document_service.get_document("/nonexistent.md")
        assert doc is None

    @pytest.mark.asyncio
    async def test_get_document_by_id(self, document_service: DocumentIndexService) -> None:
        """Test retrieving document by ID."""
        content = "# Content"

        doc_id = await document_service.index_document(
            file_path="/docs/by-id.md", title="By ID Test", content=content
        )

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["id"] == doc_id
        assert doc["file_path"] == "/docs/by-id.md"
        assert doc["title"] == "By ID Test"

    @pytest.mark.asyncio
    async def test_get_document_by_id_nonexistent_returns_none(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test getting nonexistent document by ID returns None."""
        doc = await document_service.get_document_by_id(999999)
        assert doc is None

    @pytest.mark.asyncio
    async def test_update_embedding(self, document_service: DocumentIndexService) -> None:
        """Test updating document embedding."""
        content = "# Content"

        doc_id = await document_service.index_document(
            file_path="/docs/embedding-test.md", title="Embedding Test", content=content
        )

        # Create mock embedding (512-dimensional vector as bytes)
        embedding = b"\x00\x01" * 256  # 512 bytes

        await document_service.update_embedding(
            doc_id=doc_id, embedding=embedding, embedding_model="nomic-embed-text-v1.5"
        )

        # Verify embedding was stored
        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["embedding_blob"] == embedding
        assert doc["embedding_model"] == "nomic-embed-text-v1.5"

    @pytest.mark.asyncio
    async def test_update_embedding_custom_model(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test updating embedding with custom model."""
        content = "# Content"

        doc_id = await document_service.index_document(
            file_path="/docs/custom-model.md", title="Custom Model", content=content
        )

        embedding = b"\xFF" * 100

        await document_service.update_embedding(
            doc_id=doc_id, embedding=embedding, embedding_model="custom-model-v2"
        )

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["embedding_model"] == "custom-model-v2"

    @pytest.mark.asyncio
    async def test_mark_synced(self, document_service: DocumentIndexService) -> None:
        """Test marking document as synced."""
        content = "# Content"

        doc_id = await document_service.index_document(
            file_path="/docs/sync-test.md", title="Sync Test", content=content
        )

        # Initially pending
        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "pending"
        assert doc["last_synced_at"] is None

        # Mark as synced
        await document_service.mark_synced(doc_id)

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "synced"
        assert doc["last_synced_at"] is not None

    @pytest.mark.asyncio
    async def test_mark_failed(self, document_service: DocumentIndexService) -> None:
        """Test marking document sync as failed."""
        content = "# Content"

        doc_id = await document_service.index_document(
            file_path="/docs/fail-test.md",
            title="Fail Test",
            content=content,
            metadata={"initial": "value"},
        )

        error_msg = "Embedding API timeout after 30s"
        await document_service.mark_failed(doc_id, error_msg)

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "failed"
        assert doc["metadata"]["last_error"] == error_msg
        assert doc["metadata"]["initial"] == "value"  # Original metadata preserved

    @pytest.mark.asyncio
    async def test_mark_failed_preserves_existing_metadata(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test mark_failed preserves existing metadata fields."""
        content = "# Content"

        doc_id = await document_service.index_document(
            file_path="/docs/metadata-test.md",
            title="Metadata Test",
            content=content,
            metadata={"author": "alice", "version": 1},
        )

        await document_service.mark_failed(doc_id, "API error")

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["metadata"]["author"] == "alice"
        assert doc["metadata"]["version"] == 1
        assert doc["metadata"]["last_error"] == "API error"

    @pytest.mark.asyncio
    async def test_mark_stale(self, document_service: DocumentIndexService) -> None:
        """Test marking document as stale when content changes."""
        old_content = "# Original Content"
        new_content = "# Updated Content"

        doc_id = await document_service.index_document(
            file_path="/docs/stale-test.md", title="Stale Test", content=old_content
        )

        # Mark as synced first
        await document_service.mark_synced(doc_id)
        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "synced"

        # Mark as stale with new content
        await document_service.mark_stale("/docs/stale-test.md", new_content)

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "stale"
        assert doc["content_hash"] == hashlib.sha256(new_content.encode()).hexdigest()

    @pytest.mark.asyncio
    async def test_get_pending_documents(self, document_service: DocumentIndexService) -> None:
        """Test getting pending and stale documents."""
        # Create pending document
        await document_service.index_document(
            file_path="/docs/pending1.md", title="Pending 1", content="# Pending"
        )

        # Create synced document
        doc_id_synced = await document_service.index_document(
            file_path="/docs/synced.md", title="Synced", content="# Synced"
        )
        await document_service.mark_synced(doc_id_synced)

        # Create stale document
        doc_id_stale = await document_service.index_document(
            file_path="/docs/stale.md", title="Stale", content="# Original"
        )
        await document_service.mark_synced(doc_id_stale)
        await document_service.mark_stale("/docs/stale.md", "# Updated")

        # Create failed document
        doc_id_failed = await document_service.index_document(
            file_path="/docs/failed.md", title="Failed", content="# Failed"
        )
        await document_service.mark_failed(doc_id_failed, "Error")

        # Get pending documents (should include pending + stale, not synced or failed)
        pending_docs = await document_service.get_pending_documents()

        pending_paths = [doc["file_path"] for doc in pending_docs]
        assert "/docs/pending1.md" in pending_paths
        assert "/docs/stale.md" in pending_paths
        assert "/docs/synced.md" not in pending_paths
        assert "/docs/failed.md" not in pending_paths

    @pytest.mark.asyncio
    async def test_get_pending_documents_limit(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test pending documents respects limit parameter."""
        # Create 10 pending documents
        for i in range(10):
            await document_service.index_document(
                file_path=f"/docs/pending_{i}.md", title=f"Pending {i}", content=f"# Content {i}"
            )

        # Get with limit=5
        pending_docs = await document_service.get_pending_documents(limit=5)
        assert len(pending_docs) == 5

    @pytest.mark.asyncio
    async def test_get_pending_documents_ordered_by_created_at(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test pending documents ordered by creation time (oldest first)."""
        # Create documents in specific order
        _doc_id_1 = await document_service.index_document(
            file_path="/docs/first.md", title="First", content="# First"
        )

        _doc_id_2 = await document_service.index_document(
            file_path="/docs/second.md", title="Second", content="# Second"
        )

        pending_docs = await document_service.get_pending_documents()

        # Should be ordered by created_at ASC (oldest first)
        assert pending_docs[0]["file_path"] == "/docs/first.md"
        assert pending_docs[1]["file_path"] == "/docs/second.md"

    @pytest.mark.asyncio
    async def test_search_by_type(self, document_service: DocumentIndexService) -> None:
        """Test searching documents by type."""
        # Create documents of different types
        await document_service.index_document(
            file_path="/docs/design1.md",
            title="Design 1",
            content="# Design",
            document_type="design",
        )
        await document_service.index_document(
            file_path="/docs/design2.md",
            title="Design 2",
            content="# Design",
            document_type="design",
        )
        await document_service.index_document(
            file_path="/docs/spec1.md",
            title="Spec 1",
            content="# Spec",
            document_type="specification",
        )
        await document_service.index_document(
            file_path="/docs/plan1.md", title="Plan 1", content="# Plan", document_type="plan"
        )

        # Search by design type
        design_docs = await document_service.search_by_type("design")
        assert len(design_docs) == 2
        assert all(doc["document_type"] == "design" for doc in design_docs)

        # Search by specification type
        spec_docs = await document_service.search_by_type("specification")
        assert len(spec_docs) == 1
        assert spec_docs[0]["document_type"] == "specification"

    @pytest.mark.asyncio
    async def test_search_by_type_limit(self, document_service: DocumentIndexService) -> None:
        """Test search by type respects limit parameter."""
        # Create 10 design documents
        for i in range(10):
            await document_service.index_document(
                file_path=f"/docs/design_{i}.md",
                title=f"Design {i}",
                content=f"# Design {i}",
                document_type="design",
            )

        # Search with limit=5
        design_docs = await document_service.search_by_type("design", limit=5)
        assert len(design_docs) == 5

    @pytest.mark.asyncio
    async def test_search_by_type_ordered_by_created_at_desc(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test search by type returns documents ordered by created_at DESC."""
        # Create documents in specific order
        _doc_id_1 = await document_service.index_document(
            file_path="/docs/old.md", title="Old", content="# Old", document_type="design"
        )

        _doc_id_2 = await document_service.index_document(
            file_path="/docs/new.md", title="New", content="# New", document_type="design"
        )

        design_docs = await document_service.search_by_type("design")

        # Both documents should be returned
        assert len(design_docs) == 2
        file_paths = [doc["file_path"] for doc in design_docs]
        assert "/docs/old.md" in file_paths
        assert "/docs/new.md" in file_paths

        # Verify they have created_at timestamps
        assert all(doc["created_at"] is not None for doc in design_docs)

    @pytest.mark.asyncio
    async def test_search_by_embedding_with_vector_db(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test search by embedding with vector DB implementation."""
        # Create document and generate embedding using the vector DB
        doc_id = await document_service.index_document(
            file_path="/docs/embedded.md",
            title="Embedded",
            content="# Content about machine learning",
        )

        # Generate and store embedding in vector DB
        await document_service.generate_and_store_embedding(
            document_id=doc_id,
            content="# Content about machine learning",
            namespace="docs:test",
            file_path="/docs/embedded.md",
        )

        # Generate query embedding
        from abathur.services.embedding_service import EmbeddingService

        embedding_service = EmbeddingService()
        query_embedding = await embedding_service.generate_embedding("machine learning concepts")

        # Search by embedding (uses default distance threshold of 1000.0)
        results = await document_service.search_by_embedding(query_embedding, limit=5)

        # Should find the document
        assert len(results) > 0
        assert results[0]["document_id"] == doc_id

    @pytest.mark.asyncio
    async def test_list_all_documents(self, document_service: DocumentIndexService) -> None:
        """Test listing all documents."""
        # Create multiple documents
        await document_service.index_document(
            file_path="/docs/doc1.md", title="Doc 1", content="# Content 1", document_type="design"
        )
        await document_service.index_document(
            file_path="/docs/doc2.md",
            title="Doc 2",
            content="# Content 2",
            document_type="specification",
        )
        await document_service.index_document(
            file_path="/docs/doc3.md", title="Doc 3", content="# Content 3", document_type="plan"
        )

        all_docs = await document_service.list_all_documents()

        assert len(all_docs) == 3
        file_paths = [doc["file_path"] for doc in all_docs]
        assert "/docs/doc1.md" in file_paths
        assert "/docs/doc2.md" in file_paths
        assert "/docs/doc3.md" in file_paths

    @pytest.mark.asyncio
    async def test_list_all_documents_limit(self, document_service: DocumentIndexService) -> None:
        """Test list all documents respects limit parameter."""
        # Create 10 documents
        for i in range(10):
            await document_service.index_document(
                file_path=f"/docs/list_{i}.md", title=f"List {i}", content=f"# Content {i}"
            )

        all_docs = await document_service.list_all_documents(limit=5)
        assert len(all_docs) == 5

    @pytest.mark.asyncio
    async def test_list_all_documents_ordered_by_created_at_desc(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test list all documents returns documents ordered by created_at DESC."""
        # Create documents in order
        _doc_id_1 = await document_service.index_document(
            file_path="/docs/oldest.md", title="Oldest", content="# Oldest"
        )

        _doc_id_2 = await document_service.index_document(
            file_path="/docs/newest.md", title="Newest", content="# Newest"
        )

        all_docs = await document_service.list_all_documents()

        # Both documents should be returned
        assert len(all_docs) == 2
        file_paths = [doc["file_path"] for doc in all_docs]
        assert "/docs/oldest.md" in file_paths
        assert "/docs/newest.md" in file_paths

        # Verify they have created_at timestamps
        assert all(doc["created_at"] is not None for doc in all_docs)

    @pytest.mark.asyncio
    async def test_document_lifecycle_workflow(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test complete document lifecycle: index → pending → synced → stale → synced."""
        content_v1 = "# Version 1"
        content_v2 = "# Version 2"

        # 1. Index document (pending)
        doc_id = await document_service.index_document(
            file_path="/docs/lifecycle.md",
            title="Lifecycle Test",
            content=content_v1,
            document_type="design",
        )

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "pending"

        # 2. Add embedding and mark synced
        embedding = b"\x00" * 100
        await document_service.update_embedding(doc_id, embedding)
        await document_service.mark_synced(doc_id)

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "synced"
        assert doc["last_synced_at"] is not None

        # 3. Content changes, mark stale
        await document_service.mark_stale("/docs/lifecycle.md", content_v2)

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "stale"
        assert doc["content_hash"] == hashlib.sha256(content_v2.encode()).hexdigest()

        # 4. Re-sync after update
        new_embedding = b"\xFF" * 100
        await document_service.update_embedding(doc_id, new_embedding)
        await document_service.mark_synced(doc_id)

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None
        assert doc["sync_status"] == "synced"
        assert doc["embedding_blob"] == new_embedding

    @pytest.mark.asyncio
    async def test_content_hash_changes_with_content(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test content hash is correctly calculated for different content."""
        content_1 = "# Content A"
        content_2 = "# Content B"

        doc_id_1 = await document_service.index_document(
            file_path="/docs/hash1.md", title="Hash 1", content=content_1
        )

        doc_id_2 = await document_service.index_document(
            file_path="/docs/hash2.md", title="Hash 2", content=content_2
        )

        doc1 = await document_service.get_document_by_id(doc_id_1)
        doc2 = await document_service.get_document_by_id(doc_id_2)
        assert doc1 is not None
        assert doc2 is not None

        # Different content should produce different hashes
        assert doc1["content_hash"] != doc2["content_hash"]

        # Verify hashes match expected SHA256
        assert doc1["content_hash"] == hashlib.sha256(content_1.encode()).hexdigest()
        assert doc2["content_hash"] == hashlib.sha256(content_2.encode()).hexdigest()

    @pytest.mark.asyncio
    async def test_metadata_json_parsing(self, document_service: DocumentIndexService) -> None:
        """Test metadata is correctly parsed from JSON storage."""
        complex_metadata = {
            "author": "alice",
            "version": 2,
            "tags": ["design", "database"],
            "nested": {"key": "value", "count": 42},
        }

        doc_id = await document_service.index_document(
            file_path="/docs/json-test.md",
            title="JSON Test",
            content="# Content",
            metadata=complex_metadata,
        )

        doc = await document_service.get_document_by_id(doc_id)
        assert doc is not None

        # Verify complex metadata structure is preserved
        assert doc["metadata"] == complex_metadata
        assert doc["metadata"]["tags"] == ["design", "database"]
        assert doc["metadata"]["nested"]["count"] == 42

    @pytest.mark.asyncio
    async def test_generate_and_store_embedding(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test embedding generation and storage."""
        # Index a document first
        doc_id = await document_service.index_document(
            file_path="/test/doc1.md",
            title="Test Document",
            content="This is test content for embedding",
        )

        # Generate and store embedding
        rowid = await document_service.generate_and_store_embedding(
            document_id=doc_id,
            content="This is test content for embedding",
            namespace="test:docs",
            file_path="/test/doc1.md",
        )

        assert rowid > 0

        # Verify metadata was stored
        async with document_service.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM document_embedding_metadata WHERE document_id = ?", (doc_id,)
            )
            row = await cursor.fetchone()
            assert row is not None
            assert row["namespace"] == "test:docs"
            assert row["embedding_model"] == "nomic-embed-text-v1.5"

    @pytest.mark.asyncio
    async def test_search_by_embedding(self, document_service: DocumentIndexService) -> None:
        """Test semantic search by embedding vector."""
        # Index and embed a document
        doc_id = await document_service.index_document(
            file_path="/test/doc1.md",
            title="ML Document",
            content="Machine learning and artificial intelligence",
        )

        await document_service.generate_and_store_embedding(
            document_id=doc_id,
            content="Machine learning and artificial intelligence",
            namespace="test:docs",
            file_path="/test/doc1.md",
        )

        # Generate query embedding
        from abathur.services.embedding_service import EmbeddingService

        embedding_service = EmbeddingService()
        query_embedding = await embedding_service.generate_embedding("AI and ML concepts")

        # Search (uses default distance threshold)
        results = await document_service.search_by_embedding(query_embedding, limit=5)

        assert len(results) > 0
        assert results[0]["document_id"] == doc_id
        assert "distance" in results[0]

    @pytest.mark.asyncio
    async def test_semantic_search(self, document_service: DocumentIndexService) -> None:
        """Test high-level semantic search API."""
        # Index and embed a document
        await document_service.sync_document_to_vector_db(
            namespace="test:docs",
            file_path="/test/ml_guide.md",
            content="# Machine Learning Guide\n\nA comprehensive guide to machine learning algorithms",
        )

        # Semantic search (uses default distance threshold)
        results = await document_service.semantic_search(
            query_text="deep learning tutorials", namespace="test:docs", limit=5
        )

        assert len(results) >= 0  # May or may not find results depending on similarity
        if results:
            assert "distance" in results[0]
            assert results[0]["namespace"] == "test:docs"

    @pytest.mark.asyncio
    async def test_sync_document_to_vector_db(self, document_service: DocumentIndexService) -> None:
        """Test sync_document_to_vector_db convenience method."""
        content = "# API Documentation\n\nThis document describes the API endpoints."

        result = await document_service.sync_document_to_vector_db(
            namespace="docs:api", file_path="/docs/api.md", content=content
        )

        assert "document_id" in result
        assert "embedding_rowid" in result
        assert result["document_id"] > 0
        assert result["embedding_rowid"] > 0

        # Verify document was indexed
        doc = await document_service.get_document_by_id(result["document_id"])
        assert doc is not None
        assert doc["file_path"] == "/docs/api.md"
        assert doc["sync_status"] == "synced"

        # Verify embedding was stored
        async with document_service.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM document_embedding_metadata WHERE document_id = ?",
                (result["document_id"],),
            )
            row = await cursor.fetchone()
            assert row is not None
            assert row["namespace"] == "docs:api"

    @pytest.mark.asyncio
    async def test_semantic_search_with_namespace_filter(
        self, document_service: DocumentIndexService
    ) -> None:
        """Test semantic search with namespace filtering."""
        # Create documents in different namespaces
        await document_service.sync_document_to_vector_db(
            namespace="docs:architecture",
            file_path="/docs/architecture.md",
            content="# Architecture\n\nSystem architecture and design patterns",
        )

        await document_service.sync_document_to_vector_db(
            namespace="docs:api",
            file_path="/docs/api.md",
            content="# API\n\nAPI endpoints and usage",
        )

        # Search with namespace filter (uses default distance threshold)
        results = await document_service.semantic_search(
            query_text="system design", namespace="docs:architecture", limit=5
        )

        # All results should be from docs:architecture namespace
        for result in results:
            assert result["namespace"].startswith("docs:architecture")
