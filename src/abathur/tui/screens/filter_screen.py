"""FilterScreen modal for configuring task filters.

This modal screen provides a UI for setting multi-criteria filters including:
- Status checkboxes (all 7 TaskStatus values)
- Agent type text input
- Feature branch text input
- Text search input
- Source filter checkboxes
"""

from textual.app import ComposeResult
from textual.containers import Container, Vertical, Horizontal, ScrollableContainer
from textual.screen import ModalScreen
from textual.widgets import Static, Input, Checkbox, Button
from textual.binding import Binding
from textual.message import Message

from abathur.domain.models import TaskStatus, TaskSource
from abathur.tui.models import FilterState


class FilterApplied(Message):
    """Custom event emitted when filters are applied.

    Attributes:
        filter_state: The FilterState object with applied filters
    """

    def __init__(self, filter_state: FilterState) -> None:
        """Initialize FilterApplied event.

        Args:
            filter_state: FilterState with applied filter criteria
        """
        super().__init__()
        self.filter_state = filter_state


class FilterScreen(ModalScreen[FilterState | None]):
    """Modal screen for configuring task filters with multi-criteria support.

    Displays:
    - Status checkboxes (all 7 TaskStatus values)
    - Source checkboxes (all 4 TaskSource values)
    - Agent type text input
    - Feature branch text input
    - Text search input
    - Apply, Clear, and Cancel buttons

    Returns FilterState when applied, None when cancelled.

    Keybindings:
    - escape: Cancel and close
    - ctrl+s: Apply filters
    - ctrl+r: Clear all filters
    """

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=True),
        Binding("ctrl+s", "apply", "Apply Filters", show=True),
        Binding("ctrl+r", "reset", "Clear All", show=True),
    ]

    CSS = """
    #filter-modal {
        width: 90;
        height: auto;
        max-height: 80%;
        padding: 1 2;
        border: thick $primary;
        background: $surface;
    }

    #filter-title {
        text-align: center;
        text-style: bold;
        color: $accent;
        margin-bottom: 1;
    }

    .filter-section {
        margin-bottom: 1;
        height: auto;
    }

    .filter-label {
        text-style: bold;
        margin-bottom: 1;
    }

    .checkbox-row {
        height: auto;
        align: left top;
    }

    .checkbox-row Checkbox {
        margin-right: 2;
    }

    .filter-input {
        width: 100%;
        margin-bottom: 1;
    }

    #button-row {
        height: auto;
        align: center middle;
        margin-top: 2;
    }

    #button-row Button {
        margin: 0 1;
    }
    """

    def __init__(self, current_filter: FilterState | None = None):
        """Initialize filter screen with current filter state.

        Args:
            current_filter: Existing filter state to pre-populate form
        """
        super().__init__()
        self.current_filter = current_filter or FilterState()

    def compose(self) -> ComposeResult:
        """Build the filter form UI with all input widgets."""
        with Container(id="filter-modal"):
            yield Static("Filter Tasks", id="filter-title")

            with ScrollableContainer():
                # Status checkboxes section
                with Vertical(classes="filter-section"):
                    yield Static("Status:", classes="filter-label")
                    with Horizontal(classes="checkbox-row"):
                        for status in TaskStatus:
                            is_checked = (
                                self.current_filter.status_filter is not None
                                and status in self.current_filter.status_filter
                            )
                            yield Checkbox(
                                status.value.title(),
                                value=is_checked,
                                id=f"status-{status.value}",
                            )

                # Source checkboxes section
                with Vertical(classes="filter-section"):
                    yield Static("Source:", classes="filter-label")
                    with Horizontal(classes="checkbox-row"):
                        for source in TaskSource:
                            is_checked = self.current_filter.source_filter == source
                            yield Checkbox(
                                source.value.replace("_", " ").title(),
                                value=is_checked,
                                id=f"source-{source.value}",
                            )

                # Agent type input
                with Vertical(classes="filter-section"):
                    yield Static("Agent Type (substring match):", classes="filter-label")
                    yield Input(
                        value=self.current_filter.agent_type_filter or "",
                        placeholder="e.g., python, backend, specialist",
                        id="agent-type-input",
                        classes="filter-input",
                    )

                # Feature branch input
                with Vertical(classes="filter-section"):
                    yield Static(
                        "Feature Branch (substring match):", classes="filter-label"
                    )
                    yield Input(
                        value=self.current_filter.feature_branch_filter or "",
                        placeholder="e.g., feature/filters, tui",
                        id="branch-input",
                        classes="filter-input",
                    )

                # Text search input
                with Vertical(classes="filter-section"):
                    yield Static(
                        "Text Search (in description/summary):", classes="filter-label"
                    )
                    yield Input(
                        value=self.current_filter.text_search or "",
                        placeholder="Search in task description and summary",
                        id="search-input",
                        classes="filter-input",
                    )

            # Action buttons
            with Horizontal(id="button-row"):
                yield Button("Apply", variant="primary", id="apply-btn")
                yield Button("Clear All", variant="default", id="clear-btn")
                yield Button("Cancel", variant="default", id="cancel-btn")

    def action_apply(self) -> None:
        """Apply current filter state and dismiss modal."""
        filter_state = self._build_filter_state()
        self.post_message(FilterApplied(filter_state))
        self.dismiss(filter_state)

    def action_cancel(self) -> None:
        """Cancel filtering and dismiss modal without changes."""
        self.dismiss(None)

    def action_reset(self) -> None:
        """Clear all filter inputs and reset to empty state."""
        # Uncheck all status checkboxes
        for status in TaskStatus:
            checkbox = self.query_one(f"#status-{status.value}", Checkbox)
            checkbox.value = False

        # Uncheck all source checkboxes
        for source in TaskSource:
            checkbox = self.query_one(f"#source-{source.value}", Checkbox)
            checkbox.value = False

        # Clear all text inputs
        self.query_one("#agent-type-input", Input).value = ""
        self.query_one("#branch-input", Input).value = ""
        self.query_one("#search-input", Input).value = ""

    def on_checkbox_changed(self, event: Checkbox.Changed) -> None:
        """Handle checkbox state changes for radio button behavior."""
        # Implement radio button behavior for source checkboxes
        if event.checkbox.id and event.checkbox.id.startswith("source-"):
            if event.value:  # If checkbox was checked
                # Uncheck all other source checkboxes
                for source in TaskSource:
                    if event.checkbox.id != f"source-{source.value}":
                        other_checkbox = self.query_one(
                            f"#source-{source.value}", Checkbox
                        )
                        other_checkbox.value = False

    def on_button_pressed(self, event: Button.Pressed) -> None:
        """Handle button clicks."""
        if event.button.id == "apply-btn":
            self.action_apply()
        elif event.button.id == "clear-btn":
            self.action_reset()
        elif event.button.id == "cancel-btn":
            self.action_cancel()

    def _build_filter_state(self) -> FilterState:
        """Build FilterState from current form values.

        Returns:
            FilterState with values from form inputs
        """
        # Collect checked statuses
        checked_statuses = set()
        for status in TaskStatus:
            checkbox = self.query_one(f"#status-{status.value}", Checkbox)
            if checkbox.value:
                checked_statuses.add(status)

        # Get checked source (radio button behavior enforced in on_checkbox_changed)
        selected_source = None
        for source in TaskSource:
            checkbox = self.query_one(f"#source-{source.value}", Checkbox)
            if checkbox.value:
                selected_source = source
                break

        # Get text input values (None if empty)
        agent_type = self.query_one("#agent-type-input", Input).value.strip()
        branch = self.query_one("#branch-input", Input).value.strip()
        search = self.query_one("#search-input", Input).value.strip()

        return FilterState(
            status_filter=checked_statuses if checked_statuses else None,
            agent_type_filter=agent_type if agent_type else None,
            feature_branch_filter=branch if branch else None,
            text_search=search if search else None,
            source_filter=selected_source,
        )
