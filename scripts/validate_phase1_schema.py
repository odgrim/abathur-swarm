#!/usr/bin/env python3
"""Validation script for Phase 1 enhanced task queue schema.

This script validates:
1. Schema structure (tables, columns, constraints)
2. Foreign key constraints
3. Index coverage
4. Data integrity
5. Query performance
"""

import asyncio
import sys
from pathlib import Path

# Add src to path for imports
sys.path.insert(0, str(Path(__file__).parent.parent / "src"))

from abathur.domain.models import (  # noqa: E402
    DependencyType,
    Task,
    TaskDependency,
    TaskSource,
    TaskStatus,
)
from abathur.infrastructure.database import Database  # noqa: E402


class Phase1Validator:
    """Validator for Phase 1 schema implementation."""

    def __init__(self, db_path: Path):
        self.db = Database(db_path)
        self.errors = []
        self.warnings = []
        self.validations_passed = 0

    async def run(self) -> bool:
        """Run all validations.

        Returns:
            True if all validations pass, False otherwise
        """
        print("=" * 80)
        print("Phase 1 Schema Validation")
        print("=" * 80)
        print()

        await self.db.initialize()

        # Run validation tests
        await self.validate_schema_structure()
        await self.validate_foreign_keys()
        await self.validate_indexes()
        await self.validate_data_integrity()
        await self.validate_enum_values()
        await self.validate_query_performance()

        await self.db.close()

        # Print summary
        print()
        print("=" * 80)
        print("Validation Summary")
        print("=" * 80)
        print(f"Passed: {self.validations_passed}")
        print(f"Errors: {len(self.errors)}")
        print(f"Warnings: {len(self.warnings)}")
        print()

        if self.errors:
            print("ERRORS:")
            for error in self.errors:
                print(f"  ✗ {error}")
            print()

        if self.warnings:
            print("WARNINGS:")
            for warning in self.warnings:
                print(f"  ⚠ {warning}")
            print()

        if not self.errors:
            print("✓ Phase 1 Schema Validation PASSED")
            return True
        else:
            print("✗ Phase 1 Schema Validation FAILED")
            return False

    async def validate_schema_structure(self):
        """Validate that all required tables and columns exist."""
        print("Validating schema structure...")

        async with self.db._get_connection() as conn:
            # Check tasks table columns
            cursor = await conn.execute("PRAGMA table_info(tasks)")
            columns = await cursor.fetchall()
            column_names = {col["name"] for col in columns}

            required_columns = {
                "id",
                "prompt",
                "agent_type",
                "priority",
                "status",
                "source",
                "dependency_type",
                "calculated_priority",
                "deadline",
                "estimated_duration_seconds",
                "dependency_depth",
            }

            missing = required_columns - column_names
            if missing:
                self.errors.append(f"Missing columns in tasks table: {missing}")
            else:
                self.validations_passed += 1
                print("  ✓ All required columns exist in tasks table")

            # Check task_dependencies table exists
            cursor = await conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='task_dependencies'"
            )
            if not await cursor.fetchone():
                self.errors.append("task_dependencies table not found")
            else:
                self.validations_passed += 1
                print("  ✓ task_dependencies table exists")

                # Check task_dependencies columns
                cursor = await conn.execute("PRAGMA table_info(task_dependencies)")
                dep_columns = await cursor.fetchall()
                dep_column_names = {col["name"] for col in dep_columns}

                required_dep_columns = {
                    "id",
                    "dependent_task_id",
                    "prerequisite_task_id",
                    "dependency_type",
                    "created_at",
                    "resolved_at",
                }

                missing_dep = required_dep_columns - dep_column_names
                if missing_dep:
                    self.errors.append(f"Missing columns in task_dependencies table: {missing_dep}")
                else:
                    self.validations_passed += 1
                    print("  ✓ All required columns exist in task_dependencies table")

    async def validate_foreign_keys(self):
        """Validate foreign key constraints."""
        print("\nValidating foreign key constraints...")

        violations = await self.db.validate_foreign_keys()
        if violations:
            self.errors.append(f"Foreign key violations detected: {violations}")
        else:
            self.validations_passed += 1
            print("  ✓ No foreign key violations")

    async def validate_indexes(self):
        """Validate that required indexes exist."""
        print("\nValidating indexes...")

        index_info = await self.db.get_index_usage()
        index_names = {idx["name"] for idx in index_info["indexes"]}

        required_indexes = {
            "idx_task_dependencies_prerequisite",
            "idx_task_dependencies_dependent",
            "idx_tasks_ready_priority",
            "idx_tasks_source_created",
            "idx_tasks_deadline",
            "idx_tasks_blocked",
        }

        missing_indexes = required_indexes - index_names
        if missing_indexes:
            self.errors.append(f"Missing indexes: {missing_indexes}")
        else:
            self.validations_passed += 1
            print("  ✓ All required indexes exist")
            print(f"  ℹ Total indexes: {len(index_names)}")

    async def validate_data_integrity(self):
        """Validate data integrity by inserting and retrieving test data."""
        print("\nValidating data integrity...")

        try:
            # Create test task with all new fields
            test_task = Task(
                prompt="Validation test task",
                source=TaskSource.AGENT_PLANNER,
                dependency_type=DependencyType.PARALLEL,
                calculated_priority=7.5,
                dependency_depth=2,
                estimated_duration_seconds=3600,
            )

            await self.db.insert_task(test_task)

            # Retrieve and verify
            retrieved = await self.db.get_task(test_task.id)
            if retrieved is None:
                self.errors.append("Failed to retrieve test task")
            elif (
                retrieved.source != TaskSource.AGENT_PLANNER
                or retrieved.calculated_priority != 7.5
                or retrieved.dependency_depth != 2
            ):
                self.errors.append("Retrieved task fields don't match inserted values")
            else:
                self.validations_passed += 1
                print("  ✓ Task insert/retrieve works correctly")

            # Create test dependency
            test_task2 = Task(prompt="Dependent test task")
            await self.db.insert_task(test_task2)

            test_dep = TaskDependency(
                dependent_task_id=test_task2.id,
                prerequisite_task_id=test_task.id,
                dependency_type=DependencyType.SEQUENTIAL,
            )

            await self.db.insert_task_dependency(test_dep)

            # Retrieve dependencies
            deps = await self.db.get_task_dependencies(test_task2.id)
            if len(deps) != 1:
                self.errors.append("Failed to retrieve task dependency")
            elif deps[0].prerequisite_task_id != test_task.id:
                self.errors.append("Retrieved dependency doesn't match inserted values")
            else:
                self.validations_passed += 1
                print("  ✓ Task dependency insert/retrieve works correctly")

            # Test dependency resolution
            await self.db.resolve_dependency(test_task.id)
            resolved_deps = await self.db.get_task_dependencies(test_task2.id)
            if resolved_deps[0].resolved_at is None:
                self.errors.append("Dependency resolution failed")
            else:
                self.validations_passed += 1
                print("  ✓ Dependency resolution works correctly")

        except Exception as e:
            self.errors.append(f"Data integrity validation failed: {e}")

    async def validate_enum_values(self):
        """Validate that all enum values are supported."""
        print("\nValidating enum values...")

        try:
            # Test all TaskStatus values
            for status in TaskStatus:
                task = Task(prompt=f"Status test {status.value}", status=status)
                await self.db.insert_task(task)
                retrieved = await self.db.get_task(task.id)
                if retrieved.status != status:
                    self.errors.append(f"TaskStatus {status.value} not persisted correctly")

            self.validations_passed += 1
            print("  ✓ All TaskStatus enum values supported")

            # Test all TaskSource values
            for source in TaskSource:
                task = Task(prompt=f"Source test {source.value}", source=source)
                await self.db.insert_task(task)
                retrieved = await self.db.get_task(task.id)
                if retrieved.source != source:
                    self.errors.append(f"TaskSource {source.value} not persisted correctly")

            self.validations_passed += 1
            print("  ✓ All TaskSource enum values supported")

            # Test all DependencyType values
            for dep_type in DependencyType:
                task = Task(prompt=f"DepType test {dep_type.value}", dependency_type=dep_type)
                await self.db.insert_task(task)
                retrieved = await self.db.get_task(task.id)
                if retrieved.dependency_type != dep_type:
                    self.errors.append(f"DependencyType {dep_type.value} not persisted correctly")

            self.validations_passed += 1
            print("  ✓ All DependencyType enum values supported")

        except Exception as e:
            self.errors.append(f"Enum validation failed: {e}")

    async def validate_query_performance(self):
        """Validate that critical queries use indexes."""
        print("\nValidating query performance...")

        try:
            # Test priority queue query
            query = """
                SELECT * FROM tasks
                WHERE status = 'ready'
                ORDER BY calculated_priority DESC, submitted_at ASC
                LIMIT 1
            """
            plan = await self.db.explain_query_plan(query, ())
            plan_text = " ".join(plan).lower()

            if "index" not in plan_text:
                self.warnings.append("Priority queue query may not be using an index efficiently")
            else:
                self.validations_passed += 1
                print("  ✓ Priority queue query uses index")

            # Test dependency resolution query
            from uuid import uuid4

            test_id = str(uuid4())
            query = """
                SELECT * FROM task_dependencies
                WHERE prerequisite_task_id = ? AND resolved_at IS NULL
            """
            plan = await self.db.explain_query_plan(query, (test_id,))
            plan_text = " ".join(plan).lower()

            if "index" not in plan_text:
                self.warnings.append(
                    "Dependency resolution query may not be using an index efficiently"
                )
            else:
                self.validations_passed += 1
                print("  ✓ Dependency resolution query uses index")

        except Exception as e:
            self.errors.append(f"Query performance validation failed: {e}")


async def main():
    """Main entry point."""
    # Use in-memory database for validation
    db_path = Path(":memory:")

    validator = Phase1Validator(db_path)
    success = await validator.run()

    sys.exit(0 if success else 1)


if __name__ == "__main__":
    asyncio.run(main())
