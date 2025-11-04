#!/usr/bin/env bash
# Cleanup worktree for a single completed task
# Usage: cleanup_task_worktree.sh <task_id>

set -euo pipefail

TASK_ID="${1:?Task ID required}"
WORKTREES_DIR=".worktrees"

echo "Cleaning up worktree for task: ${TASK_ID}"

# Function to safely remove a worktree
remove_worktree() {
    local worktree_path="$1"
    local branch_name="$2"

    if [ -d "${worktree_path}" ]; then
        echo "Removing worktree: ${worktree_path}"
        git worktree remove "${worktree_path}" --force || {
            echo "Warning: Failed to remove worktree ${worktree_path}"
            return 1
        }
    else
        echo "Worktree not found: ${worktree_path}"
    fi

    # Check if branch exists and delete it
    if git show-ref --verify --quiet "refs/heads/${branch_name}"; then
        echo "Deleting branch: ${branch_name}"
        git branch -D "${branch_name}" || {
            echo "Warning: Failed to delete branch ${branch_name}"
            return 1
        }
    fi
}

# Determine the worktree path and branch name
WORKTREE_PATH="${WORKTREES_DIR}/task-${TASK_ID}"
BRANCH_NAME="task-${TASK_ID}"

# Remove the task worktree and branch
if ! remove_worktree "${WORKTREE_PATH}" "${BRANCH_NAME}"; then
    echo "Error: Failed to clean up task ${TASK_ID}"
    exit 1
fi

# Prune any stale worktree references
echo "Pruning stale worktree references..."
git worktree prune

echo "✓ Worktree cleanup complete for task: ${TASK_ID}"
