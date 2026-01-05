#!/usr/bin/env bash
# test_process_architect_decomposition.sh - Tests for process_architect_decomposition.sh
#
# This test validates that the shell script uses correct variable names
# to prevent regressions like using $FEATURE_COUNT instead of $SUBPROJECT_COUNT

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_DIR="$(dirname "$(dirname "$SCRIPT_DIR")")/template"
SCRIPT_PATH="${TEMPLATE_DIR}/.abathur/hooks/process_architect_decomposition.sh"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

FAILED=0

# Test 1: Verify script exists
echo -n "Test 1: Script exists... "
if [[ -f "$SCRIPT_PATH" ]]; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL${NC} - Script not found at $SCRIPT_PATH"
    exit 1
fi

# Test 2: Verify no undefined FEATURE_COUNT variable
echo -n "Test 2: No undefined FEATURE_COUNT variable... "
# The script defines SUBPROJECT_COUNT, not FEATURE_COUNT
# So any use of FEATURE_COUNT is a bug
if grep -q '\$FEATURE_COUNT' "$SCRIPT_PATH" 2>/dev/null; then
    echo -e "${RED}FAIL${NC}"
    echo "  Found undefined variable \$FEATURE_COUNT in script"
    echo "  This should be \$SUBPROJECT_COUNT"
    grep -n '\$FEATURE_COUNT' "$SCRIPT_PATH"
    FAILED=1
else
    echo -e "${GREEN}PASS${NC}"
fi

# Test 3: Verify SUBPROJECT_COUNT is defined
echo -n "Test 3: SUBPROJECT_COUNT is defined... "
if grep -q 'SUBPROJECT_COUNT=' "$SCRIPT_PATH" 2>/dev/null; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL${NC} - SUBPROJECT_COUNT variable is not defined"
    FAILED=1
fi

# Test 4: Verify SUBPROJECT_COUNT is used in output messages
echo -n "Test 4: SUBPROJECT_COUNT is used in output messages... "
USAGE_COUNT=$(grep -c '\$SUBPROJECT_COUNT' "$SCRIPT_PATH" 2>/dev/null || echo "0")
if [[ "$USAGE_COUNT" -ge 2 ]]; then
    echo -e "${GREEN}PASS${NC} ($USAGE_COUNT usages found)"
else
    echo -e "${RED}FAIL${NC} - Expected at least 2 usages of \$SUBPROJECT_COUNT"
    FAILED=1
fi

# Test 5: Verify script is syntactically valid bash
echo -n "Test 5: Script has valid bash syntax... "
if bash -n "$SCRIPT_PATH" 2>/dev/null; then
    echo -e "${GREEN}PASS${NC}"
else
    echo -e "${RED}FAIL${NC} - Script has syntax errors"
    FAILED=1
fi

echo ""
if [[ $FAILED -eq 0 ]]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed!${NC}"
    exit 1
fi
