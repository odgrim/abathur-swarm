#!/usr/bin/env bash
# create_task_worktree.sh - Creates a task branch with git worktree
#
# Usage: ./create_task_worktree.sh <task_id> <branch> <feature_branch> <worktree_path>
#
# This hook is triggered when an implementation task starts.
# It creates a task branch worktree branched from the feature branch.

set -euo pipefail

TASK_ID="${1:-}"
BRANCH="${2:-}"
FEATURE_BRANCH="${3:-}"
WORKTREE_PATH="${4:-}"

if [[ -z "$TASK_ID" || -z "$BRANCH" || -z "$FEATURE_BRANCH" || -z "$WORKTREE_PATH" ]]; then
    echo "[ERROR] Usage: $0 <task_id> <branch> <feature_branch> <worktree_path>"
    exit 1
fi

echo "[INFO] Creating task branch worktree"
echo "[INFO]   Task ID: $TASK_ID"
echo "[INFO]   Branch: $BRANCH"
echo "[INFO]   Feature Branch: $FEATURE_BRANCH"
echo "[INFO]   Worktree Path: $WORKTREE_PATH"

# Check if worktree already exists AND is a valid git worktree
if [[ -d "$WORKTREE_PATH" ]]; then
    # Verify it's actually a git worktree (has .git file pointing to main repo)
    if [[ -f "$WORKTREE_PATH/.git" ]] && git -C "$WORKTREE_PATH" rev-parse --git-dir >/dev/null 2>&1; then
        echo "[INFO] Valid git worktree already exists at $WORKTREE_PATH"
        echo "[INFO] Reusing existing worktree"
        exit 0
    else
        echo "[WARN] Directory exists at $WORKTREE_PATH but is NOT a valid git worktree"
        echo "[INFO] Removing invalid directory and creating proper worktree"
        rm -rf "$WORKTREE_PATH"
    fi
fi

# Verify feature branch exists
if ! git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    echo "[ERROR] Feature branch $FEATURE_BRANCH does not exist"
    echo "[ERROR] Cannot create task branch without feature branch"
    exit 1
fi

# Check if task branch already exists
if git show-ref --verify --quiet "refs/heads/$BRANCH"; then
    echo "[WARN] Branch $BRANCH already exists"
    echo "[INFO] Creating worktree from existing branch"
    git worktree add "$WORKTREE_PATH" "$BRANCH"
else
    echo "[INFO] Creating new branch from $FEATURE_BRANCH"
    git worktree add -b "$BRANCH" "$WORKTREE_PATH" "$FEATURE_BRANCH"
fi

echo "[INFO] Task branch worktree created successfully"
echo "[INFO] Branch: $BRANCH"
echo "[INFO] Worktree Path: $WORKTREE_PATH"

# Output structured data for hook executor to parse and update task fields
# These values will be captured and used to update the task's branch and worktree_path fields
echo "ABATHUR_BRANCH=$BRANCH"
echo "ABATHUR_FEATURE_BRANCH=$FEATURE_BRANCH"
echo "ABATHUR_WORKTREE_PATH=$WORKTREE_PATH"

exit 0
