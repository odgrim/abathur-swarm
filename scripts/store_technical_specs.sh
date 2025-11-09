#!/usr/bin/env bash
# store_technical_specs.sh - Store technical specifications in memory
#
# Usage: ./store_technical_specs.sh <task_id> <memory_key>
#
# This hook stores the technical specs output from create_technical_specs step
# into the MCP memory system for future reference.
#
# Input: Step output via stdin or ABATHUR_STEP_OUTPUT environment variable

set -euo pipefail

TASK_ID="${1:-}"
MEMORY_KEY="${2:-technical_specs}"

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

log_info "Storing technical specs in memory for task: $TASK_ID"

# Get step output from stdin or environment variable
if [[ -n "${ABATHUR_STEP_OUTPUT:-}" ]]; then
    STEP_OUTPUT="$ABATHUR_STEP_OUTPUT"
else
    log_info "Reading step output from stdin..."
    STEP_OUTPUT=$(cat)
fi

if [[ -z "$STEP_OUTPUT" ]]; then
    log_error "No step output provided"
    exit 1
fi

# Validate that output is valid JSON
if ! echo "$STEP_OUTPUT" | jq empty 2>/dev/null; then
    log_error "Step output is not valid JSON"
    log_error "Output: $STEP_OUTPUT"
    exit 1
fi

log_info "Step output is valid JSON"

# Check if abathur CLI is available
if ! command -v abathur &> /dev/null; then
    log_error "abathur CLI not found in PATH"
    exit 1
fi

# Store in memory using abathur CLI
NAMESPACE="task:${TASK_ID}:${MEMORY_KEY}"
log_info "Storing in memory namespace: $NAMESPACE"

# Use jq to properly escape the JSON for the CLI
ESCAPED_JSON=$(echo "$STEP_OUTPUT" | jq -c .)

if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "data" \
    --value "$ESCAPED_JSON" \
    --type "semantic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Technical specs stored successfully in memory"
    log_info "  Namespace: $NAMESPACE"
    log_info "  Key: data"
else
    log_error "Failed to store technical specs in memory"
    exit 1
fi

# Also store a timestamp
TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "timestamp" \
    --value "\"$TIMESTAMP\"" \
    --type "episodic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Timestamp stored: $TIMESTAMP"
fi

# Extract and store key metadata for easy access
COMPONENT_COUNT=$(echo "$STEP_OUTPUT" | jq -r '.components | length // 0')
if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "component_count" \
    --value "$COMPONENT_COUNT" \
    --type "semantic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Component count stored: $COMPONENT_COUNT"
fi

PHASE_COUNT=$(echo "$STEP_OUTPUT" | jq -r '.implementation_phases | length // 0')
if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "phase_count" \
    --value "$PHASE_COUNT" \
    --type "semantic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Phase count stored: $PHASE_COUNT"
fi

FEATURE_BRANCH=$(echo "$STEP_OUTPUT" | jq -r '.feature_branch // "feature/unknown"')
if abathur memory add \
    --namespace "$NAMESPACE" \
    --key "feature_branch" \
    --value "\"$FEATURE_BRANCH\"" \
    --type "semantic" \
    --created-by "technical_feature_workflow"; then
    log_info "✓ Feature branch stored: $FEATURE_BRANCH"
fi

log_info "✓ Technical specs storage complete for task $TASK_ID"
exit 0
