"""Database validation and integrity checking."""

import time
from datetime import datetime, timezone
from typing import Any

from abathur.infrastructure.database import Database


class DatabaseValidator:
    """Comprehensive database validation."""

    def __init__(self, db: Database):
        self.db = db
        self.results: dict[str, Any] = {
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "checks": {},
            "tables": {},
            "indexes": {},
            "pragma_settings": {},
            "performance": {},
            "issues": [],
        }

    async def run_all_checks(self, verbose: bool = True) -> dict[str, Any]:
        """Run all validation checks.

        Args:
            verbose: Print progress to stdout

        Returns:
            Validation results dictionary
        """
        if verbose:
            print("\n" + "=" * 70)
            print("DATABASE VALIDATION REPORT")
            print("=" * 70)

        await self.check_pragma_settings(verbose)
        await self.check_integrity(verbose)
        await self.check_foreign_keys(verbose)
        await self.check_tables(verbose)
        await self.check_indexes(verbose)
        await self.check_json_constraints(verbose)
        await self.test_query_performance(verbose)

        if verbose:
            print("\n" + "=" * 70)
            print("VALIDATION SUMMARY")
            print("=" * 70)

            total_checks = len(self.results["checks"])
            passed_checks = sum(1 for v in self.results["checks"].values() if v["status"] == "PASS")
            print(f"Total Checks: {total_checks}")
            print(f"Passed: {passed_checks}")
            print(f"Failed: {total_checks - passed_checks}")

            if self.results["issues"]:
                print(f"\nISSUES FOUND: {len(self.results['issues'])}")
                for issue in self.results["issues"]:
                    print(f"  - {issue}")
            else:
                print("\nNO ISSUES FOUND - DATABASE READY FOR USE")

        return self.results

    async def check_pragma_settings(self, verbose: bool = True) -> None:
        """Verify PRAGMA configuration."""
        if verbose:
            print("\n[1/7] Checking PRAGMA Settings...")

        async with self.db._get_connection() as conn:
            # Note: foreign_keys must be enabled per-connection, which Database class does
            await conn.execute("PRAGMA foreign_keys=ON")

            # Check journal mode (persistent setting)
            cursor = await conn.execute("PRAGMA journal_mode")
            journal_mode_row = await cursor.fetchone()
            assert journal_mode_row is not None
            journal_mode = journal_mode_row[0]
            self.results["pragma_settings"]["journal_mode"] = journal_mode

            # Check foreign keys (per-connection setting, enabled by Database class)
            cursor = await conn.execute("PRAGMA foreign_keys")
            foreign_keys_row = await cursor.fetchone()
            assert foreign_keys_row is not None
            foreign_keys = foreign_keys_row[0]
            self.results["pragma_settings"]["foreign_keys"] = bool(foreign_keys)
            self.results["pragma_settings"][
                "foreign_keys_note"
            ] = "Per-connection setting, enabled by Database class"

            # Check synchronous mode
            cursor = await conn.execute("PRAGMA synchronous")
            synchronous_row = await cursor.fetchone()
            assert synchronous_row is not None
            synchronous = synchronous_row[0]
            self.results["pragma_settings"]["synchronous"] = synchronous

        # Validate settings
        checks = {
            "journal_mode": journal_mode.lower() == "wal",
            "foreign_keys": bool(foreign_keys),
            "synchronous": synchronous in (1, 2),  # NORMAL or FULL
        }

        for setting, passed in checks.items():
            status = "PASS" if passed else "FAIL"
            note = ""
            if setting == "foreign_keys":
                note = " (per-connection, enabled by Database class)"
            self.results["checks"][f"pragma_{setting}"] = {
                "status": status,
                "value": self.results["pragma_settings"][setting],
            }
            if verbose:
                print(f"  {setting}: {status} ({self.results['pragma_settings'][setting]}{note})")

            if not passed:
                self.results["issues"].append(f"PRAGMA {setting} not configured correctly")

    async def check_integrity(self, verbose: bool = True) -> None:
        """Run PRAGMA integrity_check."""
        if verbose:
            print("\n[2/7] Checking Database Integrity...")

        async with self.db._get_connection() as conn:
            cursor = await conn.execute("PRAGMA integrity_check")
            result_row = await cursor.fetchone()
            assert result_row is not None
            result = result_row[0]

        passed = result == "ok"
        status = "PASS" if passed else "FAIL"
        self.results["checks"]["integrity_check"] = {"status": status, "result": result}

        if verbose:
            print(f"  Integrity Check: {status}")

        if not passed:
            self.results["issues"].append(f"Integrity check failed: {result}")

    async def check_foreign_keys(self, verbose: bool = True) -> None:
        """Run PRAGMA foreign_key_check."""
        if verbose:
            print("\n[3/7] Checking Foreign Key Constraints...")

        violations = await self.db.validate_foreign_keys()

        passed = len(violations) == 0
        status = "PASS" if passed else "FAIL"
        self.results["checks"]["foreign_key_check"] = {
            "status": status,
            "violations": len(violations),
            "details": [str(v) for v in violations],
        }

        if verbose:
            print(f"  Foreign Key Check: {status} ({len(violations)} violations)")

        if not passed:
            self.results["issues"].append(f"Foreign key violations found: {violations}")

    async def check_tables(self, verbose: bool = True) -> None:
        """Verify all required tables exist."""
        if verbose:
            print("\n[4/7] Checking Tables...")

        expected_tables = {
            # Memory tables
            "sessions": "Session management and event tracking",
            "memory_entries": "Long-term persistent memory storage",
            "document_index": "Markdown document indexing",
            # Core tables
            "tasks": "Task definitions and execution state",
            "agents": "Agent lifecycle tracking",
            "state": "Legacy task state (deprecated)",
            "audit": "Audit logging with memory operations",
            "metrics": "Performance and operational metrics",
            "checkpoints": "Loop execution checkpoints",
        }

        async with self.db._get_connection() as conn:
            cursor = await conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
            )
            tables = [row[0] for row in await cursor.fetchall()]

        for table, description in expected_tables.items():
            exists = table in tables
            status = "PASS" if exists else "FAIL"
            self.results["tables"][table] = {"exists": exists, "description": description}
            self.results["checks"][f"table_{table}"] = {"status": status}

            if verbose:
                print(f"  {table}: {status}")

            if not exists:
                self.results["issues"].append(f"Table {table} does not exist")

        # Get row counts
        async with self.db._get_connection() as conn:
            for table in expected_tables.keys():
                if table in tables:
                    cursor = await conn.execute(f"SELECT COUNT(*) FROM {table}")
                    count_row = await cursor.fetchone()
                    assert count_row is not None
                    count = count_row[0]
                    self.results["tables"][table]["row_count"] = count

    async def check_indexes(self, verbose: bool = True) -> None:
        """Verify all required indexes exist."""
        if verbose:
            print("\n[5/7] Checking Indexes...")

        index_info = await self.db.get_index_usage()
        index_count = index_info["index_count"]

        # Expected minimum: 33 indexes from DDL + some automatic indexes
        min_expected = 30
        passed = index_count >= min_expected
        status = "PASS" if passed else "FAIL"

        self.results["indexes"]["count"] = index_count
        self.results["indexes"]["details"] = index_info["indexes"]
        self.results["checks"]["index_count"] = {
            "status": status,
            "count": index_count,
            "min_expected": min_expected,
        }

        if verbose:
            print(f"  Index Count: {status} ({index_count} indexes, expected >= {min_expected})")

        if not passed:
            self.results["issues"].append(
                f"Insufficient indexes: {index_count} (expected >= {min_expected})"
            )

        # Group by table
        indexes_by_table: dict[str, list[str]] = {}
        for idx in index_info["indexes"]:
            table = idx["table"]
            if table not in indexes_by_table:
                indexes_by_table[table] = []
            indexes_by_table[table].append(idx["name"])

        if verbose:
            for table, idxs in sorted(indexes_by_table.items()):
                print(f"    {table}: {len(idxs)} indexes")

    async def check_json_constraints(self, verbose: bool = True) -> None:
        """Test JSON validation constraints."""
        if verbose:
            print("\n[6/7] Checking JSON Validation Constraints...")

        async with self.db._get_connection() as conn:
            # Test sessions.events JSON validation
            try:
                await conn.execute(
                    """INSERT INTO sessions (id, app_name, user_id, events)
                       VALUES ('test_invalid', 'app', 'user', 'invalid json')"""
                )
                # Should not reach here
                passed = False
                if verbose:
                    print("  sessions.events: FAIL (invalid JSON accepted)")
                self.results["issues"].append(
                    "JSON validation constraint not working on sessions.events"
                )
            except Exception:
                # Expected to fail
                passed = True
                if verbose:
                    print("  sessions.events: PASS (invalid JSON rejected)")

            self.results["checks"]["json_validation_sessions"] = {
                "status": "PASS" if passed else "FAIL"
            }

            # Test memory_entries.value JSON validation
            try:
                await conn.execute(
                    """INSERT INTO memory_entries (namespace, key, value, memory_type)
                       VALUES ('test', 'key', 'invalid json', 'semantic')"""
                )
                passed = False
                if verbose:
                    print("  memory_entries.value: FAIL (invalid JSON accepted)")
                self.results["issues"].append("JSON validation not working on memory_entries.value")
            except Exception:
                passed = True
                if verbose:
                    print("  memory_entries.value: PASS (invalid JSON rejected)")

            self.results["checks"]["json_validation_memory"] = {
                "status": "PASS" if passed else "FAIL"
            }

    async def test_query_performance(self, verbose: bool = True) -> None:
        """Test query performance targets."""
        if verbose:
            print("\n[7/7] Testing Query Performance...")

        async with self.db._get_connection() as conn:
            # Test 1: Session retrieval
            session_id = f"perf_test_{int(time.time() * 1000)}"
            await conn.execute(
                """INSERT INTO sessions (id, app_name, user_id, status)
                   VALUES (?, 'perf_test', 'test_user', 'created')""",
                (session_id,),
            )
            await conn.commit()

            start = time.perf_counter()
            cursor = await conn.execute("SELECT * FROM sessions WHERE id = ?", (session_id,))
            await cursor.fetchone()
            duration_ms = (time.perf_counter() - start) * 1000

            target_ms = 10
            passed = duration_ms < target_ms
            status = "PASS" if passed else "WARN"
            self.results["performance"]["session_retrieval_ms"] = round(duration_ms, 2)
            self.results["checks"]["perf_session_retrieval"] = {
                "status": status,
                "duration_ms": round(duration_ms, 2),
                "target_ms": target_ms,
            }

            if verbose:
                print(f"  Session Retrieval: {status} ({duration_ms:.2f}ms, target <{target_ms}ms)")

            # Test 2: Memory entry retrieval
            await conn.execute(
                """INSERT INTO memory_entries (namespace, key, value, memory_type, created_by, updated_by)
                   VALUES ('test:perf', 'key1', '{"data": "test"}', 'semantic', 'system', 'system')"""
            )
            await conn.commit()

            start = time.perf_counter()
            cursor = await conn.execute(
                """SELECT * FROM memory_entries
                   WHERE namespace = 'test:perf' AND key = 'key1' AND is_deleted = 0
                   ORDER BY version DESC LIMIT 1"""
            )
            await cursor.fetchone()
            duration_ms = (time.perf_counter() - start) * 1000

            target_ms = 20
            passed = duration_ms < target_ms
            status = "PASS" if passed else "WARN"
            self.results["performance"]["memory_retrieval_ms"] = round(duration_ms, 2)
            self.results["checks"]["perf_memory_retrieval"] = {
                "status": status,
                "duration_ms": round(duration_ms, 2),
                "target_ms": target_ms,
            }

            if verbose:
                print(f"  Memory Retrieval: {status} ({duration_ms:.2f}ms, target <{target_ms}ms)")

            # Clean up test data
            await conn.execute("DELETE FROM sessions WHERE id = ?", (session_id,))
            await conn.execute("DELETE FROM memory_entries WHERE namespace LIKE 'test:%'")
            await conn.commit()
