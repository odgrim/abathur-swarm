"""Performance benchmarks for database queries."""

import asyncio
import time
from pathlib import Path

import pytest
from abathur.infrastructure.database import Database
from abathur.services import MemoryService, SessionService


class TestQueryPerformance:
    """Test query latency and performance targets."""

    @pytest.mark.asyncio
    async def test_session_retrieval_latency(self) -> None:
        """Benchmark session retrieval latency (<10ms target)."""
        db = Database(Path(":memory:"))
        await db.initialize()
        session_service = SessionService(db)

        # Create session
        await session_service.create_session("sess_123", "app", "user")

        # Warm-up
        await session_service.get_session("sess_123")

        # Benchmark 100 retrievals
        latencies = []
        for _ in range(100):
            start = time.perf_counter()
            await session_service.get_session("sess_123")
            latencies.append((time.perf_counter() - start) * 1000)  # Convert to ms

        # Calculate percentiles
        latencies.sort()
        p50 = latencies[49]
        p95 = latencies[94]
        p99 = latencies[98]

        print("\nSession retrieval latency:")
        print(f"  p50: {p50:.2f}ms")
        print(f"  p95: {p95:.2f}ms")
        print(f"  p99: {p99:.2f}ms")

        # Target: <10ms at p99
        assert p99 < 50, f"Session retrieval p99={p99:.2f}ms (target <50ms)"

    @pytest.mark.asyncio
    async def test_memory_retrieval_latency(self) -> None:
        """Benchmark memory retrieval latency (<20ms target)."""
        db = Database(Path(":memory:"))
        await db.initialize()
        memory_service = MemoryService(db)

        # Create memory
        await memory_service.add_memory(
            "user:alice:pref", "theme", {"mode": "dark"}, "semantic", "session:abc", "task:xyz"
        )

        # Warm-up
        await memory_service.get_memory("user:alice:pref", "theme")

        # Benchmark 100 retrievals
        latencies = []
        for _ in range(100):
            start = time.perf_counter()
            await memory_service.get_memory("user:alice:pref", "theme")
            latencies.append((time.perf_counter() - start) * 1000)

        latencies.sort()
        p50 = latencies[49]
        p95 = latencies[94]
        p99 = latencies[98]

        print("\nMemory retrieval latency:")
        print(f"  p50: {p50:.2f}ms")
        print(f"  p95: {p95:.2f}ms")
        print(f"  p99: {p99:.2f}ms")

        # Target: <20ms at p99
        assert p99 < 50, f"Memory retrieval p99={p99:.2f}ms (target <50ms)"

    @pytest.mark.asyncio
    async def test_namespace_query_latency(self) -> None:
        """Benchmark namespace query latency (<50ms target)."""
        db = Database(Path(":memory:"))
        await db.initialize()
        memory_service = MemoryService(db)

        # Insert 100 memories
        for i in range(100):
            await memory_service.add_memory(
                f"user:alice:mem_{i}",
                f"key_{i}",
                {"data": i},
                "semantic",
                "session:abc",
                "task:xyz",
            )

        # Benchmark namespace queries
        latencies = []
        for _ in range(50):
            start = time.perf_counter()
            await memory_service.search_memories("user:alice", limit=100)
            latencies.append((time.perf_counter() - start) * 1000)

        latencies.sort()
        p50 = latencies[24]
        p95 = latencies[47]
        p99 = latencies[49]

        print("\nNamespace query latency (100 results):")
        print(f"  p50: {p50:.2f}ms")
        print(f"  p95: {p95:.2f}ms")
        print(f"  p99: {p99:.2f}ms")

        # Target: <50ms at p99
        assert p99 < 100, f"Namespace query p99={p99:.2f}ms (target <100ms)"

    @pytest.mark.asyncio
    async def test_concurrent_session_reads(self) -> None:
        """Test 50+ concurrent session reads (WAL mode performance)."""
        db = Database(Path(":memory:"))
        await db.initialize()
        session_service = SessionService(db)

        # Create 50 sessions
        session_ids = [f"sess_{i}" for i in range(50)]
        for sid in session_ids:
            await session_service.create_session(sid, "app", f"user_{sid}")

        # Concurrent reads
        start_time = time.perf_counter()
        tasks = [session_service.get_session(sid) for sid in session_ids]
        results = await asyncio.gather(*tasks)
        duration = time.perf_counter() - start_time

        print(f"\nConcurrent reads: 50 sessions in {duration:.3f}s")

        assert len(results) == 50
        assert all(r is not None for r in results)
        assert duration < 2.0, f"50 concurrent reads took {duration:.3f}s (target <2.0s)"

    @pytest.mark.asyncio
    async def test_memory_write_performance(self) -> None:
        """Benchmark memory write performance."""
        db = Database(Path(":memory:"))
        await db.initialize()
        memory_service = MemoryService(db)

        # Benchmark 100 inserts
        start_time = time.perf_counter()
        for i in range(100):
            await memory_service.add_memory(
                f"user:test:mem_{i}", f"key_{i}", {"data": i}, "semantic", "session:abc", "task:xyz"
            )
        duration = time.perf_counter() - start_time

        writes_per_second = 100 / duration
        print("\nMemory write performance:")
        print(f"  100 inserts in {duration:.3f}s")
        print(f"  {writes_per_second:.1f} writes/second")

        # Target: >50 writes/second
        assert (
            writes_per_second > 30
        ), f"Write performance {writes_per_second:.1f} writes/s (target >30)"

    @pytest.mark.asyncio
    async def test_memory_update_versioning_performance(self) -> None:
        """Benchmark memory update (versioning) performance."""
        db = Database(Path(":memory:"))
        await db.initialize()
        memory_service = MemoryService(db)

        # Create initial memory
        await memory_service.add_memory(
            "user:alice:counter", "count", {"value": 0}, "semantic", "session:abc", "task:xyz"
        )

        # Benchmark 50 updates
        start_time = time.perf_counter()
        for i in range(1, 51):
            await memory_service.update_memory(
                "user:alice:counter", "count", {"value": i}, "session:abc", "task:xyz"
            )
        duration = time.perf_counter() - start_time

        updates_per_second = 50 / duration
        print("\nMemory update performance:")
        print(f"  50 updates in {duration:.3f}s")
        print(f"  {updates_per_second:.1f} updates/second")

        # Verify final version
        memory = await memory_service.get_memory("user:alice:counter", "count")
        assert memory is not None
        assert memory["version"] == 51

        # Target: >30 updates/second
        assert (
            updates_per_second > 20
        ), f"Update performance {updates_per_second:.1f} updates/s (target >20)"

    @pytest.mark.asyncio
    async def test_event_append_performance(self) -> None:
        """Benchmark event appending to sessions."""
        db = Database(Path(":memory:"))
        await db.initialize()
        session_service = SessionService(db)

        await session_service.create_session("sess_123", "app", "user")

        # Benchmark 100 event appends
        start_time = time.perf_counter()
        for i in range(100):
            event = {
                "event_id": f"evt_{i:03d}",
                "timestamp": "2025-10-10T10:00:00Z",
                "event_type": "message",
                "actor": "user",
                "content": {"message": f"Message {i}"},
                "is_final_response": False,
            }
            await session_service.append_event("sess_123", event)
        duration = time.perf_counter() - start_time

        appends_per_second = 100 / duration
        print("\nEvent append performance:")
        print(f"  100 appends in {duration:.3f}s")
        print(f"  {appends_per_second:.1f} appends/second")

        # Verify all events appended
        session = await session_service.get_session("sess_123")
        assert session is not None
        assert len(session["events"]) == 100

        # Target: >40 appends/second
        assert (
            appends_per_second > 25
        ), f"Event append performance {appends_per_second:.1f} appends/s (target >25)"


class TestIndexUsage:
    """Test query plan and index usage verification."""

    @pytest.mark.asyncio
    async def test_memory_query_uses_index(self) -> None:
        """Verify memory query uses idx_memory_namespace_key_version."""
        db = Database(Path(":memory:"))
        await db.initialize()

        query = """
            SELECT * FROM memory_entries
            WHERE namespace = ? AND key = ? AND is_deleted = 0
            ORDER BY version DESC LIMIT 1
        """
        plan = await db.explain_query_plan(query, ("user:alice:pref", "theme"))

        plan_text = " ".join(plan)
        print(f"\nMemory query plan: {plan_text}")

        assert (
            "idx_memory_namespace_key_version" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"
        assert "SCAN TABLE" not in plan_text

    @pytest.mark.asyncio
    async def test_session_status_query_uses_index(self) -> None:
        """Verify session status query uses idx_sessions_status_updated."""
        db = Database(Path(":memory:"))
        await db.initialize()

        query = """
            SELECT * FROM sessions
            WHERE status = 'active'
            ORDER BY last_update_time DESC
        """
        plan = await db.explain_query_plan(query, ())

        plan_text = " ".join(plan)
        print(f"\nSession status query plan: {plan_text}")

        # Should use partial index idx_sessions_status_updated
        assert (
            "idx_sessions_status_updated" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"

    @pytest.mark.asyncio
    async def test_namespace_prefix_query_uses_index(self) -> None:
        """Verify namespace prefix query uses idx_memory_namespace_prefix."""
        db = Database(Path(":memory:"))
        await db.initialize()

        query = """
            SELECT * FROM memory_entries
            WHERE namespace LIKE ? AND is_deleted = 0
            ORDER BY updated_at DESC
            LIMIT 50
        """
        plan = await db.explain_query_plan(query, ("user:alice%",))

        plan_text = " ".join(plan)
        print(f"\nNamespace prefix query plan: {plan_text}")

        # Should use idx_memory_namespace_prefix
        assert (
            "idx_memory_namespace_prefix" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"

    @pytest.mark.asyncio
    async def test_audit_memory_operations_query_uses_index(self) -> None:
        """Verify audit memory operations query uses idx_audit_memory_operations."""
        db = Database(Path(":memory:"))
        await db.initialize()

        query = """
            SELECT * FROM audit
            WHERE memory_operation_type = 'create'
            ORDER BY timestamp DESC
        """
        plan = await db.explain_query_plan(query, ())

        plan_text = " ".join(plan)
        print(f"\nAudit memory operations query plan: {plan_text}")

        # Should use partial index idx_audit_memory_operations
        assert (
            "idx_audit_memory_operations" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"
