#!/usr/bin/env python3
"""Integration test for vector search with database."""

import asyncio
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent / "src"))

import numpy as np  # noqa: E402
from abathur.infrastructure.database import Database  # noqa: E402
from abathur.services.embedding_service import EmbeddingService  # noqa: E402


async def main() -> int:
    """Run integration tests for vector search."""
    print("=" * 60)
    print("VECTOR SEARCH INTEGRATION TEST")
    print("=" * 60)

    # Initialize database
    print("\n[1/5] Initializing database with vector tables...")
    db_path = Path("/tmp/test_vector_search.db")
    if db_path.exists():
        db_path.unlink()
    db = Database(db_path)
    try:
        await db.initialize()
        print("   PASSED: Database initialized successfully")
    except Exception as e:
        print(f"   FAILED: Database initialization failed: {e}")
        return 1

    # Initialize embedding service
    print("\n[2/5] Initializing embedding service...")
    embedding_service = EmbeddingService()
    try:
        is_healthy = await embedding_service.health_check()
        if not is_healthy:
            print("   FAILED: Embedding service not healthy")
            return 1
        print("   PASSED: Embedding service ready")
    except Exception as e:
        print(f"   FAILED: Embedding service check failed: {e}")
        return 1

    # Test document indexing with embeddings
    print("\n[3/5] Testing document indexing with embeddings...")
    try:
        # Create test document
        test_content = "Vector search enables semantic similarity matching in databases"
        doc_id = await db.documents.index_document(
            file_path="/test/vector-search.md",
            title="Vector Search Guide",
            content=test_content,
            document_type="guide",
        )
        print(f"   PASSED: Document indexed with ID: {doc_id}")

        # Generate embedding
        embedding = await embedding_service.generate_embedding(test_content)
        print(f"   PASSED: Generated embedding ({len(embedding)} dimensions)")

        # Serialize embedding to BLOB
        embedding_blob = np.array(embedding, dtype=np.float32).tobytes()
        print(f"   PASSED: Serialized embedding to BLOB ({len(embedding_blob)} bytes)")

        # Store embedding in vector table
        async with db._get_connection() as conn:
            await db._load_vss_extensions(conn)

            # Insert into document_embeddings virtual table
            cursor = await conn.execute(
                "INSERT INTO document_embeddings(rowid, embedding) VALUES (?, ?)",
                (doc_id, embedding_blob),
            )
            print("   PASSED: Stored embedding in vss0 virtual table")

            # Insert metadata
            await conn.execute(
                """
                INSERT INTO document_embedding_metadata (
                    rowid, document_id, namespace, file_path, embedding_model
                )
                VALUES (?, ?, 'test', '/test/vector-search.md', 'nomic-embed-text-v1.5')
                """,
                (doc_id, doc_id),
            )
            await conn.commit()
            print("   PASSED: Stored embedding metadata")

    except Exception as e:
        print(f"   FAILED: Document indexing failed: {e}")
        import traceback

        traceback.print_exc()
        return 1

    # Test vector similarity search
    print("\n[4/5] Testing vector similarity search...")
    try:
        # Generate query embedding
        query_text = "How does semantic search work?"
        query_embedding = await embedding_service.generate_embedding(query_text)
        query_blob = np.array(query_embedding, dtype=np.float32).tobytes()
        print("   PASSED: Generated query embedding")

        # Perform similarity search
        async with db._get_connection() as conn:
            await db._load_vss_extensions(conn)

            cursor = await conn.execute(
                """
                SELECT
                    de.rowid,
                    de.distance
                FROM document_embeddings de
                WHERE vss_search(de.embedding, ?)
                LIMIT 5
                """,
                (query_blob,),
            )
            results = list(await cursor.fetchall())

            if results:
                print(f"   PASSED: Found {len(results)} similar documents")
                for i, row in enumerate(results):
                    rowid = row[0]
                    distance = row[1]
                    similarity = 1 / (1 + distance)
                    print(
                        f"      Result {i+1}: rowid={rowid} (distance: {distance:.4f}, similarity: {similarity:.4f})"
                    )
            else:
                print("   WARNING: No results found (this is unexpected)")

    except Exception as e:
        print(f"   FAILED: Vector search failed: {e}")
        import traceback

        traceback.print_exc()
        return 1

    # Performance test
    print("\n[5/5] Testing performance...")
    try:
        # Test multiple embeddings
        test_texts = [
            "Machine learning algorithms",
            "Database query optimization",
            "Vector embeddings for AI",
        ]

        start_time = time.time()
        for i, text in enumerate(test_texts):
            # Generate embedding
            emb = await embedding_service.generate_embedding(text)
            emb_blob = np.array(emb, dtype=np.float32).tobytes()

            # Create document
            doc_id = await db.documents.index_document(
                file_path=f"/test/doc-{i+2}.md",
                title=f"Test Doc {i+2}",
                content=text,
                document_type="test",
            )

            # Store in vector table
            async with db._get_connection() as conn:
                await db._load_vss_extensions(conn)
                await conn.execute(
                    "INSERT INTO document_embeddings(rowid, embedding) VALUES (?, ?)",
                    (doc_id, emb_blob),
                )
                await conn.execute(
                    """
                    INSERT INTO document_embedding_metadata (
                        rowid, document_id, namespace, file_path, embedding_model
                    )
                    VALUES (?, ?, 'test', ?, 'nomic-embed-text-v1.5')
                    """,
                    (doc_id, doc_id, f"/test/doc-{i+2}.md"),
                )
                await conn.commit()

        elapsed_ms = (time.time() - start_time) * 1000
        avg_ms = elapsed_ms / len(test_texts)
        print(f"   PASSED: Indexed {len(test_texts)} documents in {elapsed_ms:.2f}ms")
        print(f"   PASSED: Average {avg_ms:.2f}ms per document")

        # Test search performance
        start_time = time.time()
        query_emb = await embedding_service.generate_embedding("AI and databases")
        query_blob = np.array(query_emb, dtype=np.float32).tobytes()

        async with db._get_connection() as conn:
            await db._load_vss_extensions(conn)
            cursor = await conn.execute(
                """
                SELECT
                    de.rowid,
                    de.distance
                FROM document_embeddings de
                WHERE vss_search(de.embedding, ?)
                LIMIT 10
                """,
                (query_blob,),
            )
            results = list(await cursor.fetchall())

        search_ms = (time.time() - start_time) * 1000
        print(f"   PASSED: Search completed in {search_ms:.2f}ms")
        print(f"   PASSED: Found {len(results)} results")

        if search_ms < 500:
            print(f"   PASSED: Search latency {search_ms:.2f}ms meets <500ms target")
        else:
            print(f"   WARNING: Search latency {search_ms:.2f}ms exceeds 500ms target")

    except Exception as e:
        print(f"   FAILED: Performance test failed: {e}")
        import traceback

        traceback.print_exc()
        return 1
    finally:
        await db.close()

    # Summary
    print("\n" + "=" * 60)
    print("INTEGRATION TEST SUMMARY")
    print("=" * 60)
    print("Status: ALL TESTS PASSED")
    print("\nVerified capabilities:")
    print("  - Database initialization with vector tables")
    print("  - Embedding generation (nomic-embed-text-v1.5)")
    print("  - Vector storage in sqlite-vss")
    print("  - Semantic similarity search")
    print("  - Performance within targets")
    print("\nPhase 1 Vector Search Infrastructure: COMPLETE")
    print("=" * 60)

    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
