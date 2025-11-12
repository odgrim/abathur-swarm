#!/usr/bin/env bash
# create_feature_branch.sh - Creates a feature branch for the task
#
# Usage: ./create_feature_branch.sh <task_id> <feature_name> <decomposition_strategy>
#
# This hook creates a feature branch with a human-readable name based on the feature being implemented.
# For single projects, it creates a simple feature branch.
# For multiple projects, it may create additional branches.

set -euo pipefail

TASK_ID="${1:-}"
FEATURE_NAME="${2:-}"
DECOMPOSITION_STRATEGY="${3:-single}"

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

if [[ -z "$FEATURE_NAME" ]]; then
    log_error "Feature name is required"
    exit 1
fi

log_info "Creating feature branch for task: $TASK_ID"
log_info "Feature name: $FEATURE_NAME"
log_info "Decomposition strategy: $DECOMPOSITION_STRATEGY"

# Sanitize feature name for use in branch (convert to kebab-case, lowercase, remove special chars)
SANITIZED_NAME=$(echo "$FEATURE_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

# Create branch name
BRANCH_NAME="feature/${SANITIZED_NAME}"

log_info "Branch name: $BRANCH_NAME"

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    log_error "Not in a git repository"
    exit 1
fi

# Get current branch name for branch point
CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD)
log_info "Current branch: $CURRENT_BRANCH"

# Check if branch already exists
if git show-ref --verify --quiet "refs/heads/$BRANCH_NAME"; then
    log_warn "Branch $BRANCH_NAME already exists"
    log_info "Skipping branch creation"
else
    log_info "Creating new feature branch from $CURRENT_BRANCH (without checkout)"
    # Create new branch without checking it out - everything uses worktrees
    git branch "$BRANCH_NAME" "$CURRENT_BRANCH"
    log_info "✓ Created branch: $BRANCH_NAME"
fi

# Store branch name in memory for later use
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:git"
    if abathur memory add \
        --namespace "$NAMESPACE" \
        --key "feature_branch" \
        --value "\"$BRANCH_NAME\"" \
        --type "episodic" \
        --created-by "technical_feature_workflow"; then
        log_info "✓ Branch name stored in memory"
    else
        log_warn "Could not store branch name in memory (non-fatal)"
    fi
fi

log_info "✓ Feature branch creation complete"
log_info "  Branch: $BRANCH_NAME"
log_info "  Strategy: $DECOMPOSITION_STRATEGY"

exit 0
