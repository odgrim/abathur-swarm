#!/usr/bin/env bash
# create_feature_branch.sh - Creates a feature branch with git worktree
#
# Usage: ./create_feature_branch.sh <task_id> <feature_name>
#
# This hook is triggered when a technical-requirements-specialist task starts.
# It creates a feature branch worktree for isolated development.

set -euo pipefail

TASK_ID="${1:-}"
FEATURE_NAME="${2:-}"

if [[ -z "$TASK_ID" || -z "$FEATURE_NAME" ]]; then
    echo "[ERROR] Usage: $0 <task_id> <feature_name>"
    exit 1
fi

# Sanitize feature name (remove spaces, special chars, lowercase)
FEATURE_NAME_CLEAN=$(echo "$FEATURE_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

# Truncate to reasonable length
FEATURE_NAME_CLEAN=$(echo "$FEATURE_NAME_CLEAN" | cut -c1-50)

FEATURE_BRANCH="feature/${FEATURE_NAME_CLEAN}"
# Feature branches: feature/my-feature -> .abathur/worktrees/feature-my-feature
WORKTREE_PATH=".abathur/worktrees/feature-${FEATURE_NAME_CLEAN}"

echo "[INFO] Creating feature branch worktree"
echo "[INFO]   Task ID: $TASK_ID"
echo "[INFO]   Branch: $FEATURE_BRANCH"
echo "[INFO]   Path: $WORKTREE_PATH"

# Check if worktree already exists
if [[ -d "$WORKTREE_PATH" ]]; then
    echo "[WARN] Worktree already exists at $WORKTREE_PATH"
    echo "[INFO] Reusing existing worktree"
    exit 0
fi

# Check if branch already exists
if git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    echo "[WARN] Branch $FEATURE_BRANCH already exists"
    echo "[INFO] Creating worktree from existing branch"
    git worktree add "$WORKTREE_PATH" "$FEATURE_BRANCH"
else
    echo "[INFO] Creating new feature branch and worktree"
    git worktree add -b "$FEATURE_BRANCH" "$WORKTREE_PATH"
fi

echo "[INFO] Feature branch worktree created successfully"
echo "[INFO] Branch: $FEATURE_BRANCH"
echo "[INFO] Path: $WORKTREE_PATH"

# Output structured data for hook executor to parse and update task fields
# These values will be captured and used to update the task's feature_branch and worktree_path fields
echo "ABATHUR_FEATURE_BRANCH=$FEATURE_BRANCH"
echo "ABATHUR_WORKTREE_PATH=$WORKTREE_PATH"

exit 0
