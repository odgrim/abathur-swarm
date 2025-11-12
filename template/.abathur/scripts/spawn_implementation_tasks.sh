#!/usr/bin/env bash
# spawn_implementation_tasks.sh - Spawn implementation tasks via MCP task queue
#
# Usage: ./spawn_implementation_tasks.sh <task_id> <tasks_json>
#
# This hook reads the task plan and spawns individual implementation tasks
# via the MCP task queue with proper dependencies and agent assignments.
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

log_info "Spawning implementation tasks for parent task: $TASK_ID"

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

# Check if abathur CLI is available
if ! command -v abathur &> /dev/null; then
    log_error "abathur CLI not found in PATH"
    exit 1
fi

# Extract tasks array
TASKS=$(echo "$TASKS_JSON" | jq -c '.tasks[]')
TASK_COUNT=$(echo "$TASKS_JSON" | jq -r '.tasks | length')

log_info "Found $TASK_COUNT tasks to spawn"

# Create a map to store task IDs to spawned task IDs
declare -A TASK_ID_MAP

# Spawn tasks
SPAWNED_COUNT=0
FAILED_COUNT=0

while IFS= read -r task_json; do
    SUBTASK_ID=$(echo "$task_json" | jq -r '.id')
    SUMMARY=$(echo "$task_json" | jq -r '.summary')
    DESCRIPTION=$(echo "$task_json" | jq -r '.description')
    AGENT_TYPE=$(echo "$task_json" | jq -r '.agent_type')
    PHASE=$(echo "$task_json" | jq -r '.phase // 1')
    DEPENDENCIES=$(echo "$task_json" | jq -r '.dependencies // []')

    log_info "Spawning task: $SUBTASK_ID"
    log_info "  Summary: $SUMMARY"
    log_info "  Agent: $AGENT_TYPE"
    log_info "  Phase: $PHASE"

    # Build dependency arguments
    DEPENDENCY_ARGS=""
    if [[ "$DEPENDENCIES" != "[]" ]]; then
        # Map dependency IDs to spawned task IDs
        MAPPED_DEPS=""
        while IFS= read -r dep_id; do
            if [[ -n "${TASK_ID_MAP[$dep_id]:-}" ]]; then
                if [[ -n "$MAPPED_DEPS" ]]; then
                    MAPPED_DEPS="$MAPPED_DEPS,${TASK_ID_MAP[$dep_id]}"
                else
                    MAPPED_DEPS="${TASK_ID_MAP[$dep_id]}"
                fi
            else
                log_warn "  Dependency $dep_id not found in spawned tasks (may not exist yet)"
            fi
        done < <(echo "$DEPENDENCIES" | jq -r '.[]')

        if [[ -n "$MAPPED_DEPS" ]]; then
            DEPENDENCY_ARGS="--dependencies $MAPPED_DEPS"
        fi
    fi

    # Spawn the task
    if SPAWN_OUTPUT=$(abathur task enqueue \
        --summary "$SUMMARY" \
        --description "$DESCRIPTION" \
        --agent-type "$AGENT_TYPE" \
        --parent-task-id "$TASK_ID" \
        --priority 5 \
        $DEPENDENCY_ARGS 2>&1); then

        # Extract the spawned task ID from the output
        SPAWNED_TASK_ID=$(echo "$SPAWN_OUTPUT" | grep -o 'task-[a-f0-9-]*' | head -1 || echo "unknown")

        log_info "  ✓ Spawned as task: $SPAWNED_TASK_ID"
        ((SPAWNED_COUNT++))

        # Store mapping
        TASK_ID_MAP[$SUBTASK_ID]="$SPAWNED_TASK_ID"

        # Store spawned task info in memory
        SUB_NAMESPACE="task:${TASK_ID}:subtask:${SUBTASK_ID}"
        abathur memory add \
            --namespace "$SUB_NAMESPACE" \
            --key "spawned_task_id" \
            --value "\"$SPAWNED_TASK_ID\"" \
            --type "episodic" \
            --created-by "technical_feature_workflow" 2>/dev/null || true
    else
        log_error "  Failed to spawn task: $SUBTASK_ID"
        log_error "  Error: $SPAWN_OUTPUT"
        ((FAILED_COUNT++))
    fi
done <<< "$TASKS"

log_info "✓ Task spawning complete"
log_info "  Spawned: $SPAWNED_COUNT"
log_info "  Failed: $FAILED_COUNT"

# Store summary in memory
NAMESPACE="task:${TASK_ID}:spawn_summary"
SUMMARY_JSON=$(jq -n \
    --arg spawned "$SPAWNED_COUNT" \
    --arg failed "$FAILED_COUNT" \
    --arg total "$TASK_COUNT" \
    '{spawned: ($spawned | tonumber), failed: ($failed | tonumber), total: ($total | tonumber)}')

abathur memory add \
    --namespace "$NAMESPACE" \
    --key "summary" \
    --value "$SUMMARY_JSON" \
    --type "semantic" \
    --created-by "technical_feature_workflow" 2>/dev/null || true

if [[ $FAILED_COUNT -gt 0 ]]; then
    log_error "Some tasks failed to spawn"
    exit 1
fi

exit 0
