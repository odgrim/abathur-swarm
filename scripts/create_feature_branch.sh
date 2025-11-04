#!/usr/bin/env bash
# Create feature branch for a task
# Usage: create_feature_branch.sh <task_id> <strategy>

set -euo pipefail

TASK_ID="${1:?Task ID required}"
STRATEGY="${2:-single}"
BRANCH_NAME="feature/${TASK_ID}"

echo "Creating feature branch: ${BRANCH_NAME}"

# Check if branch already exists
if git rev-parse --verify "${BRANCH_NAME}" >/dev/null 2>&1; then
    echo "Branch ${BRANCH_NAME} already exists, checking it out"
    git checkout "${BRANCH_NAME}"
else
    # Create new branch from main
    git checkout -b "${BRANCH_NAME}" main
    echo "Created new feature branch: ${BRANCH_NAME}"
fi

# Log the branch creation
echo "Feature branch ready: ${BRANCH_NAME} (strategy: ${STRATEGY})"
