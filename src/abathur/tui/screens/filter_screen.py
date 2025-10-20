"""Filter screen for task filtering in TaskQueueTUI.

This is a placeholder implementation for Phase 1.
Full implementation will be completed in Phase 4.
"""

from typing import Any

from textual.app import ComposeResult
from textual.screen import ModalScreen
from textual.widgets import Button, Static
from textual.containers import Container, Horizontal


class FilterScreen(ModalScreen[dict[str, Any] | None]):
    """Modal screen for filtering tasks by status, agent, branch, and text.

    This modal allows users to set filter criteria:
    - Status: Checkboxes for each TaskStatus value
    - Agent type: Dropdown/input for agent_type filter
    - Feature branch: Input for feature_branch filter
    - Text search: Input for searching task summaries/prompts

    Returns FilterState dict or None if cancelled.

    This is a placeholder for Phase 1. Full form widgets and filtering
    logic will be implemented in Phase 4.
    """

    def compose(self) -> ComposeResult:
        """Create filter form widgets.

        Phase 1 placeholder: Basic layout with cancel button.
        """
        with Container(id="filter-form"):
            yield Static("Filter Options - Coming Soon")

            with Horizontal():
                yield Button("Cancel", id="cancel")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        """Handle button clicks.

        Args:
            event: Button press event
        """
        # Phase 1: Only cancel functionality
        if event.button.id == "cancel":
            self.dismiss(None)
