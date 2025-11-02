#!/usr/bin/env bash
# Validate Project Context Memory Hook
#
# This script validates that the project-context-scanner agent properly saved
# project context to memory. If validation fails, it re-enqueues the scanner task.
#
# Usage: validate_project_context_memory.sh <task_id>

set -euo pipefail

TASK_ID="${1:-}"

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

log_info "Validating project context memory for task: $TASK_ID"

# Check if abathur CLI is available
if ! command -v abathur &> /dev/null; then
    log_error "abathur CLI not found in PATH"
    exit 1
fi

# Query memory to check if project context was saved
log_info "Checking for project context in memory..."

# Try to get the memory entry for project:context/metadata
if ! MEMORY_OUTPUT=$(abathur memory get project:context metadata 2>&1); then
    log_error "Failed to retrieve project context from memory"
    log_error "Output: $MEMORY_OUTPUT"

    # Re-enqueue the project-context-scanner task
    log_warn "Re-enqueueing project-context-scanner task..."

    if abathur task enqueue \
        --summary "Scan project context" \
        --description "Initial project scan to detect language, framework, and conventions (retry after validation failure)." \
        --agent-type "project-context-scanner" \
        --priority 10; then
        log_info "Successfully re-enqueued project-context-scanner task"
    else
        log_error "Failed to re-enqueue project-context-scanner task"
        exit 1
    fi

    exit 1
fi

# Validate that the memory contains required fields
log_info "Validating project context structure..."

# Check if the output contains expected JSON structure
if echo "$MEMORY_OUTPUT" | grep -q '"language"' && \
   echo "$MEMORY_OUTPUT" | grep -q '"frameworks"' && \
   echo "$MEMORY_OUTPUT" | grep -q '"tooling"' && \
   echo "$MEMORY_OUTPUT" | grep -q '"validation_requirements"'; then
    log_info "✓ Project context contains all required fields"
else
    log_error "Project context is missing required fields"
    log_error "Memory output: $MEMORY_OUTPUT"

    # Re-enqueue the project-context-scanner task
    log_warn "Re-enqueueing project-context-scanner task due to incomplete data..."

    if abathur task enqueue \
        --summary "Scan project context" \
        --description "Initial project scan to detect language, framework, and conventions (retry after incomplete data)." \
        --agent-type "project-context-scanner" \
        --priority 10; then
        log_info "Successfully re-enqueued project-context-scanner task"
    else
        log_error "Failed to re-enqueue project-context-scanner task"
        exit 1
    fi

    exit 1
fi

# Validate primary language is set
if echo "$MEMORY_OUTPUT" | grep -q '"primary"'; then
    log_info "✓ Primary language detected"
else
    log_warn "Primary language not detected in project context"
fi

# Validate validation_agent is set
if echo "$MEMORY_OUTPUT" | grep -q '"validation_agent"'; then
    log_info "✓ Validation agent specified"
else
    log_warn "Validation agent not specified in project context"
fi

log_info "✓ Project context memory validation passed for task $TASK_ID"
exit 0
