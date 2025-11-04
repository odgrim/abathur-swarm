#!/usr/bin/env bash
# Prepare worktree for feature branch
# Usage: prepare_feature_worktree.sh <task_id> <branch_name>

set -euo pipefail

TASK_ID="${1:?Task ID required}"
BRANCH_NAME="${2:?Branch name required}"
WORKTREE_PATH=".worktrees/${BRANCH_NAME}"

echo "Preparing feature worktree for: ${BRANCH_NAME}"

# Create worktrees directory if it doesn't exist
mkdir -p .worktrees

# Check if worktree already exists
if [ -d "${WORKTREE_PATH}" ]; then
    echo "Worktree already exists at ${WORKTREE_PATH}"
    cd "${WORKTREE_PATH}"
else
    # Create worktree
    git worktree add "${WORKTREE_PATH}" "${BRANCH_NAME}" 2>/dev/null || {
        echo "Branch doesn't exist yet, creating from main"
        git worktree add -b "${BRANCH_NAME}" "${WORKTREE_PATH}" main
    }
    echo "Created worktree at: ${WORKTREE_PATH}"
    cd "${WORKTREE_PATH}"
fi

# Verify we're in the right branch
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [ "${CURRENT_BRANCH}" != "${BRANCH_NAME}" ]; then
    echo "ERROR: Expected branch ${BRANCH_NAME}, but got ${CURRENT_BRANCH}"
    exit 1
fi

echo "Feature worktree ready at: $(pwd)"
echo "Branch: ${CURRENT_BRANCH}"
