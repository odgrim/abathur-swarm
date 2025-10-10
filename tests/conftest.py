"""Pytest configuration and fixtures."""

import asyncio
from collections.abc import Generator
from typing import Any

import pytest


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
pytest.helpers = Helpers


# Configure asyncio event loop for tests
@pytest.fixture(scope="session")
def event_loop() -> Generator[asyncio.AbstractEventLoop, None, None]:
    """Create an instance of the default event loop for the test session."""
    loop = asyncio.get_event_loop_policy().new_event_loop()
    yield loop
    loop.close()
