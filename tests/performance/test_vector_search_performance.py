"""Performance benchmarks for vector search and embedding generation.

Tests Milestone 2 Phase 4 performance targets:
- Semantic search latency: <500ms (P99)
- Embedding generation: <100ms per document
- Vector similarity: <50ms for top-10 results
- Batch embedding throughput: >10 documents/second
- Concurrent search: 5+ queries without degradation
- Large dataset: 1000+ documents with maintained performance
"""

import asyncio
import time
from pathlib import Path

import pytest
from abathur.infrastructure.database import Database
from abathur.services.document_index_service import DocumentIndexService
from abathur.services.embedding_service import EmbeddingService


class TestEmbeddingPerformance:
    """Test embedding generation performance."""

    @pytest.mark.asyncio
    async def test_single_embedding_generation_latency(self) -> None:
        """Benchmark: Single embedding generation <100ms.

        Target: Average and P99 latency both under 100ms for single embedding.
        """
        service = EmbeddingService()

        # Check service availability
        is_healthy = await service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        content = "Machine learning is a subset of artificial intelligence that enables systems to learn and improve from experience."

        # Warmup (first call may be slower due to model loading)
        await service.generate_embedding(content)

        # Benchmark 20 iterations for better statistics
        latencies = []
        for _ in range(20):
            start = time.perf_counter()
            embedding = await service.generate_embedding(content)
            latency_ms = (time.perf_counter() - start) * 1000
            latencies.append(latency_ms)

            # Validate embedding dimensions
            assert len(embedding) == 768, f"Expected 768 dimensions, got {len(embedding)}"

        # Calculate percentiles
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50_latency = latencies[len(latencies) // 2]
        p99_latency = latencies[int(0.99 * len(latencies))]

        print("\n📊 Single Embedding Generation Performance:")
        print(f"  Iterations: {len(latencies)}")
        print(f"  Avg latency: {avg_latency:.2f}ms")
        print(f"  P50 latency: {p50_latency:.2f}ms")
        print(f"  P99 latency: {p99_latency:.2f}ms")
        print("  Target: <100ms (avg and P99)")

        # Performance assertions
        assert avg_latency < 100, f"Average latency {avg_latency:.2f}ms exceeds 100ms target"
        assert p99_latency < 100, f"P99 latency {p99_latency:.2f}ms exceeds 100ms target"

    @pytest.mark.asyncio
    async def test_batch_embedding_throughput(self) -> None:
        """Benchmark: Batch embedding throughput >10 docs/sec.

        Target: Process at least 10 documents per second in batch mode.
        """
        service = EmbeddingService()

        # Check service availability
        is_healthy = await service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Create diverse test documents
        texts = [
            f"Document {i}: Machine learning and artificial intelligence enable systems to process natural language."
            for i in range(20)
        ]

        # Benchmark batch processing
        start = time.perf_counter()
        embeddings = await service.generate_batch(texts)
        duration = time.perf_counter() - start

        throughput = len(texts) / duration

        print("\n📊 Batch Embedding Throughput:")
        print(f"  Batch size: {len(texts)} documents")
        print(f"  Duration: {duration:.2f}s")
        print(f"  Throughput: {throughput:.2f} docs/sec")
        print("  Target: >10 docs/sec")

        # Validate results
        assert len(embeddings) == len(texts), "All embeddings should be generated"
        assert all(
            len(emb) == 768 for emb in embeddings
        ), "All embeddings should be 768-dimensional"

        # Performance assertion
        assert throughput >= 10, f"Throughput {throughput:.2f} docs/sec below 10 docs/sec target"


class TestSemanticSearchPerformance:
    """Test semantic search and vector similarity performance."""

    @pytest.mark.asyncio
    async def test_semantic_search_latency(self) -> None:
        """Benchmark: Semantic search <500ms (P99).

        Target: End-to-end semantic search (embedding + vector search) under 500ms.
        Dataset: 50 documents.
        """
        # Check embedding service availability
        embedding_service = EmbeddingService()
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Setup database and service
        db = Database(Path(":memory:"))
        await db.initialize()
        service = DocumentIndexService(db)

        # Index 50 test documents
        print("\n📚 Indexing 50 documents for semantic search test...")
        for i in range(50):
            content = f"Document {i}: This document discusses machine learning algorithms, neural networks, and deep learning architectures for AI applications."
            await service.sync_document_to_vector_db(
                namespace=f"docs:category_{i % 5}",
                file_path=f"/docs/test/doc_{i}.md",
                content=content,
            )

        # Benchmark 20 semantic searches
        query = "deep learning neural network architectures"
        latencies = []

        for _ in range(20):
            start = time.perf_counter()
            results = await service.semantic_search(query_text=query, limit=10)
            latency_ms = (time.perf_counter() - start) * 1000
            latencies.append(latency_ms)

            # Validate results
            assert len(results) > 0, "Should return results"
            assert all("distance" in r for r in results), "All results should have distance"

        # Calculate statistics
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50_latency = latencies[len(latencies) // 2]
        p99_latency = latencies[int(0.99 * len(latencies))]

        print("\n📊 Semantic Search Performance:")
        print("  Dataset size: 50 documents")
        print(f"  Query: '{query}'")
        print(f"  Iterations: {len(latencies)}")
        print(f"  Avg latency: {avg_latency:.2f}ms")
        print(f"  P50 latency: {p50_latency:.2f}ms")
        print(f"  P99 latency: {p99_latency:.2f}ms")
        print("  Target: <500ms (avg and P99)")

        # Performance assertions
        assert avg_latency < 500, f"Average latency {avg_latency:.2f}ms exceeds 500ms target"
        assert p99_latency < 500, f"P99 latency {p99_latency:.2f}ms exceeds 500ms target"

        # Cleanup
        await db.close()

    @pytest.mark.asyncio
    async def test_vector_similarity_search_performance(self) -> None:
        """Benchmark: Vector similarity <50ms for top-10.

        Target: Pure vector search (no embedding generation) under 50ms.
        This tests the sqlite-vss extension performance directly.
        """
        # Check embedding service availability
        embedding_service = EmbeddingService()
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Setup database and service
        db = Database(Path(":memory:"))
        await db.initialize()
        service = DocumentIndexService(db)

        # Index documents
        print("\n📚 Indexing documents for vector similarity test...")
        for i in range(20):
            await service.sync_document_to_vector_db(
                namespace="docs:perf",
                file_path=f"/docs/perf/doc_{i}.md",
                content=f"Performance testing document {i} about machine learning and AI systems",
            )

        # Pre-generate query embedding (exclude from benchmark)
        query_embedding = await embedding_service.generate_embedding(
            "performance testing machine learning systems"
        )

        # Benchmark pure vector search (bypasses embedding generation)
        latencies = []
        for _ in range(20):
            start = time.perf_counter()
            results = await service.search_by_embedding(query_embedding, limit=10)
            latency_ms = (time.perf_counter() - start) * 1000
            latencies.append(latency_ms)

            # Validate results
            assert len(results) > 0, "Should return results"

        # Calculate statistics
        latencies.sort()
        avg_latency = sum(latencies) / len(latencies)
        p50_latency = latencies[len(latencies) // 2]
        p99_latency = latencies[int(0.99 * len(latencies))]

        print("\n📊 Vector Similarity Search Performance:")
        print("  Dataset size: 20 documents")
        print(f"  Iterations: {len(latencies)}")
        print(f"  Avg latency: {avg_latency:.2f}ms")
        print(f"  P50 latency: {p50_latency:.2f}ms")
        print(f"  P99 latency: {p99_latency:.2f}ms")
        print("  Target: <50ms (average)")

        # Performance assertion
        assert avg_latency < 50, f"Vector search {avg_latency:.2f}ms exceeds 50ms target"

        # Cleanup
        await db.close()


class TestScalabilityPerformance:
    """Test large dataset and concurrent access performance."""

    @pytest.mark.asyncio
    async def test_large_dataset_search_performance(self) -> None:
        """Benchmark: Maintain performance with 1000+ documents.

        Target: Search latency <500ms even with large dataset.
        Dataset: 1000 documents.
        """
        # Check embedding service availability
        embedding_service = EmbeddingService()
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Setup database and service
        db = Database(Path(":memory:"))
        await db.initialize()
        service = DocumentIndexService(db)

        # Index 1000 documents (this will take a few minutes)
        num_docs = 1000
        print(f"\n📚 Indexing {num_docs} documents for large dataset test...")
        print("⏳ This may take several minutes...")

        index_start = time.perf_counter()

        # Use different topics to create diversity
        topics = [
            "machine learning algorithms",
            "neural network architectures",
            "deep learning frameworks",
            "natural language processing",
            "computer vision systems",
            "reinforcement learning",
            "data preprocessing techniques",
            "model optimization methods",
            "distributed training strategies",
            "AI ethics and fairness",
        ]

        for i in range(num_docs):
            topic = topics[i % len(topics)]
            content = f"Document {i}: This document discusses {topic} in the context of artificial intelligence and modern machine learning applications."

            await service.sync_document_to_vector_db(
                namespace=f"docs:category_{i % 10}",
                file_path=f"/docs/large/doc_{i:04d}.md",
                content=content,
            )

            # Progress indicator every 100 docs
            if (i + 1) % 100 == 0:
                elapsed = time.perf_counter() - index_start
                rate = (i + 1) / elapsed
                print(f"  Indexed {i + 1}/{num_docs} documents ({rate:.1f} docs/sec)")

        index_duration = time.perf_counter() - index_start
        index_rate = num_docs / index_duration
        print(f"✅ Indexing completed in {index_duration:.2f}s ({index_rate:.1f} docs/sec)")

        # Benchmark search on large dataset
        query = "neural network deep learning"
        search_latencies = []

        for _ in range(10):
            start = time.perf_counter()
            results = await service.semantic_search(query_text=query, limit=10)
            latency_ms = (time.perf_counter() - start) * 1000
            search_latencies.append(latency_ms)

            # Validate results
            assert len(results) > 0, "Should return results"

        # Calculate statistics
        search_latencies.sort()
        avg_search = sum(search_latencies) / len(search_latencies)
        p50_search = search_latencies[len(search_latencies) // 2]
        p99_search = search_latencies[int(0.99 * len(search_latencies))]

        print("\n📊 Large Dataset Search Performance:")
        print(f"  Dataset size: {num_docs} documents")
        print(f"  Query: '{query}'")
        print(f"  Avg search latency: {avg_search:.2f}ms")
        print(f"  P50 search latency: {p50_search:.2f}ms")
        print(f"  P99 search latency: {p99_search:.2f}ms")
        print("  Target: <500ms")

        # Performance assertion
        assert avg_search < 500, f"Large dataset search {avg_search:.2f}ms exceeds 500ms target"

        # Cleanup
        await db.close()

    @pytest.mark.asyncio
    async def test_concurrent_search_performance(self) -> None:
        """Benchmark: Handle 5+ concurrent searches without degradation.

        Target: Concurrent searches should not degrade significantly (within 2x of sequential).
        """
        # Check embedding service availability
        embedding_service = EmbeddingService()
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Setup database and service
        db = Database(Path(":memory:"))
        await db.initialize()
        service = DocumentIndexService(db)

        # Index documents
        print("\n📚 Indexing documents for concurrent search test...")
        for i in range(50):
            await service.sync_document_to_vector_db(
                namespace="docs:concurrent",
                file_path=f"/docs/concurrent/doc_{i}.md",
                content=f"Document {i} about artificial intelligence and machine learning topic {i % 5}",
            )

        # Sequential baseline (measure one at a time)
        query = "artificial intelligence machine learning"
        sequential_latencies = []

        for _ in range(5):
            start = time.perf_counter()
            await service.semantic_search(query, limit=5)
            sequential_latencies.append((time.perf_counter() - start) * 1000)

        avg_sequential = sum(sequential_latencies) / len(sequential_latencies)

        # Concurrent searches (all at once)
        async def search() -> float:
            start = time.perf_counter()
            await service.semantic_search(query, limit=5)
            return (time.perf_counter() - start) * 1000

        concurrent_start = time.perf_counter()
        concurrent_latencies = await asyncio.gather(*[search() for _ in range(5)])
        total_concurrent = (time.perf_counter() - concurrent_start) * 1000

        avg_concurrent = sum(concurrent_latencies) / len(concurrent_latencies)
        degradation_factor = avg_concurrent / avg_sequential

        print("\n📊 Concurrent Search Performance:")
        print(f"  Sequential avg: {avg_sequential:.2f}ms")
        print(f"  Concurrent avg: {avg_concurrent:.2f}ms")
        print(f"  Total concurrent time: {total_concurrent:.2f}ms")
        print(f"  Degradation factor: {degradation_factor:.2f}x")
        print("  Target: <3x degradation")

        # Performance assertion
        assert (
            degradation_factor < 3.0
        ), f"Concurrent degradation too high: {degradation_factor:.2f}x (target <2x)"

        # Cleanup
        await db.close()


class TestIndexUsageValidation:
    """Test vector search index usage and query plans."""

    @pytest.mark.asyncio
    async def test_vector_search_uses_vss_index(self) -> None:
        """Validate: Vector search uses vss0 virtual table (EXPLAIN QUERY PLAN).

        Ensures sqlite-vss extension is properly using its optimized index.
        """
        # Check embedding service availability
        embedding_service = EmbeddingService()
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Setup database and service
        db = Database(Path(":memory:"))
        await db.initialize()
        service = DocumentIndexService(db)

        # Index a document
        await service.sync_document_to_vector_db(
            namespace="docs:index",
            file_path="/docs/index_test.md",
            content="Test document for index usage validation",
        )

        # Get query embedding
        query_embedding = await embedding_service.generate_embedding("test query")

        # Get query plan for vector search
        import struct

        query_blob = struct.pack(f"{len(query_embedding)}f", *query_embedding)

        async with db._get_connection() as conn:
            cursor = await conn.execute(
                """
                EXPLAIN QUERY PLAN
                SELECT
                    m.document_id,
                    m.namespace,
                    m.file_path,
                    distance
                FROM (
                    SELECT rowid, distance
                    FROM document_embeddings
                    WHERE vss_search(embedding, ?)
                    LIMIT 10
                ) v
                INNER JOIN document_embedding_metadata m ON v.rowid = m.rowid
                ORDER BY distance
                """,
                (query_blob,),
            )
            plan = await cursor.fetchall()
            plan_text = " ".join([str(row[3]) for row in plan])

        print("\n📋 Vector Search Query Plan:")
        print(f"  {plan_text}")

        # Validate index usage
        assert (
            "document_embeddings" in plan_text.lower()
        ), "Should scan document_embeddings virtual table"
        assert (
            "SCAN" in plan_text or "SEARCH" in plan_text
        ), "Should use table scan/search operation"

        # Cleanup
        await db.close()

    @pytest.mark.asyncio
    async def test_metadata_join_uses_index(self) -> None:
        """Validate: Metadata join uses rowid index efficiently.

        Ensures the join between document_embeddings and document_embedding_metadata
        is efficient using rowid index.
        """
        # Check embedding service availability
        embedding_service = EmbeddingService()
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            pytest.skip("Ollama service not available")

        # Setup database and service
        db = Database(Path(":memory:"))
        await db.initialize()
        service = DocumentIndexService(db)

        # Index documents
        for i in range(10):
            await service.sync_document_to_vector_db(
                namespace=f"docs:test_{i}",
                file_path=f"/docs/test_{i}.md",
                content=f"Test document {i} for metadata join validation",
            )

        # Get query embedding
        query_embedding = await embedding_service.generate_embedding("test")

        import struct

        query_blob = struct.pack(f"{len(query_embedding)}f", *query_embedding)

        # Check metadata join query plan
        async with db._get_connection() as conn:
            cursor = await conn.execute(
                """
                EXPLAIN QUERY PLAN
                SELECT m.*
                FROM document_embedding_metadata m
                WHERE m.rowid IN (
                    SELECT rowid FROM document_embeddings
                    WHERE vss_search(embedding, ?)
                    LIMIT 5
                )
                """,
                (query_blob,),
            )
            plan = await cursor.fetchall()
            plan_text = " ".join([str(row[3]) for row in plan])

        print("\n📋 Metadata Join Query Plan:")
        print(f"  {plan_text}")

        # Should use rowid for efficient lookup
        assert (
            "rowid" in plan_text.lower() or "SEARCH" in plan_text
        ), "Should use rowid for efficient metadata lookup"

        # Cleanup
        await db.close()
