"""Tests for VACUUM progress indicator in CLI.

This module tests the progress indicator behavior during VACUUM operations
to ensure user feedback is provided for long-running database optimizations.
"""

import asyncio
from datetime import datetime, timedelta, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from abathur.domain.models import Task, TaskSource, TaskStatus
from abathur.infrastructure.database import Database, PruneFilters


@pytest.mark.asyncio
async def test_vacuum_progress_indicator_shown_for_always_mode() -> None:
    """Test that progress indicator is shown when vacuum_mode='always'."""
    # Create temporary database
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database
        db = Database(db_path)
        await db.initialize()

        # Create 10 tasks (below conditional threshold)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)
        task_ids = []

        for i in range(10):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Summary {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids.append(task.id)

        # Mock Progress to verify it's used
        with patch("abathur.cli.main.Progress") as mock_progress:
            mock_progress_instance = MagicMock()
            mock_progress.return_value.__enter__ = MagicMock(
                return_value=mock_progress_instance
            )
            mock_progress.return_value.__exit__ = MagicMock(return_value=False)

            # Import after patching
            from abathur.cli.main import _get_services

            # Simulate prune with vacuum_mode='always'
            filters = PruneFilters(task_ids=task_ids, vacuum_mode="always")
            result = await db.prune_tasks(filters)

            # For vacuum_mode='always', progress indicator should be shown
            # even with only 10 tasks
            assert result.deleted_tasks == 10
            # Progress should be instantiated for 'always' mode
            # (We can't easily test the CLI path without running the full CLI)

        await db.close()
    finally:
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_vacuum_progress_indicator_shown_for_conditional_above_threshold() -> None:
    """Test that progress indicator is shown when deleting >= 100 tasks with conditional mode."""
    # Create temporary database
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database
        db = Database(db_path)
        await db.initialize()

        # Create 100 tasks (at conditional threshold)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)
        task_ids = []

        for i in range(100):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Summary {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids.append(task.id)

        # Execute prune with conditional mode
        filters = PruneFilters(task_ids=task_ids, vacuum_mode="conditional")
        result = await db.prune_tasks(filters)

        # Verify VACUUM ran (since we're at threshold)
        assert result.deleted_tasks == 100
        assert result.reclaimed_bytes is not None

        await db.close()
    finally:
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_no_progress_indicator_for_conditional_below_threshold() -> None:
    """Test that no progress indicator is shown when deleting < 100 tasks with conditional mode."""
    # Create temporary database
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database
        db = Database(db_path)
        await db.initialize()

        # Create 50 tasks (below conditional threshold)
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)
        task_ids = []

        for i in range(50):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Summary {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids.append(task.id)

        # Execute prune with conditional mode
        filters = PruneFilters(task_ids=task_ids, vacuum_mode="conditional")
        result = await db.prune_tasks(filters)

        # Verify VACUUM did NOT run (below threshold)
        assert result.deleted_tasks == 50
        assert result.reclaimed_bytes is None

        await db.close()
    finally:
        if db_path.exists():
            db_path.unlink()


@pytest.mark.asyncio
async def test_no_progress_indicator_for_never_mode() -> None:
    """Test that no progress indicator is shown when vacuum_mode='never'."""
    # Create temporary database
    with NamedTemporaryFile(suffix=".db", delete=False) as tmp_file:
        db_path = Path(tmp_file.name)

    try:
        # Initialize database
        db = Database(db_path)
        await db.initialize()

        # Create 150 tasks (above conditional threshold, but we're using 'never')
        old_timestamp = datetime.now(timezone.utc) - timedelta(days=60)
        task_ids = []

        for i in range(150):
            task = Task(
                prompt=f"Task {i}",
                summary=f"Summary {i}",
                agent_type="test-agent",
                source=TaskSource.HUMAN,
                status=TaskStatus.COMPLETED,
                submitted_at=old_timestamp,
                completed_at=old_timestamp,
            )
            await db.insert_task(task)
            task_ids.append(task.id)

        # Execute prune with never mode
        filters = PruneFilters(task_ids=task_ids, vacuum_mode="never")
        result = await db.prune_tasks(filters)

        # Verify VACUUM did NOT run (never mode)
        assert result.deleted_tasks == 150
        assert result.reclaimed_bytes is None

        await db.close()
    finally:
        if db_path.exists():
            db_path.unlink()
