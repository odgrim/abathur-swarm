#!/usr/bin/env bash
# setup_task_planner_branch.sh - Detect task branch and create worktree for task-planner
#
# Usage: ./setup_task_planner_branch.sh <task_id> <feature_branch>
#
# This hook is triggered when task-planner starts (PreStart).
# It detects the task branch created by spawn_task_planner.sh and creates the worktree.

set -euo pipefail

TASK_ID="${1:-}"
FEATURE_BRANCH="${2:-}"

if [[ -z "$TASK_ID" || -z "$FEATURE_BRANCH" ]]; then
    echo "[ERROR] Usage: $0 <task_id> <feature_branch>"
    exit 1
fi

echo "[INFO] Setting up task branch and worktree for task-planner"
echo "[INFO]   Task ID: $TASK_ID"
echo "[INFO]   Feature Branch: $FEATURE_BRANCH"

# Extract feature name from feature_branch (e.g., "feature/user-auth" -> "user-auth")
FEATURE_NAME="${FEATURE_BRANCH#feature/}"
if [[ "$FEATURE_NAME" == "$FEATURE_BRANCH" ]]; then
    # Fallback if not in feature/ format
    FEATURE_NAME="unknown"
fi

# Look for existing task branch matching pattern: task/{feature_name}/plan-*
# This branch should have been created by spawn_task_planner.sh
BRANCH_PATTERN="task/${FEATURE_NAME}/plan-"
echo "[INFO] Looking for existing task branch matching pattern: ${BRANCH_PATTERN}*"

# List branches matching the pattern
MATCHING_BRANCHES=$(git branch --list "${BRANCH_PATTERN}*" | sed 's/^[* ]*//' || echo "")

if [[ -z "$MATCHING_BRANCHES" ]]; then
    echo "[ERROR] No task branch found matching pattern: ${BRANCH_PATTERN}*"
    echo "[ERROR] Expected branch to be created by spawn_task_planner.sh"
    exit 1
fi

# Get the most recent matching branch (should only be one)
BRANCH=$(echo "$MATCHING_BRANCHES" | head -1)
echo "[INFO] Found task branch: $BRANCH"

# Extract the identifier from branch name (e.g., "plan-e1b4e4d8" from "task/feature/plan-e1b4e4d8")
BRANCH_SUFFIX="${BRANCH##*/}"  # Gets "plan-e1b4e4d8"
TASK_ID_SHORT="${BRANCH_SUFFIX#plan-}"  # Gets "e1b4e4d8"

# Generate worktree path: .abathur/worktrees/task-{short_id}
WORKTREE_PATH=".abathur/worktrees/task-${TASK_ID_SHORT}"

echo "[INFO] Creating worktree for task-planner"
echo "[INFO]   Branch: $BRANCH"
echo "[INFO]   Worktree Path: $WORKTREE_PATH"

# Check if worktree already exists
if [[ -d "$WORKTREE_PATH" ]]; then
    echo "[WARN] Worktree already exists at $WORKTREE_PATH"
    echo "[INFO] Reusing existing worktree"
else
    # Create worktree from existing branch
    echo "[INFO] Creating worktree from branch $BRANCH"
    if git worktree add "$WORKTREE_PATH" "$BRANCH" 2>&1 | sed 's/^/[GIT]   /'; then
        echo "[INFO] ✓ Worktree created successfully"
    else
        echo "[ERROR] Failed to create worktree"
        exit 1
    fi
fi

# Output structured data for hook executor to parse and update task fields
# These values will be captured and used to update the task's branch and worktree_path fields
echo "ABATHUR_BRANCH=$BRANCH"
echo "ABATHUR_WORKTREE_PATH=$WORKTREE_PATH"

echo "[INFO] ✓ Task branch and worktree setup complete"
exit 0
