#!/usr/bin/env bash
# create_task_worktrees.sh - Creates worktrees for implementation tasks
#
# Usage: ./create_task_worktrees.sh <task_id> <tasks_json>
#
# This hook creates git worktrees for each implementation task that needs one.
# It reads the task plan JSON and creates worktrees for tasks with needs_worktree=true.
#
# Input: Step output via stdin or ABATHUR_STEP_OUTPUT environment variable

set -euo pipefail

TASK_ID="${1:-}"
TASKS_JSON_ARG="${2:-}"

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

log_info "Creating task worktrees for parent task: $TASK_ID"

# Get task plan JSON from stdin, environment variable, or argument
if [[ -n "${ABATHUR_STEP_OUTPUT:-}" ]]; then
    TASKS_JSON="$ABATHUR_STEP_OUTPUT"
elif [[ "$TASKS_JSON_ARG" == "tasks_json" ]]; then
    log_info "Reading task plan from stdin..."
    TASKS_JSON=$(cat)
else
    TASKS_JSON="$TASKS_JSON_ARG"
fi

if [[ -z "$TASKS_JSON" ]]; then
    log_error "No task plan JSON provided"
    exit 1
fi

# Validate JSON
if ! echo "$TASKS_JSON" | jq empty 2>/dev/null; then
    log_error "Invalid JSON provided"
    exit 1
fi

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    log_error "Not in a git repository"
    exit 1
fi

# Get feature branch from memory or use default
FEATURE_BRANCH="feature/task-${TASK_ID}"
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:git"
    if STORED_BRANCH=$(abathur memory get "$NAMESPACE" "feature_branch" 2>/dev/null); then
        FEATURE_BRANCH=$(echo "$STORED_BRANCH" | jq -r '.value // "feature/task-'"$TASK_ID"'"')
        log_info "Retrieved feature branch from memory: $FEATURE_BRANCH"
    fi
fi

# Verify feature branch exists
if ! git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    log_error "Feature branch $FEATURE_BRANCH does not exist"
    log_error "Run create_feature_branch.sh first"
    exit 1
fi

log_info "Using feature branch: $FEATURE_BRANCH"

# Create worktrees directory
mkdir -p .abathur/worktrees/tasks

# Extract tasks that need worktrees
TASKS_NEEDING_WORKTREES=$(echo "$TASKS_JSON" | jq -c '.tasks[] | select(.needs_worktree == true)')

if [[ -z "$TASKS_NEEDING_WORKTREES" ]]; then
    log_info "No tasks require worktrees"
    exit 0
fi

# Count tasks
TASK_COUNT=$(echo "$TASKS_NEEDING_WORKTREES" | wc -l | tr -d ' ')
log_info "Creating worktrees for $TASK_COUNT tasks"

# Create worktree for each task
CREATED_COUNT=0
SKIPPED_COUNT=0
FAILED_COUNT=0

while IFS= read -r task_json; do
    SUBTASK_ID=$(echo "$task_json" | jq -r '.id')
    SUBTASK_SUMMARY=$(echo "$task_json" | jq -r '.summary')

    # Create sanitized task branch name
    TASK_BRANCH="task/${SUBTASK_ID}"
    TASK_PATH=$(echo "$SUBTASK_ID" | sed 's|/|-|g')
    WORKTREE_PATH=".abathur/worktrees/tasks/${TASK_PATH}"

    log_info "Processing task: $SUBTASK_ID"
    log_info "  Summary: $SUBTASK_SUMMARY"
    log_info "  Branch: $TASK_BRANCH"
    log_info "  Path: $WORKTREE_PATH"

    # Check if worktree already exists
    if [[ -d "$WORKTREE_PATH" ]]; then
        log_warn "  Worktree already exists, skipping"
        ((SKIPPED_COUNT++))
        continue
    fi

    # Create worktree
    if git worktree add -b "$TASK_BRANCH" "$WORKTREE_PATH" "$FEATURE_BRANCH" 2>&1; then
        log_info "  ✓ Worktree created successfully"
        ((CREATED_COUNT++))

        # Store worktree info in memory
        if command -v abathur &> /dev/null; then
            SUB_NAMESPACE="task:${TASK_ID}:subtask:${SUBTASK_ID}"
            abathur memory add \
                --namespace "$SUB_NAMESPACE" \
                --key "worktree_path" \
                --value "\"$WORKTREE_PATH\"" \
                --type "episodic" \
                --created-by "technical_feature_workflow" 2>/dev/null || true
            abathur memory add \
                --namespace "$SUB_NAMESPACE" \
                --key "branch_name" \
                --value "\"$TASK_BRANCH\"" \
                --type "episodic" \
                --created-by "technical_feature_workflow" 2>/dev/null || true
        fi
    else
        log_error "  Failed to create worktree"
        ((FAILED_COUNT++))
    fi
done <<< "$TASKS_NEEDING_WORKTREES"

log_info "✓ Task worktree creation complete"
log_info "  Created: $CREATED_COUNT"
log_info "  Skipped: $SKIPPED_COUNT"
log_info "  Failed: $FAILED_COUNT"

if [[ $FAILED_COUNT -gt 0 ]]; then
    log_error "Some worktrees failed to create"
    exit 1
fi

exit 0
