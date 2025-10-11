"""Session management service for conversation tracking."""

import json
from typing import TYPE_CHECKING, Any

import aiosqlite

if TYPE_CHECKING:
    from abathur.infrastructure.database import Database


class SessionService:
    """Service for managing conversation sessions with events and state.

    Provides CRUD operations for sessions, event appending, and state management
    with hierarchical namespace support (session:, temp:, user:, app:, project:).
    """

    def __init__(self, db: "Database") -> None:
        """Initialize session service.

        Args:
            db: Database instance for storage operations
        """
        self.db = db

    async def create_session(
        self,
        session_id: str,
        app_name: str,
        user_id: str,
        project_id: str | None = None,
        initial_state: dict[str, Any] | None = None,
    ) -> None:
        """Create new session with optional initial state.

        Args:
            session_id: UUID or unique session identifier
            app_name: Application context (e.g., "abathur")
            user_id: User identifier
            project_id: Optional project association for cross-agent collaboration
            initial_state: Optional initial state dictionary with namespace prefixes

        Raises:
            ValueError: If session_id already exists

        Example:
            >>> await session_service.create_session(
            ...     session_id="abc123",
            ...     app_name="abathur",
            ...     user_id="alice",
            ...     project_id="schema_redesign",
            ...     initial_state={"user:alice:theme": "dark"}
            ... )
        """
        state_json = json.dumps(initial_state or {})

        async with self.db._get_connection() as conn:
            try:
                await conn.execute(
                    """
                    INSERT INTO sessions (id, app_name, user_id, project_id, status, events, state)
                    VALUES (?, ?, ?, ?, 'created', '[]', ?)
                    """,
                    (session_id, app_name, user_id, project_id, state_json),
                )
                await conn.commit()
            except aiosqlite.IntegrityError as e:
                await conn.rollback()
                raise ValueError(f"Session {session_id} already exists") from e
            except Exception:
                await conn.rollback()
                raise

    async def get_session(self, session_id: str) -> dict[str, Any] | None:
        """Retrieve session by ID with parsed JSON fields.

        Args:
            session_id: Session identifier

        Returns:
            Dictionary with session fields (events and state parsed as dicts/lists),
            or None if session not found

        Example:
            >>> session = await session_service.get_session("abc123")
            >>> print(session['status'])  # 'active'
            >>> print(session['events'][0]['event_type'])  # 'message'
        """
        async with self.db._get_connection() as conn:
            cursor = await conn.execute("SELECT * FROM sessions WHERE id = ?", (session_id,))
            row = await cursor.fetchone()

            if row is None:
                return None

            # Convert to dict and parse JSON fields
            session = dict(row)
            session["events"] = json.loads(session["events"])
            session["state"] = json.loads(session["state"])
            session["metadata"] = json.loads(session.get("metadata", "{}"))

            return session

    async def list_sessions(
        self, project_id: str | None = None, status: str | None = None, limit: int = 50
    ) -> list[dict[str, Any]]:
        """List sessions with optional filters.

        Args:
            project_id: Optional project ID filter
            status: Optional status filter
            limit: Maximum results to return

        Returns:
            List of session dictionaries
        """
        async with self.db._get_connection() as conn:
            query = "SELECT * FROM sessions WHERE 1=1"
            params: list[str | int] = []

            if project_id:
                query += " AND project_id = ?"
                params.append(project_id)

            if status:
                query += " AND status = ?"
                params.append(status)

            query += " ORDER BY created_at DESC LIMIT ?"
            params.append(limit)

            cursor = await conn.execute(query, params)
            rows = await cursor.fetchall()

            sessions = []
            for row in rows:
                session = dict(row)
                session["events"] = json.loads(session["events"])
                session["state"] = json.loads(session["state"])
                session["metadata"] = json.loads(session.get("metadata", "{}"))
                sessions.append(session)

            return sessions

    async def append_event(
        self,
        session_id: str,
        event: dict[str, Any],
        state_delta: dict[str, Any] | None = None,
    ) -> None:
        """Append event to session with optional state update.

        Args:
            session_id: Session identifier
            event: Event dictionary with keys:
                - event_id: str (unique event identifier)
                - timestamp: str (ISO 8601 timestamp)
                - event_type: str (message|action|tool_call|reflection)
                - actor: str (user|agent:<agent_id>|system)
                - content: dict (event-specific data)
                - is_final_response: bool
            state_delta: Optional state changes to merge into session state

        Raises:
            ValueError: If session not found

        Example:
            >>> await session_service.append_event(
            ...     session_id="abc123",
            ...     event={
            ...         "event_id": "evt_001",
            ...         "timestamp": "2025-10-10T10:00:00Z",
            ...         "event_type": "message",
            ...         "actor": "user",
            ...         "content": {"message": "Design the schema"},
            ...         "is_final_response": False
            ...     },
            ...     state_delta={"session:abc123:current_task": "schema_design"}
            ... )
        """
        async with self.db._get_connection() as conn:
            try:
                # Read current events and state
                cursor = await conn.execute(
                    "SELECT events, state FROM sessions WHERE id = ?", (session_id,)
                )
                row = await cursor.fetchone()

                if row is None:
                    raise ValueError(f"Session {session_id} not found")

                # Parse JSON
                events = json.loads(row["events"])
                state = json.loads(row["state"])

                # Append event
                events.append(event)

                # Merge state delta if provided
                if state_delta:
                    state.update(state_delta)

                # Update session
                await conn.execute(
                    """
                    UPDATE sessions
                    SET events = ?, state = ?, last_update_time = CURRENT_TIMESTAMP
                    WHERE id = ?
                    """,
                    (json.dumps(events), json.dumps(state), session_id),
                )
                await conn.commit()
            except Exception:
                await conn.rollback()
                raise

    async def update_status(self, session_id: str, status: str) -> None:
        """Update session lifecycle status.

        Args:
            session_id: Session identifier
            status: New status (created|active|paused|terminated|archived)

        Raises:
            ValueError: If status is invalid or session not found

        Example:
            >>> await session_service.update_status("abc123", "active")
        """
        valid_statuses = {"created", "active", "paused", "terminated", "archived"}
        if status not in valid_statuses:
            raise ValueError(f"Invalid status: {status}. Must be one of {valid_statuses}")

        async with self.db._get_connection() as conn:
            try:
                if status == "terminated":
                    await conn.execute(
                        """
                        UPDATE sessions
                        SET status = ?, terminated_at = CURRENT_TIMESTAMP, last_update_time = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (status, session_id),
                    )
                else:
                    await conn.execute(
                        """
                        UPDATE sessions
                        SET status = ?, last_update_time = CURRENT_TIMESTAMP
                        WHERE id = ?
                        """,
                        (status, session_id),
                    )
                await conn.commit()
            except Exception:
                await conn.rollback()
                raise

    async def terminate_session(self, session_id: str) -> None:
        """Terminate session (convenience method).

        Args:
            session_id: Session identifier
        """
        await self.update_status(session_id, "terminated")

    async def get_state(self, session_id: str, key: str) -> Any | None:
        """Get specific state value from session.

        Args:
            session_id: Session identifier
            key: State key (with namespace prefix, e.g., "user:alice:theme")

        Returns:
            State value if key exists, None otherwise

        Example:
            >>> theme = await session_service.get_state("abc123", "user:alice:theme")
            >>> print(theme)  # "dark"
        """
        session = await self.get_session(session_id)
        if session is None:
            return None

        return session["state"].get(key)

    async def set_state(self, session_id: str, key: str, value: Any) -> None:
        """Set specific state value in session.

        Args:
            session_id: Session identifier
            key: State key (with namespace prefix)
            value: State value (must be JSON-serializable)

        Example:
            >>> await session_service.set_state("abc123", "user:alice:theme", "dark")
        """
        async with self.db._get_connection() as conn:
            try:
                cursor = await conn.execute(
                    "SELECT state FROM sessions WHERE id = ?", (session_id,)
                )
                row = await cursor.fetchone()

                if row is None:
                    raise ValueError(f"Session {session_id} not found")

                state = json.loads(row["state"])
                state[key] = value

                await conn.execute(
                    """
                    UPDATE sessions
                    SET state = ?, last_update_time = CURRENT_TIMESTAMP
                    WHERE id = ?
                    """,
                    (json.dumps(state), session_id),
                )
                await conn.commit()
            except Exception:
                await conn.rollback()
                raise
