# Abathur TUI Task Visualizer Guide

The Abathur TUI (Text User Interface) provides an interactive, real-time terminal interface for visualizing and managing task queues.

---

## Quick Start

Launch the TUI with:

```bash
abathur tui
```

The TUI will display your task queue in an interactive tree view with keyboard navigation.

---

## View Modes

The TUI supports multiple visualization modes, each optimized for different workflows:

### 1. Tree View (Default)
**Hierarchical parent-child relationships**

Shows tasks organized by their `parent_task_id` field, revealing the natural hierarchy of your task decomposition.

```
✓ Parent Task (9.0)
├── ◎ Child Task 1 (8.5)
└── ○ Child Task 2 (7.0)
```

**Best for:**
- Understanding task decomposition
- Viewing subtask structures
- Tracking work breakdown

**Switch to:** Press `t` or use filter menu

### 2. Dependency View
**Prerequisite relationships (DAG)**

Displays tasks organized by their dependency graph, showing which tasks must complete before others can start.

```
◉ Foundation Task (10.0)
├── ◎ Dependent Task A (8.0)
└── ◎ Dependent Task B (7.5)
    └── ○ Final Task (6.0)
```

**Best for:**
- Understanding execution order
- Identifying bottlenecks
- Planning parallel execution

**Switch to:** Press `d` or use filter menu

### 3. Timeline View
**Chronological ordering**

Tasks sorted by `submitted_at` timestamp, showing the temporal evolution of work.

```
2025-01-15 10:00 ✓ Early task
2025-01-15 11:30 ◉ Current task
2025-01-15 14:00 ○ Recent task
```

**Best for:**
- Tracking recent activity
- Understanding work progression over time
- Finding oldest pending tasks

**Switch to:** Press `i` (timeline) or use filter menu

### 4. Feature Branch View
**Grouped by feature branch**

Groups tasks by their `feature_branch` field, organizing work by feature or epic.

```
📁 feature/user-auth
├── ✓ Database schema
├── ◉ API endpoints
└── ○ Frontend integration

📁 feature/reporting
├── ○ Data aggregation
└── ○ Chart generation
```

**Best for:**
- Multi-feature development
- Feature progress tracking
- Branch-specific work organization

**Switch to:** Press `b` or use filter menu

### 5. Flat List View
**Simple sequential list**

All tasks in a flat list, sorted by priority.

```
✓ High priority task (10.0)
◉ Medium priority task (7.5)
○ Low priority task (3.0)
```

**Best for:**
- Quick scanning
- Priority-based review
- Simple linear workflows

**Switch to:** Press `l` or use filter menu

---

## Status Icons and Colors

Tasks are color-coded by status for quick visual scanning:

| Icon | Status | Color | Meaning |
|------|--------|-------|---------|
| ○ | PENDING | Blue | Waiting to start |
| ⊗ | BLOCKED | Yellow | Blocked by dependencies |
| ◎ | READY | Green | Ready for execution |
| ◉ | RUNNING | Magenta | Currently executing |
| ✓ | COMPLETED | Bright Green | Successfully finished |
| ✗ | FAILED | Red | Execution failed |
| ⊘ | CANCELLED | Dim Gray | Cancelled by user |

The icons provide accessibility for color-blind users or terminals with limited color support.

---

## Keyboard Controls

### Navigation
- `↑` / `k` - Move cursor up
- `↓` / `j` - Move cursor down
- `PageUp` - Scroll up one page
- `PageDown` - Scroll down one page
- `Home` / `g` - Jump to first task
- `End` / `G` - Jump to last task

### View Modes
- `t` - Tree view (hierarchical)
- `d` - Dependency view (DAG)
- `i` - Timeline view (chronological)
- `b` - Feature branch view
- `l` - Flat list view

### Filtering and Search
- `f` - Toggle filter modal
- `/` - Quick text search
- `Esc` - Clear current filter
- `Ctrl+R` - Refresh task data

### Task Actions
- `Enter` - Expand/collapse task details
- `Space` - Select task for bulk operations
- `Delete` - Cancel selected task(s)

### General
- `?` - Show help overlay
- `q` - Quit TUI

---

## Filtering Tasks

The TUI provides powerful multi-criteria filtering via the filter modal (press `f`).

### Available Filters

All filters use **AND logic** - tasks must match ALL active criteria:

#### 1. Status Filter
Select one or more task statuses to display:
- Pending
- Blocked
- Ready
- Running
- Completed
- Failed
- Cancelled

**Example:** Show only running OR ready tasks

#### 2. Agent Type Filter
Case-insensitive substring match on `agent_type` field.

**Examples:**
- `python` → Matches `python-backend-specialist`, `python-testing-specialist`
- `backend` → Matches `python-backend-specialist`, `go-backend-specialist`

#### 3. Feature Branch Filter
Case-insensitive substring match on `feature_branch` field.

**Examples:**
- `auth` → Matches `feature/user-auth`, `feature/oauth-integration`
- `feature/` → Matches all tasks with feature branches

#### 4. Text Search
Case-insensitive search across both `prompt` (description) and `summary` fields.

**Examples:**
- `database` → Finds tasks mentioning "database" anywhere in description or summary
- `bug fix` → Finds tasks about bug fixes

#### 5. Source Filter
Exact match on task source:
- `human` - Tasks submitted by users
- `agent_requirements` - Generated by requirements agent
- `agent_planner` - Generated by planning agent
- `agent_implementation` - Generated by implementation agent

### Filter Examples

**Show only failed Python backend tasks:**
- Status: Failed
- Agent Type: `python-backend`

**Find all authentication-related work:**
- Text Search: `auth`
- Feature Branch: `user-auth`

**Track current work in progress:**
- Status: Running, Ready
- Source: human

---

## Architecture

### Data Models

The TUI uses specialized models in `src/abathur/tui/models.py`:

#### TreeNode
Represents a single task within the hierarchical layout.

```python
class TreeNode(BaseModel):
    task_id: UUID              # Task identifier
    task: Task                 # Full task object
    children: list[UUID]       # Child task IDs
    level: int                 # Depth in hierarchy
    is_expanded: bool          # Visibility of children
    position: int              # Order within level
```

**See:** `src/abathur/tui/models.py:26`

#### TreeLayout
Complete tree structure ready for rendering.

```python
class TreeLayout(BaseModel):
    nodes: dict[UUID, TreeNode]  # All nodes in tree
    root_nodes: list[UUID]       # Top-level tasks
    max_depth: int               # Maximum tree depth
    total_nodes: int             # Total node count
```

**Key method:** `get_visible_nodes()` - Returns only visible nodes based on expand/collapse state

**See:** `src/abathur/tui/models.py:49`

#### FilterState
Multi-criteria filter configuration with AND logic.

```python
class FilterState(BaseModel):
    status_filter: set[TaskStatus] | None
    agent_type_filter: str | None
    feature_branch_filter: str | None
    text_search: str | None
    source_filter: TaskSource | None
```

**Key method:** `matches(task: Task) -> bool` - Returns True if task passes all active filters

**See:** `src/abathur/tui/models.py:148`

### Rendering Engine

The TUI uses Rich library for terminal rendering. See `src/abathur/tui/rendering/tree_renderer.py`:

#### TreeRenderer
Computes layout and generates Rich renderables.

```python
class TreeRenderer:
    def compute_layout(
        self,
        tasks: list[Task],
        dependency_graph: dict[UUID, list[UUID]],
    ) -> TreeLayout:
        """Compute hierarchical layout from tasks."""
```

**Algorithm:**
1. Group tasks by `dependency_depth` (hierarchical levels)
2. Sort within each level by `calculated_priority` (descending)
3. Build parent-child relationships using `parent_task_id`
4. Assign position numbers for rendering order

**See:** `src/abathur/tui/rendering/tree_renderer.py:154`

#### Color Mapping
Status-based color scheme for visual clarity:

```python
TASK_STATUS_COLORS: Dict[TaskStatus, str] = {
    TaskStatus.PENDING: "blue",
    TaskStatus.BLOCKED: "yellow",
    TaskStatus.READY: "green",
    TaskStatus.RUNNING: "magenta",
    TaskStatus.COMPLETED: "bright_green",
    TaskStatus.FAILED: "red",
    TaskStatus.CANCELLED: "dim",
}
```

**See:** `src/abathur/tui/rendering/tree_renderer.py:20`

### Data Service Layer

The `TaskDataService` provides real-time task data with caching and auto-refresh:

```python
class TaskDataService:
    async def get_filtered_tasks(
        self,
        filter_state: FilterState | None = None
    ) -> list[Task]:
        """Get tasks with optional filtering and caching."""
```

**Features:**
- TTL-based caching (configurable, default 5 seconds)
- Background auto-refresh
- Multi-criteria filtering
- Efficient database queries

**See:** `src/abathur/tui/services/task_data_service.py`

### Application Structure

The TUI follows Textual framework patterns:

```
AbathurTUIApp (src/abathur/tui/app.py)
├── MainScreen (src/abathur/tui/screens/main_screen.py)
│   ├── TaskTree Widget (hierarchical display)
│   ├── StatusBar Widget (queue statistics)
│   └── FilterModal Screen (filter configuration)
└── TaskDataService (data layer with caching)
```

---

## Performance Considerations

### Caching Strategy
The TUI uses a TTL-based cache to minimize database queries:

- **Default TTL:** 5 seconds
- **Auto-refresh:** Background task updates cache every TTL interval
- **Manual refresh:** `Ctrl+R` forces immediate cache invalidation

### Large Task Queues
For queues with 1000+ tasks:

1. **Use filters** to reduce visible set
2. **Flat list view** renders faster than tree views
3. **Increase TTL** if real-time updates aren't critical
4. **Feature branch view** groups tasks efficiently

### Database Queries
The TUI uses optimized queries:
- Indexed lookups on `status`, `agent_type`, `feature_branch`
- Batch loading of related tasks
- Lazy loading of task details

---

## Troubleshooting

### TUI doesn't show all tasks
**Cause:** Active filter is hiding tasks
**Solution:** Press `Esc` to clear filters, or press `f` to review filter settings

### Tree view shows flat list
**Cause:** Tasks don't have `parent_task_id` relationships
**Solution:** Switch to dependency view (`d`) or timeline view (`i`)

### Performance is slow with many tasks
**Solutions:**
- Apply status filter to show only active tasks
- Use flat list view instead of tree view
- Increase cache TTL in configuration
- Consider pruning completed/cancelled tasks

### Colors don't display correctly
**Cause:** Terminal doesn't support 256-color mode
**Solution:** The TUI falls back to basic colors automatically. Status icons provide visual distinction even without colors.

---

## Configuration

TUI settings can be configured in `.abathur/config.yaml`:

```yaml
tui:
  cache_ttl_seconds: 5        # Data cache lifetime
  auto_refresh: true          # Background cache updates
  default_view: "tree"        # Initial view mode
  max_visible_tasks: 1000     # Performance limit
  unicode_box_drawing: true   # Use Unicode vs ASCII
```

---

## Example Workflows

### Daily Task Review
1. Launch TUI: `abathur tui`
2. Switch to timeline view: Press `i`
3. Filter for today's tasks: Press `f`, set text search to current date
4. Review status icons for progress

### Feature Development
1. Switch to feature branch view: Press `b`
2. Filter by feature: Press `f`, set feature branch filter
3. Track task completion within feature
4. Identify blocked or failed tasks

### Agent Performance Monitoring
1. Filter by agent type: Press `f`, set agent type filter to specific agent
2. Review failed tasks: Add status filter for "Failed"
3. Analyze error patterns in task descriptions

### Dependency Analysis
1. Switch to dependency view: Press `d`
2. Identify critical path (longest chain)
3. Find parallel-executable tasks (same depth level)
4. Detect blocked tasks waiting on dependencies

---

## Advanced Features

### Expand/Collapse Navigation
In tree or dependency views:
- Press `Enter` on a task to toggle its children
- Collapsed tasks show `[+]` indicator
- Expanded tasks show `[-]` indicator

### Multi-Select Operations
(Planned for future releases)
- Press `Space` to select/deselect tasks
- Press `Delete` to cancel all selected tasks
- Press `Ctrl+A` to select all visible tasks

### Export Visualization
(Planned for future releases)
- Press `e` to export current view to GraphViz DOT format
- Press `Ctrl+S` to save filtered task list to CSV

---

## Code References

For implementation details, see:

- **TUI Models:** `src/abathur/tui/models.py`
- **Tree Rendering:** `src/abathur/tui/rendering/tree_renderer.py`
- **Main Application:** `src/abathur/tui/app.py`
- **Main Screen:** `src/abathur/tui/screens/main_screen.py`
- **Data Service:** `src/abathur/tui/services/task_data_service.py`
- **CLI Integration:** `src/abathur/cli/main.py` (search for `@app.command("tui")`)

---

## Contributing

To extend the TUI:

1. **Add new view mode:**
   - Add enum value to `ViewMode` in `src/abathur/tui/models.py:14`
   - Implement rendering logic in `TreeRenderer`
   - Add keyboard binding in `MainScreen`

2. **Add new filter:**
   - Add field to `FilterState` in `src/abathur/tui/models.py:148`
   - Update `matches()` method with new criteria
   - Add UI control in `FilterModal` screen

3. **Optimize performance:**
   - Review database queries in `TaskDataService`
   - Add indexes for new filter fields
   - Implement incremental rendering for large trees

---

## Future Enhancements

Planned features:
- Real-time task updates via WebSocket
- Task editing directly from TUI
- Graph export to various formats (PNG, SVG, PDF)
- Custom color schemes and themes
- Vim-style navigation modes
- Task dependency visualization with arrows
- Performance metrics dashboard
- Agent execution logs inline

---

**Version:** 0.1.0
**Last Updated:** 2025-01-22
