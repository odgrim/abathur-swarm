"""Pytest configuration and fixtures."""

import asyncio
import tempfile
from collections.abc import AsyncGenerator, Generator
from pathlib import Path
from typing import Any
from uuid import uuid4

import pytest
from abathur.infrastructure.database import Database
from abathur.services import DocumentIndexService, MemoryService, SessionService


# Register pytest helpers
class Helpers:
    """Helper functions for tests."""

    @staticmethod
    def run_async(coro: Any) -> Any:
        """Run an async coroutine in a sync test."""
        loop = asyncio.get_event_loop()
        return loop.run_until_complete(coro)


@pytest.fixture
def helpers() -> type[Helpers]:
    """Provide helper functions to tests."""
    return Helpers


# Add helpers to pytest namespace
pytest.helpers = Helpers  # type: ignore[attr-defined]


# Configure asyncio event loop for tests
@pytest.fixture(scope="session")
def event_loop() -> Generator[asyncio.AbstractEventLoop, None, None]:
    """Create an instance of the default event loop for the test session."""
    loop = asyncio.get_event_loop_policy().new_event_loop()
    yield loop
    loop.close()


# Database fixtures
@pytest.fixture
def temp_db_path() -> Generator[Path, None, None]:
    """Create temporary database file path."""
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as f:
        db_path = Path(f.name)
    yield db_path
    # Cleanup
    if db_path.exists():
        db_path.unlink()
    # Also cleanup WAL files
    wal_path = db_path.with_suffix(".db-wal")
    shm_path = db_path.with_suffix(".db-shm")
    if wal_path.exists():
        wal_path.unlink()
    if shm_path.exists():
        shm_path.unlink()


@pytest.fixture
async def memory_db() -> AsyncGenerator[Database, None]:
    """Create in-memory database for fast tests."""
    db = Database(Path(":memory:"))
    await db.initialize()
    yield db
    # Cleanup: close the shared connection for :memory: databases
    await db.close()


@pytest.fixture
async def file_db(temp_db_path: Path) -> AsyncGenerator[Database, None]:
    """Create file-based database for persistence tests."""
    db = Database(temp_db_path)
    await db.initialize()
    yield db
    # File-based databases close connections automatically, no cleanup needed


# Service fixtures
@pytest.fixture
async def session_service(memory_db: Database) -> SessionService:
    """Create SessionService with in-memory database."""
    return SessionService(memory_db)


@pytest.fixture
async def memory_service(memory_db: Database) -> MemoryService:
    """Create MemoryService with in-memory database."""
    return MemoryService(memory_db)


@pytest.fixture
async def document_service(memory_db: Database) -> DocumentIndexService:
    """Create DocumentIndexService with in-memory database."""
    return DocumentIndexService(memory_db)


# Test data fixtures
@pytest.fixture
def sample_session_id() -> str:
    """Generate unique session ID."""
    return str(uuid4())


@pytest.fixture
def sample_task_id() -> str:
    """Generate unique task ID."""
    return f"task:{uuid4()}"


@pytest.fixture
async def populated_db(memory_db: Database) -> Database:
    """Create database with sample data."""
    session_svc = SessionService(memory_db)
    memory_svc = MemoryService(memory_db)

    # Create sample sessions
    await session_svc.create_session("sess_1", "abathur", "alice", "project_1")
    await session_svc.create_session("sess_2", "abathur", "bob", "project_1")

    # Create sample memories
    await memory_svc.add_memory(
        namespace="user:alice:preferences",
        key="theme",
        value={"mode": "dark"},
        memory_type="semantic",
        created_by="sess_1",
        task_id="task_1",
    )
    await memory_svc.add_memory(
        namespace="user:alice:settings",
        key="language",
        value={"code": "python"},
        memory_type="semantic",
        created_by="sess_1",
        task_id="task_1",
    )

    return memory_db
