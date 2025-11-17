#!/usr/bin/env bash
# process_architect_decomposition.sh - Process technical-architect decomposition
#
# Usage: ./process_architect_decomposition.sh <task_id>
#
# This hook is triggered when technical-architect completes (PostComplete).
# It reads the decomposition plan from memory, creates feature branches,
# and spawns technical-requirements-specialist tasks with feature_branch already set.

set -euo pipefail

TASK_ID="${1:-}"

if [[ -z "$TASK_ID" ]]; then
    echo "[ERROR] Usage: $0 <task_id>"
    exit 1
fi

echo "[INFO] Processing technical-architect decomposition for task $TASK_ID"

# Check if abathur CLI is available
if ! command -v abathur &> /dev/null; then
    echo "[ERROR] abathur CLI not found in PATH"
    exit 1
fi

# Read the decomposition plan from memory
# Expected namespace: task:{task_id}:decomposition
# Expected key: plan
# Expected format: JSON array of features
DECOMPOSITION_JSON=$(abathur memory get --namespace "task:${TASK_ID}:decomposition" --key "plan" 2>/dev/null || echo "")

if [[ -z "$DECOMPOSITION_JSON" || "$DECOMPOSITION_JSON" == "null" ]]; then
    echo "[WARN] No decomposition plan found in memory at task:${TASK_ID}:decomposition:plan"
    echo "[INFO] This may be a Mode 1 (single feature) scenario - no action needed"
    exit 0
fi

echo "[INFO] Found decomposition plan in memory"
echo "[DEBUG] Plan: $DECOMPOSITION_JSON"

# Parse JSON array and process each feature
# We expect the JSON to be an array of objects with at least a "name" field
# Example: [{"name": "user-auth", "description": "User authentication"}, ...]

# Count features
FEATURE_COUNT=$(echo "$DECOMPOSITION_JSON" | jq '. | length' 2>/dev/null || echo "0")

if [[ "$FEATURE_COUNT" -eq 0 ]]; then
    echo "[WARN] Decomposition plan is empty or invalid"
    exit 0
fi

echo "[INFO] Processing $FEATURE_COUNT features"

# Process each feature
for i in $(seq 0 $((FEATURE_COUNT - 1))); do
    FEATURE_NAME=$(echo "$DECOMPOSITION_JSON" | jq -r ".[$i].name" 2>/dev/null || echo "")
    FEATURE_SUMMARY=$(echo "$DECOMPOSITION_JSON" | jq -r ".[$i].summary" 2>/dev/null || echo "")
    FEATURE_DESC=$(echo "$DECOMPOSITION_JSON" | jq -r ".[$i].description" 2>/dev/null || echo "")
    FEATURE_PRIORITY=$(echo "$DECOMPOSITION_JSON" | jq -r ".[$i].priority // 7" 2>/dev/null || echo "7")

    if [[ -z "$FEATURE_NAME" || "$FEATURE_NAME" == "null" ]]; then
        echo "[WARN] Feature at index $i has no name, skipping"
        continue
    fi

    echo ""
    echo "[INFO] Processing feature $((i + 1))/$FEATURE_COUNT: $FEATURE_NAME"

    # Sanitize feature name (remove spaces, special chars, lowercase)
    FEATURE_NAME_CLEAN=$(echo "$FEATURE_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

    # Truncate to reasonable length
    FEATURE_NAME_CLEAN=$(echo "$FEATURE_NAME_CLEAN" | cut -c1-50)

    FEATURE_BRANCH="feature/${FEATURE_NAME_CLEAN}"
    WORKTREE_PATH=".abathur/worktrees/feature-${FEATURE_NAME_CLEAN}"

    echo "[INFO]   Sanitized name: $FEATURE_NAME_CLEAN"
    echo "[INFO]   Branch: $FEATURE_BRANCH"
    echo "[INFO]   Worktree: $WORKTREE_PATH"

    # Check if worktree already exists
    if [[ -d "$WORKTREE_PATH" ]]; then
        echo "[WARN]   Worktree already exists at $WORKTREE_PATH, reusing"
    else
        # Check if branch already exists
        if git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
            echo "[INFO]   Branch $FEATURE_BRANCH already exists, creating worktree"
            git worktree add "$WORKTREE_PATH" "$FEATURE_BRANCH" 2>&1 | sed 's/^/[GIT]     /'
        else
            echo "[INFO]   Creating new feature branch and worktree"
            git worktree add -b "$FEATURE_BRANCH" "$WORKTREE_PATH" 2>&1 | sed 's/^/[GIT]     /'
        fi
        echo "[INFO]   ✓ Feature branch created successfully"
    fi

    # Prepare task summary and description
    if [[ -z "$FEATURE_SUMMARY" || "$FEATURE_SUMMARY" == "null" ]]; then
        TASK_SUMMARY="${FEATURE_NAME}: Technical requirements"
    else
        TASK_SUMMARY="$FEATURE_SUMMARY"
    fi

    if [[ -z "$FEATURE_DESC" || "$FEATURE_DESC" == "null" ]]; then
        TASK_DESC="Technical requirements and specifications for ${FEATURE_NAME}. Architecture stored in memory: task:${TASK_ID}:architecture"
    else
        TASK_DESC="$FEATURE_DESC"
    fi

    # Submit technical-requirements-specialist task with feature_branch set
    echo "[INFO]   Spawning technical-requirements-specialist task"
    echo "[INFO]     Summary: $TASK_SUMMARY"
    echo "[INFO]     Priority: $FEATURE_PRIORITY"
    echo "[INFO]     Feature branch: $FEATURE_BRANCH"
    echo "[INFO]     Parent task: $TASK_ID"

    # Use abathur task submit with --feature-branch flag
    TASK_OUTPUT=$(abathur task submit \
        --agent-type technical-requirements-specialist \
        --summary "$TASK_SUMMARY" \
        --priority "$FEATURE_PRIORITY" \
        --chain technical_feature_workflow \
        --feature-branch "$FEATURE_BRANCH" \
        --dependencies "$TASK_ID" \
        "$TASK_DESC" 2>&1)

    if [[ $? -eq 0 ]]; then
        # Extract task ID from output if possible
        NEW_TASK_ID=$(echo "$TASK_OUTPUT" | grep -o '[0-9a-f]\{8\}-[0-9a-f]\{4\}-[0-9a-f]\{4\}-[0-9a-f]\{4\}-[0-9a-f]\{12\}' | head -1 || echo "")
        if [[ -n "$NEW_TASK_ID" ]]; then
            echo "[INFO]   ✓ Task spawned successfully: $NEW_TASK_ID"
        else
            echo "[INFO]   ✓ Task spawned successfully"
        fi
    else
        echo "[ERROR]   Failed to spawn task: $TASK_OUTPUT"
    fi
done

echo ""
echo "[INFO] ✓ Completed processing decomposition - $FEATURE_COUNT features processed"
exit 0
