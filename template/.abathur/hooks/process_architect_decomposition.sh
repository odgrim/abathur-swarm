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

# Read the architecture from memory
# Expected namespace: task:{task_id}:architecture
# Expected key: overview
ARCH_JSON=$(abathur memory get --namespace "task:${TASK_ID}:architecture" --key "overview" 2>/dev/null || echo "")

if [[ -z "$ARCH_JSON" || "$ARCH_JSON" == "null" ]]; then
    echo "[WARN] No architecture found in memory at task:${TASK_ID}:architecture:overview"
    echo "[INFO] Cannot process decomposition without architecture"
    exit 0
fi

echo "[INFO] Found architecture in memory"

# Extract decomposition information
DECOMPOSITION_STRATEGY=$(echo "$ARCH_JSON" | jq -r '.decomposition.strategy // "single"' 2>/dev/null || echo "single")
SUBPROJECTS=$(echo "$ARCH_JSON" | jq -r '.decomposition.subprojects // []' 2>/dev/null || echo "[]")
SUBPROJECT_COUNT=$(echo "$SUBPROJECTS" | jq '. | length' 2>/dev/null || echo "0")

echo "[INFO] Decomposition strategy: $DECOMPOSITION_STRATEGY"
echo "[INFO] Subproject count: $SUBPROJECT_COUNT"

if [[ "$SUBPROJECT_COUNT" -eq 0 ]]; then
    echo "[WARN] No subprojects found in decomposition"
    exit 0
fi

echo "[INFO] Processing $SUBPROJECT_COUNT subproject(s)"

# Process each subproject
for i in $(seq 0 $((SUBPROJECT_COUNT - 1))); do
    FEATURE_NAME=$(echo "$SUBPROJECTS" | jq -r ".[$i].name // .[$i]" 2>/dev/null || echo "")
    FEATURE_DESC=$(echo "$SUBPROJECTS" | jq -r ".[$i].description // \"\"" 2>/dev/null || echo "")
    FEATURE_SCOPE=$(echo "$SUBPROJECTS" | jq -r ".[$i].scope // \"\"" 2>/dev/null || echo "")
    FEATURE_PRIORITY=7

    if [[ -z "$FEATURE_NAME" || "$FEATURE_NAME" == "null" ]]; then
        echo "[WARN] Feature at index $i has no name, skipping"
        continue
    fi

    echo ""
    echo "[INFO] Processing feature $((i + 1))/$SUBPROJECT_COUNT: $FEATURE_NAME"

    # Sanitize feature name (remove spaces, special chars, lowercase)
    FEATURE_NAME_CLEAN=$(echo "$FEATURE_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

    # Truncate to reasonable length
    FEATURE_NAME_CLEAN=$(echo "$FEATURE_NAME_CLEAN" | cut -c1-50)

    FEATURE_BRANCH="feature/${FEATURE_NAME_CLEAN}"
    WORKTREE_PATH=".abathur/worktrees/feature-${FEATURE_NAME_CLEAN}"

    echo "[INFO]   Sanitized name: $FEATURE_NAME_CLEAN"
    echo "[INFO]   Branch: $FEATURE_BRANCH"
    echo "[INFO]   Worktree: $WORKTREE_PATH"

    # Branches are already created by create_feature_branch.sh, so just verify
    if ! git show-ref --verify --quiet "refs/heads/$FEATURE_BRANCH"; then
        echo "[WARN]   Branch $FEATURE_BRANCH does not exist (should have been created by create_feature_branch.sh)"
        echo "[INFO]   Creating branch now"
        git worktree add -b "$FEATURE_BRANCH" "$WORKTREE_PATH" 2>&1 | sed 's/^/[GIT]     /' || echo "[ERROR] Failed to create branch"
    else
        echo "[INFO]   Branch $FEATURE_BRANCH exists"
    fi

    # Prepare task summary and description
    TASK_SUMMARY="${FEATURE_NAME}: Technical requirements"

    TASK_DESC="Subproject: ${FEATURE_NAME}
Description: ${FEATURE_DESC}
Scope: ${FEATURE_SCOPE}

Architecture in memory: task:${TASK_ID}:architecture
Requirements in memory: task:${TASK_ID}:requirements (if available)

Your mission:
1. Load architecture from parent task's memory (task:${TASK_ID}:architecture)
2. Define detailed technical specifications for THIS subproject
3. Create data models and API specifications
4. Plan implementation phases
5. Identify required specialized agents

Expected Deliverables:
- Detailed technical specifications for ${FEATURE_NAME}
- Implementation plan with phases
- Suggested agent specializations"

    # Submit technical-requirements-specialist task with feature_branch set
    echo "[INFO]   Spawning technical-requirements-specialist task"
    echo "[INFO]     Summary: $TASK_SUMMARY"
    echo "[INFO]     Priority: $FEATURE_PRIORITY"
    echo "[INFO]     Feature branch: $FEATURE_BRANCH"
    echo "[INFO]     Parent task: $TASK_ID"

    # Use abathur task submit with --feature-branch flag
    # NOTE: We don't use --chain here because the progression is handled by hooks
    TASK_OUTPUT=$(abathur task submit \
        --agent-type technical-requirements-specialist \
        --summary "$TASK_SUMMARY" \
        --priority "$FEATURE_PRIORITY" \
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
echo "[INFO] ✓ Completed processing decomposition - $SUBPROJECT_COUNT features processed"
exit 0
