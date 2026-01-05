#!/usr/bin/env bash
# test_no_duplicate_agents.sh - Tests that no duplicate agent names exist across directories
#
# This test validates that:
# 1. No agent with the same filename exists in both abathur/ and workers/ directories
# 2. All agent files have unique names across the entire .claude/agents/ tree
#
# Prevents regression of duplicate agent creation issues.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$(dirname "$SCRIPT_DIR")")"
AGENTS_DIR="${PROJECT_ROOT}/.claude/agents"
TEMPLATE_AGENTS_DIR="${PROJECT_ROOT}/template/.claude/agents"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

FAILED=0

echo "=== Duplicate Agent Detection Tests ==="
echo ""

# Test 1: Check project agents directory exists
echo -n "Test 1: Project agents directory exists... "
if [[ -d "$AGENTS_DIR" ]]; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${YELLOW}SKIP${NC} - Directory not found at $AGENTS_DIR"
    exit 0
fi

# Test 2: No duplicate agent filenames between abathur/ and workers/
echo -n "Test 2: No duplicate agents between abathur/ and workers/... "
ABATHUR_DIR="${AGENTS_DIR}/abathur"
WORKERS_DIR="${AGENTS_DIR}/workers"

if [[ -d "$ABATHUR_DIR" ]] && [[ -d "$WORKERS_DIR" ]]; then
    DUPLICATES=""
    for abathur_agent in "$ABATHUR_DIR"/*.md; do
        if [[ -f "$abathur_agent" ]]; then
            agent_name=$(basename "$abathur_agent")
            if [[ -f "${WORKERS_DIR}/${agent_name}" ]]; then
                DUPLICATES="${DUPLICATES}  - ${agent_name}\n"
            fi
        fi
    done

    if [[ -n "$DUPLICATES" ]]; then
        echo -e "${RED}FAIL${NC}"
        echo -e "  Duplicate agents found in both abathur/ and workers/:"
        echo -e "$DUPLICATES"
        echo "  Each agent should only exist in ONE directory."
        echo "  - Core orchestration agents go in abathur/"
        echo "  - Worker/specialist agents go in workers/"
        FAILED=1
    else
        echo -e "${GREEN}PASS${NC}"
    fi
else
    echo -e "${YELLOW}SKIP${NC} - One or both directories not found"
fi

# Test 3: No duplicate agent 'name' field values across all agents
echo -n "Test 3: No duplicate agent 'name' field values... "
ALL_NAMES=""
DUPLICATE_NAMES=""

for agent_file in "$AGENTS_DIR"/**/*.md; do
    if [[ -f "$agent_file" ]]; then
        # Extract the 'name:' field from the YAML frontmatter
        agent_name=$(grep -E "^name:" "$agent_file" 2>/dev/null | head -1 | sed 's/name: *//' | tr -d '"' || true)
        if [[ -n "$agent_name" ]]; then
            # Check if this name already exists
            if echo "$ALL_NAMES" | grep -qF "$agent_name"; then
                DUPLICATE_NAMES="${DUPLICATE_NAMES}  - ${agent_name} (found in: ${agent_file})\n"
            else
                ALL_NAMES="${ALL_NAMES}${agent_name}\n"
            fi
        fi
    fi
done

if [[ -n "$DUPLICATE_NAMES" ]]; then
    echo -e "${RED}FAIL${NC}"
    echo -e "  Duplicate agent names found:"
    echo -e "$DUPLICATE_NAMES"
    echo "  Each agent must have a unique 'name' field value."
    FAILED=1
else
    echo -e "${GREEN}PASS${NC}"
fi

# Test 4: Check template directory for duplicates (if exists)
echo -n "Test 4: No duplicates in template agents directory... "
if [[ -d "$TEMPLATE_AGENTS_DIR" ]]; then
    TEMPLATE_ABATHUR="${TEMPLATE_AGENTS_DIR}/abathur"
    TEMPLATE_WORKERS="${TEMPLATE_AGENTS_DIR}/workers"

    if [[ -d "$TEMPLATE_ABATHUR" ]] && [[ -d "$TEMPLATE_WORKERS" ]]; then
        DUPLICATES=""
        for template_agent in "$TEMPLATE_ABATHUR"/*.md; do
            if [[ -f "$template_agent" ]]; then
                agent_name=$(basename "$template_agent")
                if [[ -f "${TEMPLATE_WORKERS}/${agent_name}" ]]; then
                    DUPLICATES="${DUPLICATES}  - ${agent_name}\n"
                fi
            fi
        done

        if [[ -n "$DUPLICATES" ]]; then
            echo -e "${RED}FAIL${NC}"
            echo -e "  Duplicate agents found in template:"
            echo -e "$DUPLICATES"
            FAILED=1
        else
            echo -e "${GREEN}PASS${NC}"
        fi
    else
        echo -e "${GREEN}PASS${NC} (one or both template subdirs not found)"
    fi
else
    echo -e "${YELLOW}SKIP${NC} - Template agents directory not found"
fi

# Test 5: Verify agent-creator has duplicate prevention instructions
echo -n "Test 5: agent-creator has duplicate prevention checklist... "
AGENT_CREATOR="${AGENTS_DIR}/abathur/agent-creator.md"
if [[ -f "$AGENT_CREATOR" ]]; then
    if grep -q "Duplicate Prevention Checklist" "$AGENT_CREATOR"; then
        echo -e "${GREEN}PASS${NC}"
    else
        echo -e "${RED}FAIL${NC}"
        echo "  agent-creator.md is missing 'Duplicate Prevention Checklist' section"
        FAILED=1
    fi
else
    echo -e "${YELLOW}SKIP${NC} - agent-creator.md not found"
fi

echo ""
echo "=== Summary ==="
if [[ $FAILED -eq 0 ]]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed!${NC}"
    exit 1
fi
