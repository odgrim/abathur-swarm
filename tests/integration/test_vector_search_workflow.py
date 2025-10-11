"""Integration tests for vector search end-to-end workflows.

Tests complete workflows including:
- Document indexing + embedding generation + semantic search
- Namespace filtering
- Error handling with Ollama service
- Edge cases (empty content, large documents, special characters)
- Concurrent operations
- Data integrity validation
"""

import asyncio
from typing import Any

import httpx
import pytest
from abathur.infrastructure.database import Database
from abathur.services import DocumentIndexService
from abathur.services.embedding_service import EmbeddingService


class TestVectorSearchWorkflow:
    """Integration tests for end-to-end vector search workflows."""

    @pytest.mark.asyncio
    async def test_complete_vector_search_workflow(self, memory_db: Database) -> None:
        """Test full workflow: index document → generate embedding → semantic search."""
        service = DocumentIndexService(memory_db)

        # Step 1: Index multiple documents
        doc1 = await service.sync_document_to_vector_db(
            namespace="docs:ai",
            file_path="/docs/ml_guide.md",
            content="Machine learning uses algorithms to learn from data and make predictions",
        )

        doc2 = await service.sync_document_to_vector_db(
            namespace="docs:ai",
            file_path="/docs/neural_networks.md",
            content="Neural networks are computing systems inspired by biological neural networks",
        )

        _doc3 = await service.sync_document_to_vector_db(
            namespace="docs:web",
            file_path="/docs/html_basics.md",
            content="HTML is the standard markup language for creating web pages",
        )

        # Step 2: Semantic search with AI-related query
        results = await service.semantic_search(
            query_text="artificial intelligence and deep learning",
            limit=10,
            distance_threshold=1000.0,
        )

        # Verify: AI documents should rank higher than HTML
        assert len(results) >= 2, "Should find at least 2 results"
        ai_doc_ids = {doc1["document_id"], doc2["document_id"]}
        top_result_id = results[0]["document_id"]
        assert top_result_id in ai_doc_ids, "AI document should rank highest"

        # Verify distance field exists
        assert "distance" in results[0], "Results should include distance metric"
        assert results[0]["distance"] >= 0, "Distance should be non-negative"

    @pytest.mark.asyncio
    async def test_semantic_search_with_namespace_filtering(self, memory_db: Database) -> None:
        """Test namespace filtering in semantic search."""
        service = DocumentIndexService(memory_db)

        # Index documents in different namespaces
        await service.sync_document_to_vector_db(
            namespace="docs:python",
            file_path="/docs/python_guide.md",
            content="Python is a high-level programming language",
        )

        await service.sync_document_to_vector_db(
            namespace="docs:javascript",
            file_path="/docs/js_guide.md",
            content="JavaScript is a scripting language for web development",
        )

        # Search with namespace filter
        results = await service.semantic_search(
            query_text="programming languages", namespace="docs:python", limit=5
        )

        # Verify: Only python namespace results
        assert len(results) > 0, "Should find results"
        assert all(
            r["namespace"].startswith("docs:python") for r in results
        ), "All results should be from docs:python namespace"

    @pytest.mark.asyncio
    async def test_multiple_search_iterations(self, memory_db: Database) -> None:
        """Test multiple semantic searches don't interfere."""
        service = DocumentIndexService(memory_db)

        # Index documents
        await service.sync_document_to_vector_db(
            namespace="docs:db",
            file_path="/docs/sql.md",
            content="SQL is a domain-specific language for managing databases",
        )

        # Multiple searches
        results1 = await service.semantic_search("database queries", limit=5)
        results2 = await service.semantic_search("relational databases", limit=5)
        results3 = await service.semantic_search("SQL commands", limit=5)

        # All should return results
        assert len(results1) > 0, "First search should return results"
        assert len(results2) > 0, "Second search should return results"
        assert len(results3) > 0, "Third search should return results"

    @pytest.mark.asyncio
    async def test_embedding_generation_ollama_unavailable(self, memory_db: Database) -> None:
        """Test graceful handling when Ollama service is unavailable."""
        # Create embedding service with invalid URL
        bad_service = EmbeddingService(base_url="http://localhost:99999")

        # Attempt embedding generation should raise httpx error
        with pytest.raises((httpx.ConnectError, httpx.HTTPError, httpx.TimeoutException)):
            await bad_service.generate_embedding("test content")

    @pytest.mark.asyncio
    async def test_search_with_wrong_dimensions(self, memory_db: Database) -> None:
        """Test error handling for incorrect embedding dimensions."""
        service = DocumentIndexService(memory_db)

        # Attempt search with wrong dimensions (should be 768, not 128)
        wrong_embedding = [0.1] * 128

        # Should raise an error or return empty results
        try:
            results = await service.search_by_embedding(wrong_embedding, limit=5)
            # If no error, results should be empty due to dimension mismatch
            assert len(results) == 0, "Wrong dimensions should return no results or raise error"
        except Exception as e:
            # Expected behavior: dimension mismatch error
            error_msg = str(e).lower()
            assert (
                "dimension" in error_msg or "size" in error_msg or "struct" in error_msg
            ), f"Expected dimension-related error, got: {e}"

    @pytest.mark.asyncio
    async def test_embedding_empty_content(self, memory_db: Database) -> None:
        """Test handling of empty or whitespace-only content."""
        service = DocumentIndexService(memory_db)

        # Ollama returns 0-dimensional embeddings for empty strings, which raises ValueError
        with pytest.raises(ValueError, match="Expected 768 dimensions, got 0"):
            await service.sync_document_to_vector_db(
                namespace="docs:test", file_path="/docs/empty.md", content=""
            )

    @pytest.mark.asyncio
    async def test_large_content_embedding(self, memory_db: Database) -> None:
        """Test embedding generation for large documents."""
        service = DocumentIndexService(memory_db)

        # Create large content (~2KB - within Ollama's limits)
        large_content = (
            "This is a test document about machine learning and artificial intelligence. " * 30
        )

        result = await service.sync_document_to_vector_db(
            namespace="docs:large",
            file_path="/docs/large_doc.md",
            content=large_content,
        )

        assert result["document_id"] > 0, "Should index large document"

        # Verify searchable
        results = await service.semantic_search("machine learning test document", limit=5)
        assert any(
            r["file_path"] == "/docs/large_doc.md" for r in results
        ), "Large document should be searchable"

    @pytest.mark.asyncio
    async def test_special_characters_in_content(self, memory_db: Database) -> None:
        """Test embedding with special characters and unicode."""
        service = DocumentIndexService(memory_db)

        content = "Python code: print('Hello 世界! 🌍')\n" "SQL: SELECT * FROM users WHERE id = 1;"

        result = await service.sync_document_to_vector_db(
            namespace="docs:unicode", file_path="/docs/special.md", content=content
        )

        assert result["document_id"] > 0, "Should index content with special characters"

        # Verify searchable
        results = await service.semantic_search("programming code examples", limit=5)
        assert len(results) >= 0, "Should not crash with unicode content"

    @pytest.mark.asyncio
    async def test_distance_threshold_filtering(self, memory_db: Database) -> None:
        """Test that distance threshold properly filters results."""
        service = DocumentIndexService(memory_db)

        # Index a document
        await service.sync_document_to_vector_db(
            namespace="docs:test",
            file_path="/docs/python.md",
            content="Python programming language guide",
        )

        # Search with very strict threshold (very low distance)
        strict_results = await service.semantic_search(
            query_text="Java programming",  # Different language
            limit=10,
            distance_threshold=0.1,  # Very strict
        )

        # Search with permissive threshold
        permissive_results = await service.semantic_search(
            query_text="Java programming", limit=10, distance_threshold=1000.0
        )

        # Permissive should return more or equal results
        assert len(permissive_results) >= len(
            strict_results
        ), "Permissive threshold should return more results"

    @pytest.mark.asyncio
    async def test_concurrent_searches(self, memory_db: Database) -> None:
        """Test multiple concurrent semantic searches."""
        service = DocumentIndexService(memory_db)

        # Index documents
        await service.sync_document_to_vector_db(
            namespace="docs:async",
            file_path="/docs/asyncio.md",
            content="Asyncio is Python's async I/O library",
        )

        # Concurrent searches
        async def search(query: str) -> list[dict[str, Any]]:
            return await service.semantic_search(query, limit=5)

        results = await asyncio.gather(
            search("asynchronous programming"),
            search("Python async"),
            search("I/O operations"),
        )

        # All should complete successfully
        assert len(results) == 3, "All concurrent searches should complete"
        assert all(isinstance(r, list) for r in results), "All results should be lists"

    @pytest.mark.asyncio
    async def test_embedding_metadata_consistency(self, memory_db: Database) -> None:
        """Test that embedding metadata matches document data."""
        service = DocumentIndexService(memory_db)

        namespace = "docs:consistency"
        file_path = "/docs/test.md"

        result = await service.sync_document_to_vector_db(
            namespace=namespace,
            file_path=file_path,
            content="Test document for consistency checks",
        )

        # Verify metadata matches
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT * FROM document_embedding_metadata WHERE document_id = ?",
                (result["document_id"],),
            )
            metadata = await cursor.fetchone()

            assert metadata is not None, "Metadata should exist"
            assert metadata["namespace"] == namespace, "Namespace should match"
            assert metadata["file_path"] == file_path, "File path should match"
            assert metadata["embedding_model"] == "nomic-embed-text-v1.5", "Model should match"
            assert metadata["rowid"] == result["embedding_rowid"], "Rowid should match"

    @pytest.mark.asyncio
    async def test_search_relevance_ranking(self, memory_db: Database) -> None:
        """Test that search results are ranked by relevance (distance)."""
        service = DocumentIndexService(memory_db)

        # Index documents with varying relevance
        await service.sync_document_to_vector_db(
            namespace="docs:test",
            file_path="/docs/exact_match.md",
            content="Python machine learning and deep learning frameworks",
        )

        await service.sync_document_to_vector_db(
            namespace="docs:test",
            file_path="/docs/partial_match.md",
            content="Machine learning algorithms for data analysis",
        )

        await service.sync_document_to_vector_db(
            namespace="docs:test",
            file_path="/docs/unrelated.md",
            content="HTML and CSS web design fundamentals",
        )

        # Search for ML/DL related content
        results = await service.semantic_search(
            query_text="Python deep learning", limit=10, distance_threshold=1000.0
        )

        # Verify results are ordered by distance (ascending)
        assert len(results) > 0, "Should find results"
        for i in range(len(results) - 1):
            assert (
                results[i]["distance"] <= results[i + 1]["distance"]
            ), "Results should be ordered by distance ascending"

    @pytest.mark.asyncio
    async def test_namespace_hierarchy_search(self, memory_db: Database) -> None:
        """Test searching with hierarchical namespace patterns."""
        service = DocumentIndexService(memory_db)

        # Index documents with hierarchical namespaces
        await service.sync_document_to_vector_db(
            namespace="docs:python:basics",
            file_path="/docs/python/intro.md",
            content="Introduction to Python programming",
        )

        await service.sync_document_to_vector_db(
            namespace="docs:python:advanced",
            file_path="/docs/python/decorators.md",
            content="Advanced Python decorators and metaclasses",
        )

        await service.sync_document_to_vector_db(
            namespace="docs:javascript:basics",
            file_path="/docs/js/intro.md",
            content="Introduction to JavaScript programming",
        )

        # Search with parent namespace filter
        results = await service.semantic_search(
            query_text="programming introduction", namespace="docs:python", limit=10
        )

        # All results should be from docs:python hierarchy
        assert len(results) > 0, "Should find results"
        for result in results:
            assert result["namespace"].startswith(
                "docs:python"
            ), f"Expected docs:python namespace, got {result['namespace']}"

    @pytest.mark.asyncio
    async def test_duplicate_document_sync_fails(self, memory_db: Database) -> None:
        """Test that syncing duplicate file paths fails appropriately."""
        service = DocumentIndexService(memory_db)

        # First sync succeeds
        result1 = await service.sync_document_to_vector_db(
            namespace="docs:test",
            file_path="/docs/duplicate.md",
            content="Original content",
        )

        assert result1["document_id"] > 0, "First sync should succeed"

        # Second sync with same file_path should fail
        with pytest.raises(ValueError, match="already exists"):
            await service.sync_document_to_vector_db(
                namespace="docs:test",
                file_path="/docs/duplicate.md",
                content="Different content",
            )

    @pytest.mark.asyncio
    async def test_search_with_no_indexed_documents(self, memory_db: Database) -> None:
        """Test semantic search on empty database returns empty results."""
        service = DocumentIndexService(memory_db)

        # Search on empty database
        results = await service.semantic_search(
            query_text="anything", limit=5, distance_threshold=1000.0
        )

        assert results == [], "Search on empty database should return empty list"

    @pytest.mark.asyncio
    async def test_batch_document_indexing(self, memory_db: Database) -> None:
        """Test indexing multiple documents and searching across them."""
        service = DocumentIndexService(memory_db)

        # Batch index documents
        documents = [
            ("docs:lang", "/docs/python.md", "Python is a versatile programming language"),
            ("docs:lang", "/docs/java.md", "Java is an object-oriented programming language"),
            ("docs:lang", "/docs/rust.md", "Rust is a systems programming language"),
            ("docs:web", "/docs/react.md", "React is a JavaScript library for building UIs"),
            ("docs:web", "/docs/vue.md", "Vue is a progressive JavaScript framework"),
        ]

        for namespace, file_path, content in documents:
            await service.sync_document_to_vector_db(
                namespace=namespace, file_path=file_path, content=content
            )

        # Search for programming languages
        results = await service.semantic_search(
            query_text="programming language comparison", limit=10
        )

        # Should find multiple language docs
        assert len(results) >= 3, "Should find multiple programming language docs"

        # Search with web framework filter
        web_results = await service.semantic_search(
            query_text="user interface development", namespace="docs:web", limit=10
        )

        # Should only return web framework docs
        assert len(web_results) > 0, "Should find web framework docs"
        assert all(
            r["namespace"].startswith("docs:web") for r in web_results
        ), "Should only return docs:web results"

    @pytest.mark.asyncio
    async def test_embedding_service_health_check(self, memory_db: Database) -> None:
        """Test Ollama service health check."""
        service = EmbeddingService()

        # Check if service is accessible
        is_healthy = await service.health_check()

        # Should return True if Ollama is running, False otherwise
        assert isinstance(is_healthy, bool), "Health check should return boolean"

        # If unhealthy, embedding generation should fail
        if not is_healthy:
            with pytest.raises(Exception):  # noqa: B017
                await service.generate_embedding("test")

    @pytest.mark.asyncio
    async def test_whitespace_only_content(self, memory_db: Database) -> None:
        """Test handling of whitespace-only content."""
        service = DocumentIndexService(memory_db)

        # Sync document with only whitespace
        result = await service.sync_document_to_vector_db(
            namespace="docs:test",
            file_path="/docs/whitespace.md",
            content="   \n\t\n   ",  # Only whitespace
        )

        assert result["document_id"] > 0, "Should create document with whitespace"
        assert result["embedding_rowid"] > 0, "Should create embedding"

        # Document should be searchable (even if not relevant)
        doc = await service.get_document_by_id(result["document_id"])
        assert doc is not None, "Document should exist"

    @pytest.mark.asyncio
    async def test_very_long_namespace(self, memory_db: Database) -> None:
        """Test handling of very long namespace strings."""
        service = DocumentIndexService(memory_db)

        # Create very long namespace
        long_namespace = "docs:" + ":".join(["level"] * 50)  # 50 levels deep

        result = await service.sync_document_to_vector_db(
            namespace=long_namespace,
            file_path="/docs/deep.md",
            content="Test content with deep namespace",
        )

        assert result["document_id"] > 0, "Should handle long namespaces"

        # Search with namespace filter
        results = await service.semantic_search(
            query_text="test content", namespace=long_namespace, limit=5
        )

        # Should filter correctly
        if results:
            assert results[0]["namespace"] == long_namespace, "Namespace should match exactly"
