#!/usr/bin/env bash
# create_task_worktree.sh - Creates a task branch with git worktree
#
# Usage: ./create_task_worktree.sh <task_id> <task_branch> <feature_branch> <worktree_path>
#
# This hook is triggered when an implementation task starts.
# It creates a task branch worktree branched from the feature branch.

set -euo pipefail

TASK_ID="${1:-}"
TASK_BRANCH="${2:-}"
FEATURE_BRANCH="${3:-}"
WORKTREE_PATH="${4:-}"

if [[ -z "$TASK_ID" || -z "$TASK_BRANCH" || -z "$FEATURE_BRANCH" || -z "$WORKTREE_PATH" ]]; then
    echo "[ERROR] Usage: $0 <task_id> <task_branch> <feature_branch> <worktree_path>"
    exit 1
fi

echo "[INFO] Creating task branch worktree"
echo "[INFO]   Task ID: $TASK_ID"
echo "[INFO]   Task Branch: $TASK_BRANCH"
echo "[INFO]   Feature Branch: $FEATURE_BRANCH"
echo "[INFO]   Worktree Path: $WORKTREE_PATH"

# Check if worktree already exists
if [[ -d "$WORKTREE_PATH" ]]; then
    echo "[WARN] Worktree already exists at $WORKTREE_PATH"
    echo "[INFO] Reusing existing worktree"
    exit 0
fi

# Verify feature branch exists
if ! git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    echo "[ERROR] Feature branch $FEATURE_BRANCH does not exist"
    echo "[ERROR] Cannot create task branch without feature branch"
    exit 1
fi

# Check if task branch already exists
if git show-ref --verify --quiet "refs/heads/$TASK_BRANCH"; then
    echo "[WARN] Task branch $TASK_BRANCH already exists"
    echo "[INFO] Creating worktree from existing branch"
    git worktree add "$WORKTREE_PATH" "$TASK_BRANCH"
else
    echo "[INFO] Creating new task branch from $FEATURE_BRANCH"
    git worktree add -b "$TASK_BRANCH" "$WORKTREE_PATH" "$FEATURE_BRANCH"
fi

echo "[INFO] Task branch worktree created successfully"
echo "[INFO] Task Branch: $TASK_BRANCH"
echo "[INFO] Worktree Path: $WORKTREE_PATH"

# Output structured data for hook executor to parse and update task fields
# These values will be captured and used to update the task's task_branch and worktree_path fields
echo "ABATHUR_TASK_BRANCH=$TASK_BRANCH"
echo "ABATHUR_FEATURE_BRANCH=$FEATURE_BRANCH"
echo "ABATHUR_WORKTREE_PATH=$WORKTREE_PATH"

exit 0
