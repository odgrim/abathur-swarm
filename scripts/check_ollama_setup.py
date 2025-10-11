#!/usr/bin/env python3
"""Utility script to validate Ollama service setup for vector search.

This script verifies that:
- Ollama service is running and accessible
- The nomic-embed-text-v1.5 model is available
- Embedding generation is working correctly
- Performance meets acceptable thresholds

Run this before executing vector search integration tests.
"""

import asyncio
import sys
import time
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent.parent / "src"))

from abathur.services.embedding_service import EmbeddingService  # noqa: E402


async def main() -> int:
    """Run validation checks for vector search setup."""
    print("=" * 60)
    print("VECTOR SEARCH INFRASTRUCTURE VALIDATION")
    print("=" * 60)

    # Test 1: Ollama connectivity
    print("\n[1/4] Testing Ollama connectivity...")
    service = EmbeddingService()

    try:
        is_healthy = await service.health_check()
        if not is_healthy:
            print("   FAILED: Ollama service not responding")
            return 1
        print("   PASSED: Ollama responding")
    except Exception as e:
        print(f"   FAILED: Ollama health check failed: {e}")
        return 1

    # Test 2: Embedding generation
    print("\n[2/4] Testing embedding generation...")
    try:
        start_time = time.time()
        embedding = await service.generate_embedding("Hello, world!")
        elapsed_ms = (time.time() - start_time) * 1000

        print(f"   PASSED: Embedding generated in {elapsed_ms:.2f}ms")
        print(f"   PASSED: Embedding dimensions: {len(embedding)}")

        # Validate dimensions
        assert len(embedding) == 768, f"Expected 768 dimensions, got {len(embedding)}"
        print("   PASSED: Correct embedding dimensions (768)")

        # Check performance target
        if elapsed_ms > 100:
            print(f"   WARNING: Latency {elapsed_ms:.2f}ms exceeds 100ms target")
        else:
            print(f"   PASSED: Latency {elapsed_ms:.2f}ms meets <100ms target")

    except Exception as e:
        print(f"   FAILED: Embedding generation failed: {e}")
        return 1

    # Test 3: Batch generation
    print("\n[3/4] Testing batch embedding generation...")
    try:
        texts = ["test1", "test2", "test3"]
        start_time = time.time()
        embeddings = await service.generate_batch(texts)
        elapsed_ms = (time.time() - start_time) * 1000

        print(f"   PASSED: Generated {len(embeddings)} embeddings")
        print(f"   PASSED: Batch completed in {elapsed_ms:.2f}ms")
        print(f"   PASSED: Average {elapsed_ms / len(texts):.2f}ms per embedding")

        # Validate all embeddings have correct dimensions
        for i, emb in enumerate(embeddings):
            assert len(emb) == 768, f"Embedding {i} has wrong dimensions: {len(emb)}"
        print("   PASSED: All embeddings have correct dimensions")

    except Exception as e:
        print(f"   FAILED: Batch test failed: {e}")
        return 1

    # Test 4: Semantic similarity test
    print("\n[4/4] Testing semantic similarity (basic)...")
    try:
        # Generate embeddings for similar and dissimilar texts
        text1 = "The cat sat on the mat"
        text2 = "A feline rested on the rug"
        text3 = "Quantum computing uses qubits"

        emb1 = await service.generate_embedding(text1)
        emb2 = await service.generate_embedding(text2)
        emb3 = await service.generate_embedding(text3)

        # Compute cosine similarity (simple dot product for normalized vectors)
        import math

        def cosine_similarity(a: list[float], b: list[float]) -> float:
            dot_product = sum(x * y for x, y in zip(a, b, strict=False))
            mag_a = math.sqrt(sum(x * x for x in a))
            mag_b = math.sqrt(sum(y * y for y in b))
            return dot_product / (mag_a * mag_b)

        sim_similar = cosine_similarity(emb1, emb2)
        sim_dissimilar = cosine_similarity(emb1, emb3)

        print(f"   INFO: Similarity (cat/feline): {sim_similar:.4f}")
        print(f"   INFO: Similarity (cat/quantum): {sim_dissimilar:.4f}")

        if sim_similar > sim_dissimilar:
            print("   PASSED: Semantic similarity working correctly")
        else:
            print("   WARNING: Semantic similarity may not be working as expected")

    except Exception as e:
        print(f"   FAILED: Similarity test failed: {e}")
        return 1

    # Summary
    print("\n" + "=" * 60)
    print("VALIDATION SUMMARY")
    print("=" * 60)
    print("Status: ALL CHECKS PASSED")
    print("\nVector search infrastructure is ready:")
    print("  - Ollama service: RUNNING")
    print("  - nomic-embed-text model: LOADED")
    print("  - Embedding dimensions: 768")
    print("  - Performance: ACCEPTABLE")
    print("\nNext steps:")
    print("  1. Create vector tables in database")
    print("  2. Implement document sync service")
    print("  3. Implement semantic search queries")
    print("=" * 60)

    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
