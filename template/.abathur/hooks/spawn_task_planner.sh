#!/usr/bin/env bash
# spawn_task_planner.sh - Create task branch and spawn task-planner
#
# Usage: ./spawn_task_planner.sh <tech_req_spec_task_id> <feature_branch>
#
# This hook is triggered when technical-requirements-specialist completes (PostComplete).
# It creates a dedicated task branch for the task-planner and spawns it.

set -euo pipefail

TECH_REQ_TASK_ID="${1:-}"
FEATURE_BRANCH="${2:-}"

if [[ -z "$TECH_REQ_TASK_ID" || -z "$FEATURE_BRANCH" ]]; then
    echo "[ERROR] Usage: $0 <tech_req_spec_task_id> <feature_branch>"
    exit 1
fi

echo "[INFO] Spawning task-planner for technical-requirements-specialist $TECH_REQ_TASK_ID"
echo "[INFO]   Feature Branch: $FEATURE_BRANCH"

# Extract feature name from feature_branch (e.g., "feature/user-auth" -> "user-auth")
FEATURE_NAME="${FEATURE_BRANCH#feature/}"
if [[ "$FEATURE_NAME" == "$FEATURE_BRANCH" ]]; then
    # Fallback if not in feature/ format
    FEATURE_NAME="unknown"
fi

# Generate short task ID (first 8 chars)
TECH_REQ_SHORT="${TECH_REQ_TASK_ID:0:8}"

# Generate branch name for task-planner: task/{feature_name}/plan-{tech-req-short-id}
TASK_BRANCH="task/${FEATURE_NAME}/plan-${TECH_REQ_SHORT}"

echo "[INFO] Creating task branch: $TASK_BRANCH"

# Verify feature branch exists
if ! git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
    echo "[ERROR] Feature branch $FEATURE_BRANCH does not exist"
    echo "[ERROR] Cannot create task branch without feature branch"
    exit 1
fi

# Check if task branch already exists
if git show-ref --verify --quiet "refs/heads/$TASK_BRANCH"; then
    echo "[WARN] Branch $TASK_BRANCH already exists"
    echo "[INFO] Will use existing branch"
else
    echo "[INFO] Creating new branch $TASK_BRANCH from $FEATURE_BRANCH"
    if ! git branch "$TASK_BRANCH" "$FEATURE_BRANCH" 2>&1 | sed 's/^/[GIT]   /'; then
        echo "[ERROR] Failed to create branch $TASK_BRANCH"
        exit 1
    fi
    echo "[INFO] ✓ Branch $TASK_BRANCH created successfully"
fi

# Prepare task description
TASK_SUMMARY="Task planning for feature ${FEATURE_NAME}"

TASK_DESC="Feature branch: ${FEATURE_BRANCH}
Task branch: ${TASK_BRANCH}
Technical specs in memory: task:${TECH_REQ_TASK_ID}:technical_specs

Your mission:
1. Load technical specifications from memory
2. Decompose work into atomic implementation tasks
3. Identify required specialized agents
4. Spawn agent-creator for missing agents
5. Spawn implementation tasks with proper dependencies
6. Workflow will automatically spawn validation and merge tasks

Expected Deliverables:
- Atomic task breakdown
- Agent creation tasks (if needed)
- Implementation tasks with dependencies

IMPORTANT: Your task branch is ${TASK_BRANCH}. All implementation tasks you spawn should branch from this task branch."

echo "[INFO] Spawning task-planner task"
echo "[INFO]   Summary: $TASK_SUMMARY"
echo "[INFO]   Feature Branch: $FEATURE_BRANCH"
echo "[INFO]   Task Branch: $TASK_BRANCH"
echo "[INFO]   Parent Task: $TECH_REQ_TASK_ID"

# Check if abathur CLI is available
if ! command -v abathur &> /dev/null; then
    echo "[ERROR] abathur CLI not found in PATH"
    exit 1
fi

# Submit task-planner task with feature_branch
# The PreStart hook will detect the existing task branch and create worktree
TASK_OUTPUT=$(abathur task submit \
    --agent-type task-planner \
    --summary "$TASK_SUMMARY" \
    --priority 6 \
    --feature-branch "$FEATURE_BRANCH" \
    --dependencies "$TECH_REQ_TASK_ID" \
    --chain none \
    "$TASK_DESC" 2>&1)

if [[ $? -eq 0 ]]; then
    # Extract task ID from output if possible
    NEW_TASK_ID=$(echo "$TASK_OUTPUT" | grep -o '[0-9a-f]\{8\}-[0-9a-f]\{4\}-[0-9a-f]\{4\}-[0-9a-f]\{4\}-[0-9a-f]\{12\}\|[0-9a-f]\{8\}' | head -1 || echo "")
    if [[ -n "$NEW_TASK_ID" ]]; then
        echo "[INFO] ✓ Task-planner spawned successfully: $NEW_TASK_ID"
        echo "[INFO]   The PreStart hook will create worktree from branch $TASK_BRANCH"
    else
        echo "[INFO] ✓ Task-planner spawned successfully"
    fi
else
    echo "[ERROR] Failed to spawn task-planner: $TASK_OUTPUT"
    exit 1
fi

echo "[INFO] ✓ Task-planner spawning complete"
exit 0
