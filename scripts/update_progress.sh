#!/usr/bin/env bash
# update_progress.sh - Update task progress in memory
#
# Usage: ./update_progress.sh <task_id> <progress_percentage>
#
# This hook updates the progress percentage for a task in memory.
# It can read progress from arguments or extract it from JSON input.
#
# Input: Step output via stdin or ABATHUR_STEP_OUTPUT environment variable

set -euo pipefail

TASK_ID="${1:-}"
PROGRESS_ARG="${2:-}"

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

log_info "Updating progress for task: $TASK_ID"

# Get progress from various sources
PROGRESS=""

# First try the argument
if [[ -n "$PROGRESS_ARG" ]] && [[ "$PROGRESS_ARG" =~ ^[0-9]+$ ]]; then
    PROGRESS="$PROGRESS_ARG"
else
    # Try to get from JSON input
    if [[ -n "${ABATHUR_STEP_OUTPUT:-}" ]]; then
        STEP_OUTPUT="$ABATHUR_STEP_OUTPUT"
    else
        log_info "Reading step output from stdin..."
        STEP_OUTPUT=$(cat || echo "")
    fi

    if [[ -n "$STEP_OUTPUT" ]]; then
        # Try to extract progress_percentage from JSON
        if echo "$STEP_OUTPUT" | jq empty 2>/dev/null; then
            PROGRESS=$(echo "$STEP_OUTPUT" | jq -r '.progress_percentage // empty')
        fi
    fi
fi

if [[ -z "$PROGRESS" ]]; then
    log_error "Could not determine progress percentage"
    log_error "Provide as argument or in JSON with 'progress_percentage' field"
    exit 1
fi

# Validate progress is a number between 0 and 100
if ! [[ "$PROGRESS" =~ ^[0-9]+$ ]] || [[ $PROGRESS -lt 0 ]] || [[ $PROGRESS -gt 100 ]]; then
    log_error "Progress must be an integer between 0 and 100, got: $PROGRESS"
    exit 1
fi

log_info "Progress: $PROGRESS%"

# Check if abathur CLI is available
if ! command -v abathur &> /dev/null; then
    log_error "abathur CLI not found in PATH"
    exit 1
fi

# Store progress in memory
NAMESPACE="task:${TASK_ID}:progress"
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# Store current progress
if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "percentage" \
    --value "$PROGRESS" \
    --type "semantic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Progress updated: $PROGRESS%"
else
    log_error "Failed to update progress in memory"
    exit 1
fi

# Store timestamp
if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "last_updated" \
    --value "\"$TIMESTAMP\"" \
    --type "episodic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Timestamp updated: $TIMESTAMP"
fi

# Store progress status based on percentage
STATUS="in_progress"
if [[ $PROGRESS -eq 0 ]]; then
    STATUS="not_started"
elif [[ $PROGRESS -eq 100 ]]; then
    STATUS="completed"
fi

abathur memory add \
    --namespace "$NAMESPACE" \
    --key "status" \
    --value "\"$STATUS\"" \
    --type "semantic" \
    --created-by "technical_feature_workflow" 2>/dev/null || true

# Also try to extract and store task counts if available
if [[ -n "${STEP_OUTPUT:-}" ]]; then
    if echo "$STEP_OUTPUT" | jq empty 2>/dev/null; then
        TOTAL=$(echo "$STEP_OUTPUT" | jq -r '.total_tasks // empty')
        COMPLETED=$(echo "$STEP_OUTPUT" | jq -r '.completed // empty')
        IN_PROGRESS=$(echo "$STEP_OUTPUT" | jq -r '.in_progress // empty')
        BLOCKED=$(echo "$STEP_OUTPUT" | jq -r '.blocked // empty')
        FAILED=$(echo "$STEP_OUTPUT" | jq -r '.failed // empty')

        if [[ -n "$TOTAL" ]]; then
            TASK_COUNTS=$(jq -n \
                --arg total "$TOTAL" \
                --arg completed "$COMPLETED" \
                --arg in_progress "$IN_PROGRESS" \
                --arg blocked "$BLOCKED" \
                --arg failed "$FAILED" \
                '{
                    total: ($total | tonumber),
                    completed: ($completed | tonumber),
                    in_progress: ($in_progress | tonumber),
                    blocked: ($blocked | tonumber),
                    failed: ($failed | tonumber)
                }')

            abathur memory add \
                --namespace "$NAMESPACE" \
                --key "task_counts" \
                --value "$TASK_COUNTS" \
                --type "semantic" \
                --created-by "technical_feature_workflow" 2>/dev/null || true

            log_info "✓ Task counts stored: $COMPLETED/$TOTAL completed"
        fi
    fi
fi

log_info "✓ Progress update complete for task $TASK_ID"
exit 0
