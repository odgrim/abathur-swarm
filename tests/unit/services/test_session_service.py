"""Unit tests for SessionService."""

from datetime import datetime, timezone

import pytest
from abathur.services import SessionService


class TestSessionService:
    """Test SessionService CRUD operations and session management."""

    @pytest.mark.asyncio
    async def test_create_session_success(
        self, session_service: SessionService, sample_session_id: str
    ) -> None:
        """Test successful session creation."""
        await session_service.create_session(
            session_id=sample_session_id, app_name="abathur", user_id="alice"
        )

        session = await session_service.get_session(sample_session_id)
        assert session is not None
        assert session["id"] == sample_session_id
        assert session["app_name"] == "abathur"
        assert session["user_id"] == "alice"
        assert session["status"] == "created"
        assert session["events"] == []
        assert session["state"] == {}

    @pytest.mark.asyncio
    async def test_create_session_with_initial_state(self, session_service: SessionService) -> None:
        """Test session creation with initial state."""
        initial_state = {"user:alice:theme": "dark", "user:alice:language": "python"}

        await session_service.create_session(
            session_id="test_session",
            app_name="abathur",
            user_id="alice",
            initial_state=initial_state,
        )

        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["state"] == initial_state

    @pytest.mark.asyncio
    async def test_create_session_with_project_id(self, session_service: SessionService) -> None:
        """Test session creation with project_id."""
        await session_service.create_session(
            session_id="project_session",
            app_name="abathur",
            user_id="alice",
            project_id="schema_redesign",
        )

        session = await session_service.get_session("project_session")
        assert session is not None
        assert session["project_id"] == "schema_redesign"

    @pytest.mark.asyncio
    async def test_create_duplicate_session_raises_error(
        self, session_service: SessionService
    ) -> None:
        """Test that duplicate session_id raises ValueError."""
        await session_service.create_session("test_session", "app", "user")

        with pytest.raises(ValueError, match="already exists"):
            await session_service.create_session("test_session", "app", "user")

    @pytest.mark.asyncio
    async def test_get_nonexistent_session_returns_none(
        self, session_service: SessionService
    ) -> None:
        """Test getting nonexistent session returns None."""
        session = await session_service.get_session("nonexistent")
        assert session is None

    @pytest.mark.asyncio
    async def test_append_event_with_state_delta(self, session_service: SessionService) -> None:
        """Test event appending with state delta merge."""
        await session_service.create_session("test_session", "app", "user")

        event = {
            "event_id": "evt_001",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "event_type": "message",
            "actor": "user",
            "content": {"message": "Hello"},
            "is_final_response": False,
        }
        state_delta = {"session:current_task": "greeting", "session:msg_count": 1}

        await session_service.append_event("test_session", event, state_delta)

        session = await session_service.get_session("test_session")
        assert session is not None
        assert len(session["events"]) == 1
        assert session["events"][0]["event_type"] == "message"
        assert session["state"]["session:current_task"] == "greeting"
        assert session["state"]["session:msg_count"] == 1

    @pytest.mark.asyncio
    async def test_append_event_without_state_delta(self, session_service: SessionService) -> None:
        """Test appending event without state changes."""
        await session_service.create_session("test_session", "app", "user")

        event = {
            "event_id": "evt_001",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "event_type": "action",
            "actor": "agent:test",
            "content": {"action": "process"},
            "is_final_response": False,
        }

        await session_service.append_event("test_session", event)

        session = await session_service.get_session("test_session")
        assert session is not None
        assert len(session["events"]) == 1
        assert session["state"] == {}  # No state changes

    @pytest.mark.asyncio
    async def test_append_multiple_events(self, session_service: SessionService) -> None:
        """Test appending multiple events maintains order."""
        await session_service.create_session("test_session", "app", "user")

        # Append 3 events
        for i in range(3):
            event = {
                "event_id": f"evt_{i:03d}",
                "timestamp": datetime.now(timezone.utc).isoformat(),
                "event_type": "message",
                "actor": "user",
                "content": {"message": f"Message {i}"},
                "is_final_response": False,
            }
            await session_service.append_event("test_session", event)

        session = await session_service.get_session("test_session")
        assert session is not None
        assert len(session["events"]) == 3
        assert session["events"][0]["event_id"] == "evt_000"
        assert session["events"][2]["event_id"] == "evt_002"

    @pytest.mark.asyncio
    async def test_append_event_to_nonexistent_session_raises_error(
        self, session_service: SessionService
    ) -> None:
        """Test appending event to nonexistent session raises ValueError."""
        event = {
            "event_id": "evt_001",
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "event_type": "message",
            "actor": "user",
            "content": {},
            "is_final_response": False,
        }

        with pytest.raises(ValueError, match="not found"):
            await session_service.append_event("nonexistent", event)

    @pytest.mark.asyncio
    async def test_update_status_to_active(self, session_service: SessionService) -> None:
        """Test updating session status to active."""
        await session_service.create_session("test_session", "app", "user")

        await session_service.update_status("test_session", "active")

        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "active"

    @pytest.mark.asyncio
    async def test_update_status_to_terminated(self, session_service: SessionService) -> None:
        """Test updating session status to terminated sets terminated_at."""
        await session_service.create_session("test_session", "app", "user")

        await session_service.update_status("test_session", "terminated")

        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "terminated"
        assert session["terminated_at"] is not None

    @pytest.mark.asyncio
    async def test_status_lifecycle_transitions(self, session_service: SessionService) -> None:
        """Test complete status lifecycle transitions."""
        await session_service.create_session("test_session", "app", "user")

        # created → active
        await session_service.update_status("test_session", "active")
        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "active"

        # active → paused
        await session_service.update_status("test_session", "paused")
        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "paused"

        # paused → active
        await session_service.update_status("test_session", "active")
        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "active"

        # active → terminated
        await session_service.update_status("test_session", "terminated")
        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "terminated"

        # terminated → archived
        await session_service.update_status("test_session", "archived")
        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "archived"

    @pytest.mark.asyncio
    async def test_invalid_status_raises_error(self, session_service: SessionService) -> None:
        """Test that invalid status raises ValueError."""
        await session_service.create_session("test_session", "app", "user")

        with pytest.raises(ValueError, match="Invalid status"):
            await session_service.update_status("test_session", "invalid_status")

    @pytest.mark.asyncio
    async def test_terminate_session_convenience_method(
        self, session_service: SessionService
    ) -> None:
        """Test terminate_session convenience method."""
        await session_service.create_session("test_session", "app", "user")

        await session_service.terminate_session("test_session")

        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["status"] == "terminated"
        assert session["terminated_at"] is not None

    @pytest.mark.asyncio
    async def test_get_state(self, session_service: SessionService) -> None:
        """Test getting specific state value."""
        await session_service.create_session(
            "test_session", "app", "user", initial_state={"user:theme": "dark", "session:count": 5}
        )

        theme = await session_service.get_state("test_session", "user:theme")
        assert theme == "dark"

        count = await session_service.get_state("test_session", "session:count")
        assert count == 5

    @pytest.mark.asyncio
    async def test_get_state_nonexistent_key_returns_none(
        self, session_service: SessionService
    ) -> None:
        """Test getting nonexistent state key returns None."""
        await session_service.create_session("test_session", "app", "user")

        value = await session_service.get_state("test_session", "nonexistent")
        assert value is None

    @pytest.mark.asyncio
    async def test_set_state(self, session_service: SessionService) -> None:
        """Test setting specific state value."""
        await session_service.create_session("test_session", "app", "user")

        await session_service.set_state("test_session", "user:theme", "light")

        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["state"]["user:theme"] == "light"

    @pytest.mark.asyncio
    async def test_set_state_updates_existing_key(self, session_service: SessionService) -> None:
        """Test updating existing state key."""
        await session_service.create_session(
            "test_session", "app", "user", initial_state={"user:theme": "dark"}
        )

        await session_service.set_state("test_session", "user:theme", "light")

        session = await session_service.get_session("test_session")
        assert session is not None
        assert session["state"]["user:theme"] == "light"

    @pytest.mark.asyncio
    async def test_set_state_on_nonexistent_session_raises_error(
        self, session_service: SessionService
    ) -> None:
        """Test setting state on nonexistent session raises ValueError."""
        with pytest.raises(ValueError, match="not found"):
            await session_service.set_state("nonexistent", "key", "value")

    @pytest.mark.asyncio
    async def test_list_sessions_no_filters(self, session_service: SessionService) -> None:
        """Test listing sessions without filters."""
        # Create 3 sessions
        await session_service.create_session("sess_1", "app", "alice")
        await session_service.create_session("sess_2", "app", "bob")
        await session_service.create_session("sess_3", "app", "charlie")

        sessions = await session_service.list_sessions()
        assert len(sessions) >= 3

    @pytest.mark.asyncio
    async def test_list_sessions_filter_by_project(self, session_service: SessionService) -> None:
        """Test listing sessions filtered by project_id."""
        await session_service.create_session("sess_1", "app", "alice", project_id="project_1")
        await session_service.create_session("sess_2", "app", "bob", project_id="project_1")
        await session_service.create_session("sess_3", "app", "charlie", project_id="project_2")

        project1_sessions = await session_service.list_sessions(project_id="project_1")
        assert len(project1_sessions) == 2

        project2_sessions = await session_service.list_sessions(project_id="project_2")
        assert len(project2_sessions) == 1

    @pytest.mark.asyncio
    async def test_list_sessions_filter_by_status(self, session_service: SessionService) -> None:
        """Test listing sessions filtered by status."""
        await session_service.create_session("sess_1", "app", "alice")
        await session_service.create_session("sess_2", "app", "bob")
        await session_service.update_status("sess_1", "active")

        active_sessions = await session_service.list_sessions(status="active")
        assert len(active_sessions) == 1
        assert active_sessions[0]["id"] == "sess_1"

        created_sessions = await session_service.list_sessions(status="created")
        assert len(created_sessions) >= 1

    @pytest.mark.asyncio
    async def test_list_sessions_with_limit(self, session_service: SessionService) -> None:
        """Test list_sessions respects limit parameter."""
        # Create 10 sessions
        for i in range(10):
            await session_service.create_session(f"sess_{i}", "app", f"user_{i}")

        sessions = await session_service.list_sessions(limit=5)
        assert len(sessions) == 5

    @pytest.mark.asyncio
    async def test_state_namespace_isolation(self, session_service: SessionService) -> None:
        """Test state namespace isolation (user:, session:, app:, project:)."""
        await session_service.create_session("test_session", "app", "user")

        # Set state at different namespace levels
        await session_service.set_state("test_session", "user:alice:theme", "dark")
        await session_service.set_state("test_session", "session:current_task", "task_1")
        await session_service.set_state("test_session", "app:log_level", "INFO")
        await session_service.set_state("test_session", "project:max_agents", 50)

        session = await session_service.get_session("test_session")
        assert session is not None

        assert session["state"]["user:alice:theme"] == "dark"
        assert session["state"]["session:current_task"] == "task_1"
        assert session["state"]["app:log_level"] == "INFO"
        assert session["state"]["project:max_agents"] == 50
