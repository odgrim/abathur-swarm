#!/usr/bin/env bash
# cleanup_all_worktrees.sh - Clean up all worktrees for a task
#
# Usage: ./cleanup_all_worktrees.sh <task_id>
#
# This hook removes all git worktrees created for a task's implementation.
# It removes both task worktrees and the feature worktree.

set -euo pipefail

TASK_ID="${1:-}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
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

log_section() {
    echo -e "\n${BLUE}[====] $* [====]${NC}\n"
}

# Validate inputs
if [[ -z "$TASK_ID" ]]; then
    log_error "Task ID is required"
    exit 1
fi

log_info "Cleaning up worktrees for task: $TASK_ID"

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    log_error "Not in a git repository"
    exit 1
fi

# Track cleanup results
REMOVED_COUNT=0
FAILED_COUNT=0
NOT_FOUND_COUNT=0

# Function to remove a worktree
remove_worktree() {
    local worktree_path="$1"
    local worktree_name="$2"

    log_info "Removing worktree: $worktree_name"
    log_info "  Path: $worktree_path"

    # Check if path exists
    if [[ ! -d "$worktree_path" ]]; then
        log_warn "  Worktree path does not exist, skipping"
        ((NOT_FOUND_COUNT++))
        return 0
    fi

    # Check if it's actually a worktree
    if ! git worktree list | grep -q "$worktree_path"; then
        log_warn "  Path exists but is not a git worktree"
        log_info "  Removing directory manually..."
        if rm -rf "$worktree_path"; then
            log_info "  ✓ Directory removed"
            ((REMOVED_COUNT++))
        else
            log_error "  ✗ Failed to remove directory"
            ((FAILED_COUNT++))
        fi
        return 0
    fi

    # Remove worktree using git
    if git worktree remove "$worktree_path" --force 2>&1; then
        log_info "  ✓ Worktree removed successfully"
        ((REMOVED_COUNT++))
    else
        log_error "  ✗ Failed to remove worktree"
        ((FAILED_COUNT++))
    fi
}

log_section "Cleaning up task worktrees"

# Find and remove task worktrees
TASK_WORKTREE_BASE=".abathur/worktrees/tasks"

if [[ -d "$TASK_WORKTREE_BASE" ]]; then
    # Find all task worktrees (they should contain task IDs or be in the tasks directory)
    while IFS= read -r worktree_path; do
        if [[ -n "$worktree_path" ]]; then
            worktree_name=$(basename "$worktree_path")
            remove_worktree "$worktree_path" "$worktree_name"
        fi
    done < <(find "$TASK_WORKTREE_BASE" -mindepth 1 -maxdepth 1 -type d 2>/dev/null || true)

    # Remove the task worktrees directory if empty
    if [[ -d "$TASK_WORKTREE_BASE" ]]; then
        if rmdir "$TASK_WORKTREE_BASE" 2>/dev/null; then
            log_info "Removed empty task worktrees directory"
        fi
    fi
else
    log_info "No task worktrees directory found"
fi

log_section "Cleaning up feature worktree"

# Try to get feature worktree path from memory
FEATURE_WORKTREE_PATH=""
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:git"
    if STORED_PATH=$(abathur memory get "$NAMESPACE" "feature_worktree_path" 2>/dev/null); then
        FEATURE_WORKTREE_PATH=$(echo "$STORED_PATH" | jq -r '.value // empty' | tr -d '"')
        log_info "Retrieved feature worktree path from memory: $FEATURE_WORKTREE_PATH"
    fi
fi

# If not found in memory, try common paths
if [[ -z "$FEATURE_WORKTREE_PATH" ]]; then
    # Try to find feature worktree in common locations
    FEATURE_BRANCH="feature/task-${TASK_ID}"
    BRANCH_PATH=$(echo "$FEATURE_BRANCH" | sed 's|^feature/||' | sed 's|/|-|g')
    FEATURE_WORKTREE_PATH=".abathur/worktrees/feature-${BRANCH_PATH}"
    log_info "Using default feature worktree path: $FEATURE_WORKTREE_PATH"
fi

if [[ -n "$FEATURE_WORKTREE_PATH" ]]; then
    remove_worktree "$FEATURE_WORKTREE_PATH" "feature"
fi

# Clean up any remaining worktrees in .abathur/worktrees
log_section "Cleaning up any remaining worktrees"

if [[ -d ".abathur/worktrees" ]]; then
    REMAINING_WORKTREES=$(find .abathur/worktrees -mindepth 1 -maxdepth 2 -type d 2>/dev/null | wc -l | tr -d ' ')

    if [[ $REMAINING_WORKTREES -gt 0 ]]; then
        log_warn "Found $REMAINING_WORKTREES remaining worktree directories"

        while IFS= read -r worktree_path; do
            if [[ -n "$worktree_path" ]] && [[ -d "$worktree_path" ]]; then
                worktree_name=$(basename "$worktree_path")
                remove_worktree "$worktree_path" "$worktree_name"
            fi
        done < <(find .abathur/worktrees -mindepth 1 -maxdepth 2 -type d 2>/dev/null || true)
    fi

    # Try to remove .abathur/worktrees directory if empty
    if rmdir .abathur/worktrees 2>/dev/null; then
        log_info "Removed empty worktrees directory"
    fi
fi

# Report results
log_section "Cleanup Results Summary"
log_info "Removed: $REMOVED_COUNT"
log_info "Not found: $NOT_FOUND_COUNT"
log_info "Failed: $FAILED_COUNT"

# Store cleanup results in memory
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:cleanup"
    TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    CLEANUP_RESULTS=$(jq -n \
        --arg removed "$REMOVED_COUNT" \
        --arg not_found "$NOT_FOUND_COUNT" \
        --arg failed "$FAILED_COUNT" \
        --arg timestamp "$TIMESTAMP" \
        '{
            removed: ($removed | tonumber),
            not_found: ($not_found | tonumber),
            failed: ($failed | tonumber),
            timestamp: $timestamp
        }')

    abathur memory add \
        --namespace "$NAMESPACE" \
        --key "results" \
        --value "$CLEANUP_RESULTS" \
        --type "semantic" \
        --created-by "technical_feature_workflow" 2>/dev/null || true

    log_info "✓ Cleanup results stored in memory"
fi

# Prune worktrees (remove stale references)
log_info "Pruning stale worktree references..."
if git worktree prune; then
    log_info "✓ Worktree references pruned"
fi

if [[ $FAILED_COUNT -gt 0 ]]; then
    log_error "Some worktrees failed to cleanup"
    exit 1
fi

log_info "✓ Worktree cleanup complete"
exit 0
