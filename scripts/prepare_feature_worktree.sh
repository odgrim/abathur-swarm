#!/usr/bin/env bash
# prepare_feature_worktree.sh - Prepares a git worktree for the feature branch
#
# Usage: ./prepare_feature_worktree.sh <task_id> <feature_branch>
#
# This hook creates a git worktree for the feature branch to enable
# isolated development without affecting the main working directory.

set -euo pipefail

TASK_ID="${1:-}"
FEATURE_BRANCH="${2:-}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

# Validate inputs
if [[ -z "$TASK_ID" ]]; then
    log_error "Task ID is required"
    exit 1
fi

if [[ -z "$FEATURE_BRANCH" ]]; then
    log_error "Feature branch is required"
    exit 1
fi

log_info "Preparing feature worktree for task: $TASK_ID"
log_info "Feature branch: $FEATURE_BRANCH"

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    log_error "Not in a git repository"
    exit 1
fi

# Clean up branch name for path (remove feature/ prefix)
BRANCH_PATH=$(echo "$FEATURE_BRANCH" | sed 's|^feature/||' | sed 's|/|-|g')
WORKTREE_PATH=".abathur/worktrees/feature-${BRANCH_PATH}"

log_info "Worktree path: $WORKTREE_PATH"

# Create worktrees directory if it doesn't exist
mkdir -p .abathur/worktrees

# Check if worktree already exists
if [[ -d "$WORKTREE_PATH" ]]; then
    log_warn "Worktree already exists at $WORKTREE_PATH"
    log_info "Reusing existing worktree"

    # Verify it's actually a valid worktree
    if git worktree list | grep -q "$WORKTREE_PATH"; then
        log_info "✓ Existing worktree is valid"
        exit 0
    else
        log_warn "Path exists but is not a valid worktree, removing..."
        rm -rf "$WORKTREE_PATH"
    fi
fi

# Check if feature branch exists
if git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    log_info "Feature branch exists, creating worktree from existing branch"
    git worktree add "$WORKTREE_PATH" "$FEATURE_BRANCH"
else
    log_info "Creating new feature branch and worktree"
    git worktree add -b "$FEATURE_BRANCH" "$WORKTREE_PATH"
fi

log_info "✓ Feature worktree created successfully"
log_info "  Branch: $FEATURE_BRANCH"
log_info "  Path: $WORKTREE_PATH"

# Store worktree path in memory
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:git"
    if abathur memory add \
        --namespace "$NAMESPACE" \
        --key "feature_worktree_path" \
        --value "\"$WORKTREE_PATH\"" \
        --type "episodic" \
        --created-by "technical_feature_workflow"; then
        log_info "✓ Worktree path stored in memory"
    else
        log_warn "Could not store worktree path in memory (non-fatal)"
    fi
fi

exit 0
