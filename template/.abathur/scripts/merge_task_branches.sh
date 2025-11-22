#!/usr/bin/env bash
# merge_branches.sh - Merge task branches to feature branch
#
# Usage: ./merge_branches.sh <task_id> <branches_json>
#
# This hook merges all task branches into the feature branch.
# It reads the branches to merge from JSON input.
#
# Input: Step output via stdin or ABATHUR_STEP_OUTPUT environment variable

set -euo pipefail

TASK_ID="${1:-}"
BRANCHES_JSON_ARG="${2:-}"

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

log_info "Merging task branches for task: $TASK_ID"

# Get branches JSON from stdin, environment variable, or argument
if [[ -n "${ABATHUR_STEP_OUTPUT:-}" ]]; then
    STEP_OUTPUT="$ABATHUR_STEP_OUTPUT"
elif [[ "$BRANCHES_JSON_ARG" == "branches_json" ]]; then
    log_info "Reading merge plan from stdin..."
    STEP_OUTPUT=$(cat)
else
    STEP_OUTPUT="$BRANCHES_JSON_ARG"
fi

if [[ -z "$STEP_OUTPUT" ]]; then
    log_error "No merge plan provided"
    exit 1
fi

# Validate JSON
if ! echo "$STEP_OUTPUT" | jq empty 2>/dev/null; then
    log_error "Invalid JSON provided"
    exit 1
fi

# Check if we're in a git repository
if ! git rev-parse --git-dir > /dev/null 2>&1; then
    log_error "Not in a git repository"
    exit 1
fi

# Extract branches to merge
BRANCHES_TO_MERGE=$(echo "$STEP_OUTPUT" | jq -r '.branches_to_merge // []')

if [[ "$BRANCHES_TO_MERGE" == "[]" ]]; then
    log_warn "No branches to merge specified"
    exit 0
fi

BRANCH_COUNT=$(echo "$BRANCHES_TO_MERGE" | jq 'length')
log_info "Found $BRANCH_COUNT branches to merge"

# Get feature branch (target)
FEATURE_BRANCH=$(echo "$BRANCHES_TO_MERGE" | jq -r '.[0].target // empty')

if [[ -z "$FEATURE_BRANCH" ]]; then
    # Try to get from memory
    if command -v abathur &> /dev/null; then
        NAMESPACE="task:${TASK_ID}:git"
        if STORED_BRANCH=$(abathur memory get "$NAMESPACE" "feature_branch" 2>/dev/null); then
            FEATURE_BRANCH=$(echo "$STORED_BRANCH" | jq -r '.value // empty' | tr -d '"')
        fi
    fi

    if [[ -z "$FEATURE_BRANCH" ]]; then
        FEATURE_BRANCH="feature/task-${TASK_ID}"
        log_warn "Using default feature branch: $FEATURE_BRANCH"
    fi
fi

log_info "Target feature branch: $FEATURE_BRANCH"

# Verify feature branch exists
if ! git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    log_error "Feature branch $FEATURE_BRANCH does not exist"
    exit 1
fi

# Checkout feature branch
log_info "Checking out feature branch: $FEATURE_BRANCH"
git checkout "$FEATURE_BRANCH"

# Track merge results
MERGED_COUNT=0
FAILED_COUNT=0
SKIPPED_COUNT=0

# Merge each task branch
while IFS= read -r merge_info; do
    SOURCE_BRANCH=$(echo "$merge_info" | jq -r '.source')
    TARGET_BRANCH=$(echo "$merge_info" | jq -r '.target')

    log_section "Merging $SOURCE_BRANCH -> $TARGET_BRANCH"

    # Verify source branch exists
    if ! git show-ref --verify --quiet "refs/heads/$SOURCE_BRANCH"; then
        log_warn "Source branch $SOURCE_BRANCH does not exist, skipping"
        ((SKIPPED_COUNT++))
        continue
    fi

    # Check if branch is already merged
    if git merge-base --is-ancestor "$SOURCE_BRANCH" "$FEATURE_BRANCH" 2>/dev/null; then
        log_info "Branch $SOURCE_BRANCH is already merged, skipping"
        ((SKIPPED_COUNT++))
        continue
    fi

    # Perform merge
    log_info "Merging $SOURCE_BRANCH..."

    if git merge --no-ff --no-edit "$SOURCE_BRANCH"; then
        log_info "✓ Merged $SOURCE_BRANCH successfully"
        ((MERGED_COUNT++))
    else
        log_error "✗ Failed to merge $SOURCE_BRANCH"
        log_error "Merge conflicts detected"

        # Abort the merge
        git merge --abort 2>/dev/null || true

        ((FAILED_COUNT++))

        # Store conflict info
        if command -v abathur &> /dev/null; then
            NAMESPACE="task:${TASK_ID}:merge:conflicts"
            CONFLICT_INFO=$(jq -n \
                --arg source "$SOURCE_BRANCH" \
                --arg target "$TARGET_BRANCH" \
                '{source: $source, target: $target}')

            abathur memory add \
                --namespace "$NAMESPACE" \
                --key "$SOURCE_BRANCH" \
                --value "$CONFLICT_INFO" \
                --type "episodic" \
                --created-by "technical_feature_workflow" 2>/dev/null || true
        fi
    fi
done < <(echo "$BRANCHES_TO_MERGE" | jq -c '.[]')

# Report results
log_section "Merge Results Summary"
log_info "Total branches: $BRANCH_COUNT"
log_info "Merged: $MERGED_COUNT"
log_info "Skipped: $SKIPPED_COUNT"
log_info "Failed: $FAILED_COUNT"

# Store results in memory
if command -v abathur &> /dev/null; then
    NAMESPACE="task:${TASK_ID}:merge"
    TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

    MERGE_RESULTS=$(jq -n \
        --arg merged "$MERGED_COUNT" \
        --arg skipped "$SKIPPED_COUNT" \
        --arg failed "$FAILED_COUNT" \
        --arg total "$BRANCH_COUNT" \
        --arg timestamp "$TIMESTAMP" \
        '{
            total: ($total | tonumber),
            merged: ($merged | tonumber),
            skipped: ($skipped | tonumber),
            failed: ($failed | tonumber),
            timestamp: $timestamp
        }')

    abathur memory add \
        --namespace "$NAMESPACE" \
        --key "results" \
        --value "$MERGE_RESULTS" \
        --type "semantic" \
        --created-by "technical_feature_workflow" 2>/dev/null || true

    log_info "✓ Merge results stored in memory"
fi

# Exit with error if any merges failed
if [[ $FAILED_COUNT -gt 0 ]]; then
    log_error "Some branches failed to merge"
    log_error "Manual conflict resolution required"
    exit 1
fi

log_info "✓ All branches merged successfully"
exit 0
