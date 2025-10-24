#!/usr/bin/env python3
"""Quick verification test for _build_tree_string() function."""

from uuid import UUID, uuid4
from rich.text import Text
from abathur.domain.models import Task, TaskStatus
from abathur.tui.models import TreeNode


def create_mock_task(
    task_id: UUID,
    summary: str,
    status: TaskStatus,
    parent_id: UUID | None = None,
    priority: int = 5,
) -> Task:
    """Create a mock Task object for testing."""
    return Task(
        id=task_id,
        prompt=f"Test prompt for {summary}",
        summary=summary,
        description=f"Description for {summary}",
        status=status,
        source="human",
        agent_type="test-agent",
        parent_task_id=parent_id,
        base_priority=priority,
        calculated_priority=priority,
    )


def create_mock_tree_node(task: Task, children: list[UUID]) -> TreeNode:
    """Create a mock TreeNode object for testing."""
    return TreeNode(
        task_id=task.id,
        task=task,
        children=children,
        level=0,
        is_expanded=True,
        position=0,
    )


def test_tree_structure():
    """Test the _build_tree_string function with a simple hierarchy."""
    # Import the function we're testing
    import sys
    sys.path.insert(0, 'src')
    from abathur.cli.main import _build_tree_string

    # Create a simple tree structure:
    #
    # Root Task 1
    # ├── Child 1.1
    # │   ├── Grandchild 1.1.1
    # │   └── Grandchild 1.1.2
    # └── Child 1.2
    # Root Task 2
    # └── Child 2.1

    # Create task IDs
    root1_id = uuid4()
    child11_id = uuid4()
    child12_id = uuid4()
    grandchild111_id = uuid4()
    grandchild112_id = uuid4()
    root2_id = uuid4()
    child21_id = uuid4()

    # Create tasks
    root1 = create_mock_task(root1_id, "Root Task 1", TaskStatus.RUNNING)
    child11 = create_mock_task(child11_id, "Child 1.1", TaskStatus.PENDING, root1_id)
    child12 = create_mock_task(child12_id, "Child 1.2", TaskStatus.COMPLETED, root1_id)
    grandchild111 = create_mock_task(grandchild111_id, "Grandchild 1.1.1", TaskStatus.COMPLETED, child11_id)
    grandchild112 = create_mock_task(grandchild112_id, "Grandchild 1.1.2", TaskStatus.FAILED, child11_id)
    root2 = create_mock_task(root2_id, "Root Task 2", TaskStatus.PENDING)
    child21 = create_mock_task(child21_id, "Child 2.1", TaskStatus.READY, root2_id)

    # Create tree nodes
    nodes = [
        create_mock_tree_node(root1, [child11_id, child12_id]),
        create_mock_tree_node(child11, [grandchild111_id, grandchild112_id]),
        create_mock_tree_node(child12, []),
        create_mock_tree_node(grandchild111, []),
        create_mock_tree_node(grandchild112, []),
        create_mock_tree_node(root2, [child21_id]),
        create_mock_tree_node(child21, []),
    ]

    # Test Unicode mode
    print("=" * 80)
    print("UNICODE MODE TEST")
    print("=" * 80)
    lines_unicode = _build_tree_string(nodes, max_depth=5, use_unicode=True)

    for line in lines_unicode:
        # Print the plain text to see structure
        print(repr(line.plain))

    print("\n" + "=" * 80)
    print("ASCII MODE TEST")
    print("=" * 80)
    lines_ascii = _build_tree_string(nodes, max_depth=5, use_unicode=False)

    for line in lines_ascii:
        # Print the plain text to see structure
        print(repr(line.plain))

    # Test depth truncation
    print("\n" + "=" * 80)
    print("DEPTH TRUNCATION TEST (max_depth=2)")
    print("=" * 80)
    lines_truncated = _build_tree_string(nodes, max_depth=2, use_unicode=True)

    for line in lines_truncated:
        print(repr(line.plain))

    # Verify some basic assertions
    print("\n" + "=" * 80)
    print("VERIFICATION")
    print("=" * 80)

    # Check that we got the right number of lines (7 nodes total)
    assert len(lines_unicode) == 7, f"Expected 7 lines, got {len(lines_unicode)}"
    print(f"✓ Unicode mode: {len(lines_unicode)} lines (correct)")

    assert len(lines_ascii) == 7, f"Expected 7 lines, got {len(lines_ascii)}"
    print(f"✓ ASCII mode: {len(lines_ascii)} lines (correct)")

    # Check truncation shows "..." indicator
    truncation_found = any("..." in line.plain for line in lines_truncated)
    assert truncation_found, "Expected to find '...' truncation indicator"
    print("✓ Depth truncation: '...' indicator found")

    # Check for correct connectors in Unicode mode
    unicode_text = "\n".join(line.plain for line in lines_unicode)
    assert "├──" in unicode_text, "Expected to find ├── connector"
    assert "└──" in unicode_text, "Expected to find └── connector"
    assert "│" in unicode_text, "Expected to find │ vertical line"
    print("✓ Unicode connectors: ├── └── │ present")

    # Check for correct connectors in ASCII mode
    ascii_text = "\n".join(line.plain for line in lines_ascii)
    assert "|--" in ascii_text, "Expected to find |-- connector"
    assert "`--" in ascii_text, "Expected to find `-- connector"
    print("✓ ASCII connectors: |-- `-- present")

    print("\n" + "=" * 80)
    print("ALL TESTS PASSED ✓")
    print("=" * 80)


if __name__ == "__main__":
    test_tree_structure()
