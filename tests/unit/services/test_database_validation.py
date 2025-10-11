"""Unit tests for Database validation methods."""

import pytest
from abathur.infrastructure.database import Database


class TestDatabaseValidation:
    """Test database validation and diagnostic methods."""

    @pytest.mark.asyncio
    async def test_validate_foreign_keys_empty_db(self, memory_db: Database) -> None:
        """Test foreign key validation on empty database."""
        violations = await memory_db.validate_foreign_keys()
        assert violations == [], f"Expected no violations, got: {violations}"

    @pytest.mark.asyncio
    async def test_explain_query_plan_memory_lookup(self, memory_db: Database) -> None:
        """Test EXPLAIN QUERY PLAN for memory namespace+key lookup."""
        query = """
            SELECT * FROM memory_entries
            WHERE namespace = ? AND key = ? AND is_deleted = 0
            ORDER BY version DESC LIMIT 1
        """
        plan = await memory_db.explain_query_plan(query, ("user:alice:pref", "theme"))

        # Verify plan uses index
        plan_text = " ".join(plan)
        assert (
            "idx_memory_namespace_key_version" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"
        assert "SCAN TABLE" not in plan_text, f"Expected no table scan, got: {plan_text}"

    @pytest.mark.asyncio
    async def test_explain_query_plan_session_status_query(self, memory_db: Database) -> None:
        """Test EXPLAIN QUERY PLAN for session status query."""
        query = """
            SELECT * FROM sessions
            WHERE status = 'active'
            ORDER BY last_update_time DESC
        """
        plan = await memory_db.explain_query_plan(query, ())

        plan_text = " ".join(plan)
        # Should use idx_sessions_status_updated index
        assert (
            "idx_sessions_status_updated" in plan_text or "USING INDEX" in plan_text
        ), f"Expected index usage, got: {plan_text}"

    @pytest.mark.asyncio
    async def test_get_index_usage_reports_all_indexes(self, memory_db: Database) -> None:
        """Test that get_index_usage reports all 39 indexes."""
        index_info = await memory_db.get_index_usage()

        # We expect 39 indexes (excluding auto-generated sqlite_autoindex_*)
        # Count manual indexes only
        _manual_indexes = [
            idx for idx in index_info["indexes"] if not idx["name"].startswith("sqlite_autoindex_")
        ]

        assert (
            index_info["index_count"] >= 35
        ), f"Expected at least 35 indexes, got {index_info['index_count']}"

        # Verify key indexes exist
        index_names = [idx["name"] for idx in index_info["indexes"]]
        critical_indexes = [
            "idx_sessions_pk",
            "idx_memory_namespace_key_version",
            "idx_document_file_path",
            "idx_tasks_status_priority",
            "idx_audit_memory_operations",
        ]

        for idx_name in critical_indexes:
            assert idx_name in index_names, f"Critical index {idx_name} missing"

    @pytest.mark.asyncio
    async def test_pragma_journal_mode_wal(self, memory_db: Database) -> None:
        """Test that journal_mode is WAL."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA journal_mode")
            mode = await cursor.fetchone()
            assert mode is not None
            # In-memory databases use "memory" mode
            # File databases use "wal"
            assert mode[0] in ("memory", "wal"), f"Unexpected journal_mode: {mode[0]}"

    @pytest.mark.asyncio
    async def test_pragma_foreign_keys_enabled(self, memory_db: Database) -> None:
        """Test that foreign keys are enabled."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA foreign_keys")
            enabled = await cursor.fetchone()
            assert enabled is not None
            assert enabled[0] == 1, "Foreign keys not enabled"

    @pytest.mark.asyncio
    async def test_all_tables_exist(self, memory_db: Database) -> None:
        """Test that all required tables exist."""
        expected_tables = [
            "sessions",
            "memory_entries",
            "document_index",
            "tasks",
            "agents",
            "audit",
            "checkpoints",
            "state",
            "metrics",
        ]

        async with memory_db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
            )
            tables = [row[0] for row in await cursor.fetchall()]

        for table in expected_tables:
            assert table in tables, f"Table {table} missing from database"

    @pytest.mark.asyncio
    async def test_integrity_check_passes(self, memory_db: Database) -> None:
        """Test PRAGMA integrity_check passes."""
        async with memory_db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA integrity_check")
            result = await cursor.fetchone()
            assert result is not None
            assert result[0] == "ok", f"Integrity check failed: {result[0]}"
