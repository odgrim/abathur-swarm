"""Unit tests for TaskDetailPanel widget.

Tests reactive properties, watch methods, rendering logic, and formatting
for all 28 Task fields.
"""

import pytest
from datetime import datetime, timezone, timedelta
from uuid import uuid4
from io import StringIO

from rich.text import Text
from rich.console import Console
from textual.app import App, ComposeResult

from abathur.domain.models import Task, TaskStatus, TaskSource, DependencyType
from abathur.tui.widgets.task_detail import TaskDetailPanel


def render_to_str(renderable) -> str:
    """Helper to render Rich renderables to string for testing."""
    console = Console(file=StringIO(), width=100, legacy_windows=False)
    console.print(renderable)
    return console.file.getvalue()


class TaskDetailTestApp(App):
    """Test app for TaskDetailPanel widget."""

    def compose(self) -> ComposeResult:
        yield TaskDetailPanel()


@pytest.fixture
def sample_task() -> Task:
    """Create a sample task with all fields populated."""
    task_id = uuid4()
    parent_id = uuid4()
    now = datetime.now(timezone.utc)

    return Task(
        id=task_id,
        summary="Implement TaskDetailPanel widget",
        prompt="Create a Textual widget that displays all 28 task fields with rich formatting",
        agent_type="python-textual-widget-specialist",
        priority=7,
        status=TaskStatus.RUNNING,
        source=TaskSource.AGENT_PLANNER,
        dependency_type=DependencyType.SEQUENTIAL,
        calculated_priority=8.5,
        dependency_depth=2,
        feature_branch="feature/tui-implementation",
        task_branch="task/tui-detail-panel",
        worktree_path="/path/to/worktree",
        submitted_at=now - timedelta(hours=2),
        started_at=now - timedelta(minutes=30),
        completed_at=None,
        last_updated_at=now,
        parent_task_id=parent_id,
        dependencies=[uuid4(), uuid4()],
        session_id="test-session-123",
        created_by="test-user",
        retry_count=1,
        max_retries=3,
        max_execution_timeout_seconds=3600,
        deadline=now + timedelta(days=1),
        estimated_duration_seconds=1800,
        input_data={"key1": "value1", "key2": "value2"},
        result_data=None,
        error_message=None,
    )


@pytest.fixture
def completed_task_with_result() -> Task:
    """Create a completed task with result data."""
    task_id = uuid4()
    now = datetime.now(timezone.utc)

    return Task(
        id=task_id,
        summary="Completed task",
        prompt="A task that completed successfully",
        status=TaskStatus.COMPLETED,
        completed_at=now,
        result_data={
            "status": "success",
            "files_created": ["widget.py", "test_widget.py"],
            "metrics": {"lines_of_code": 450, "test_coverage": 95.5},
        },
        submitted_at=now - timedelta(hours=1),
        started_at=now - timedelta(minutes=45),
        last_updated_at=now,
    )


@pytest.fixture
def failed_task_with_error() -> Task:
    """Create a failed task with error message."""
    task_id = uuid4()
    now = datetime.now(timezone.utc)

    return Task(
        id=task_id,
        summary="Failed task",
        prompt="A task that failed",
        status=TaskStatus.FAILED,
        completed_at=now,
        error_message="ImportError: No module named 'nonexistent_module'. "
        "This is a long error message that should be truncated in the display.",
        submitted_at=now - timedelta(hours=1),
        started_at=now - timedelta(minutes=30),
        last_updated_at=now,
    )


# Test widget initialization
def test_task_detail_panel_initialization():
    """Test TaskDetailPanel initializes with None state."""
    panel = TaskDetailPanel()
    assert panel.selected_task_id is None
    assert panel.task_data is None


# Test reactive property updates
@pytest.mark.asyncio
async def test_selected_task_id_reactive_update():
    """Test that updating selected_task_id triggers watch method."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)

        # Initially None
        assert panel.selected_task_id is None

        # Update to a UUID
        task_id = uuid4()
        panel.selected_task_id = task_id

        # Wait for reactive update to propagate
        await pilot.pause()

        # Verify update
        assert panel.selected_task_id == task_id


@pytest.mark.asyncio
async def test_task_data_reactive_update(sample_task):
    """Test that updating task_data triggers re-render."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)

        # Set task data
        panel.task_data = sample_task
        await pilot.pause()

        # Verify task data is set
        assert panel.task_data == sample_task
        assert panel.task_data.summary == "Implement TaskDetailPanel widget"


# Test empty state rendering
@pytest.mark.asyncio
async def test_render_empty_state():
    """Test rendering when no task is selected."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)

        # Verify empty state is rendered
        rendered = panel._render_empty_state()
        rendered_str = render_to_str(rendered)
        assert "No task selected" in rendered_str


# Test task rendering with all fields
@pytest.mark.asyncio
async def test_render_task_all_fields(sample_task):
    """Test rendering task with all 28 fields populated."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)

        # Set task data
        panel.task_data = sample_task
        await pilot.pause()

        # Render and verify sections are present
        rendered = panel._render_task(sample_task)
        rendered_str = render_to_str(rendered)

        # Verify all sections are present
        assert "Identification" in rendered_str
        assert "Status" in rendered_str
        assert "Priority" in rendered_str
        assert "Branches" in rendered_str
        assert "Timestamps" in rendered_str
        assert "Dependencies" in rendered_str
        assert "Execution" in rendered_str
        assert "Context" in rendered_str

        # Verify key field values
        assert str(sample_task.id) in rendered_str
        assert "Implement TaskDetailPanel widget" in rendered_str
        assert "python-textual-widget-specialist" in rendered_str
        assert "RUNNING" in rendered_str


@pytest.mark.asyncio
async def test_render_completed_task_with_result(completed_task_with_result):
    """Test rendering completed task shows result data section."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = completed_task_with_result
        await pilot.pause()

        rendered = panel._render_task(completed_task_with_result)
        rendered_str = render_to_str(rendered)

        # Verify Results section is present
        assert "Results" in rendered_str
        assert "Result Data:" in rendered_str

        # Verify JSON content is present
        assert "success" in rendered_str
        assert "files_created" in rendered_str


@pytest.mark.asyncio
async def test_render_failed_task_with_error(failed_task_with_error):
    """Test rendering failed task shows error message."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = failed_task_with_error
        await pilot.pause()

        rendered = panel._render_task(failed_task_with_error)
        rendered_str = render_to_str(rendered)

        # Verify error is displayed
        assert "Error:" in rendered_str
        assert "ImportError" in rendered_str


# Test formatting methods
def test_format_status():
    """Test status formatting with color coding."""
    panel = TaskDetailPanel()

    # Test each status
    for status in TaskStatus:
        formatted = panel._format_status(status)
        assert isinstance(formatted, Text)
        assert status.value.upper() in str(formatted)


def test_format_source():
    """Test task source formatting."""
    panel = TaskDetailPanel()

    for source in TaskSource:
        formatted = panel._format_source(source)
        assert isinstance(formatted, Text)
        assert source.value in str(formatted)


def test_format_optional_with_value():
    """Test formatting optional field with value."""
    panel = TaskDetailPanel()
    formatted = panel._format_optional("test-value")
    assert isinstance(formatted, Text)
    assert "test-value" in str(formatted)


def test_format_optional_without_value():
    """Test formatting optional field without value."""
    panel = TaskDetailPanel()
    formatted = panel._format_optional(None)
    assert isinstance(formatted, Text)
    assert "—" in str(formatted)


def test_format_datetime_with_value():
    """Test datetime formatting with value."""
    panel = TaskDetailPanel()
    now = datetime.now(timezone.utc)
    formatted = panel._format_datetime(now)

    assert isinstance(formatted, Text)
    # Should contain ISO format and relative time
    assert "UTC" in str(formatted)
    assert "ago" in str(formatted)


def test_format_datetime_without_value():
    """Test datetime formatting without value."""
    panel = TaskDetailPanel()
    formatted = panel._format_datetime(None)
    assert isinstance(formatted, Text)
    assert "—" in str(formatted)


def test_get_relative_time():
    """Test relative time calculation."""
    panel = TaskDetailPanel()
    now = datetime.now(timezone.utc)

    # Test various time deltas
    assert "ago" in panel._get_relative_time(now - timedelta(seconds=30))
    assert "ago" in panel._get_relative_time(now - timedelta(minutes=5))
    assert "ago" in panel._get_relative_time(now - timedelta(hours=2))
    assert "ago" in panel._get_relative_time(now - timedelta(days=3))


def test_format_timeout():
    """Test timeout formatting."""
    panel = TaskDetailPanel()

    # Test seconds
    assert "30s" in str(panel._format_timeout(30))

    # Test minutes
    assert "5m" in str(panel._format_timeout(300))

    # Test hours
    assert "2h" in str(panel._format_timeout(7200))


def test_format_dict_size():
    """Test dictionary size formatting."""
    panel = TaskDetailPanel()

    # Empty dict
    assert "empty" in str(panel._format_dict_size({}))

    # Single key
    assert "1 key" in str(panel._format_dict_size({"a": 1}))

    # Multiple keys
    assert "3 keys" in str(panel._format_dict_size({"a": 1, "b": 2, "c": 3}))


def test_format_json():
    """Test JSON formatting with syntax highlighting."""
    panel = TaskDetailPanel()
    data = {"key": "value", "number": 42, "nested": {"inner": True}}

    formatted = panel._format_json(data)

    # Should return Syntax object
    assert formatted is not None
    # Verify JSON content is present by rendering
    rendered_str = render_to_str(formatted)
    assert "key" in rendered_str


def test_format_error_truncation():
    """Test error message truncation for long errors."""
    panel = TaskDetailPanel()

    # Short error
    short_error = "Short error message"
    formatted = panel._format_error(short_error)
    assert short_error in str(formatted)

    # Long error (over 200 chars)
    long_error = "A" * 250
    formatted = panel._format_error(long_error)
    result_str = str(formatted)
    # Should be truncated with ellipsis
    assert len(result_str) < 250
    assert "..." in result_str


def test_format_summary_with_value():
    """Test summary formatting when summary is provided."""
    panel = TaskDetailPanel()
    summary = "Test summary"
    prompt = "Test prompt"

    formatted = panel._format_summary(summary, prompt)
    assert "Test summary" in str(formatted)


def test_format_summary_without_value():
    """Test summary formatting falls back to prompt preview."""
    panel = TaskDetailPanel()
    summary = None
    prompt = "This is a very long prompt that should be truncated to 60 characters"

    formatted = panel._format_summary(summary, prompt)
    result_str = str(formatted)

    # Should show truncated prompt
    assert len(result_str) <= 63  # 60 chars + "..."
    assert "..." in result_str or len(prompt) <= 60


def test_status_icons():
    """Test status icon mapping."""
    panel = TaskDetailPanel()

    # Verify each status has an icon
    for status in TaskStatus:
        icon = panel._get_status_icon(status)
        assert isinstance(icon, str)
        assert len(icon) > 0


# Test CSS styling
def test_default_css_defined():
    """Test that DEFAULT_CSS is defined."""
    assert hasattr(TaskDetailPanel, "DEFAULT_CSS")
    assert isinstance(TaskDetailPanel.DEFAULT_CSS, str)
    assert "TaskDetailPanel" in TaskDetailPanel.DEFAULT_CSS


# Test status color mapping
def test_status_colors_complete():
    """Test that all statuses have color mappings."""
    for status in TaskStatus:
        assert status in TaskDetailPanel.STATUS_COLORS
        assert isinstance(TaskDetailPanel.STATUS_COLORS[status], str)


# Integration test: Full widget lifecycle
@pytest.mark.asyncio
async def test_widget_full_lifecycle(sample_task):
    """Test complete widget lifecycle: init -> select task -> render -> update."""
    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)

        # 1. Initial state
        assert panel.selected_task_id is None
        assert panel.task_data is None

        # 2. Select task
        panel.selected_task_id = sample_task.id
        await pilot.pause()

        # 3. Set task data (simulating fetch)
        panel.task_data = sample_task
        await pilot.pause()

        # 4. Verify rendering
        assert panel.task_data == sample_task

        # 5. Update to different task
        new_task = Task(
            id=uuid4(),
            summary="Different task",
            prompt="Different prompt",
            status=TaskStatus.COMPLETED,
        )
        panel.task_data = new_task
        await pilot.pause()

        assert panel.task_data == new_task

        # 6. Clear selection
        panel.task_data = None
        await pilot.pause()

        assert panel.task_data is None


# Test handling of minimal task (only required fields)
@pytest.mark.asyncio
async def test_render_minimal_task():
    """Test rendering task with only required fields (all optionals are None)."""
    minimal_task = Task(
        id=uuid4(),
        prompt="Minimal task prompt",
        # All other fields use defaults
    )

    app = TaskDetailTestApp()
    async with app.run_test() as pilot:
        panel = app.query_one(TaskDetailPanel)
        panel.task_data = minimal_task
        await pilot.pause()

        rendered = panel._render_task(minimal_task)
        rendered_str = render_to_str(rendered)

        # Should render without errors
        assert "Identification" in rendered_str
        assert "Minimal task prompt" in rendered_str

        # Optional fields should show as "—"
        assert "—" in rendered_str
