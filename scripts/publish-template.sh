#!/bin/bash

set -e  # Exit on error

# Configuration
TEMPLATE_DIR="./template"
REPO_URL="git@github.com:odgrim/abathur-claude-template.git"
TEMP_DIR=$(mktemp -d)
BRANCH="main"

# Convert TEMPLATE_DIR to absolute path
TEMPLATE_DIR=$(cd "$TEMPLATE_DIR" && pwd)

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== Abathur Template Publisher ===${NC}"
echo ""

# Check if template directory exists
if [ ! -d "$TEMPLATE_DIR" ]; then
    echo -e "${RED}Error: Template directory '$TEMPLATE_DIR' not found${NC}"
    exit 1
fi

echo -e "${GREEN}✓${NC} Found template directory at $TEMPLATE_DIR"

# Clone the template repository to temp directory
echo -e "${BLUE}Cloning template repository...${NC}"
git clone "$REPO_URL" "$TEMP_DIR"
cd "$TEMP_DIR"
git checkout "$BRANCH" 2>/dev/null || git checkout -b "$BRANCH"
echo -e "${GREEN}✓${NC} Repository cloned to: $TEMP_DIR"

# Remove all files except .git directory
echo -e "${BLUE}Clearing existing template files...${NC}"
find . -mindepth 1 -maxdepth 1 ! -name '.git' -exec rm -rf {} +
echo -e "${GREEN}✓${NC} Existing files cleared"

# Copy template to temp directory
echo -e "${BLUE}listing contents of template dir...${NC}"
ls -la "$TEMPLATE_DIR"/*
echo -e "${BLUE}Copying new template files...${NC}"
cp -R "$TEMPLATE_DIR"/* ./ 2>/dev/null || true
cp -R "$TEMPLATE_DIR"/.{gitignore,claude,abathur,mcp.json} ./ 2>/dev/null || true
# Remove . and .. if they were copied
echo -e "${GREEN}✓${NC} Template files copied"

# Verify hooks were copied
echo -e "${BLUE}Verifying hooks...${NC}"
if [ -f ".abathur/hooks.yaml" ]; then
    echo -e "${GREEN}  ✓${NC} hooks.yaml copied"
else
    echo -e "${RED}  ✗${NC} hooks.yaml missing!"
fi
if [ -d ".abathur/hooks" ]; then
    HOOK_COUNT=$(find .abathur/hooks -name "*.sh" -type f | wc -l | tr -d ' ')
    echo -e "${GREEN}  ✓${NC} hooks/ directory copied ($HOOK_COUNT scripts)"
    # Make hook scripts executable
    chmod +x .abathur/hooks/*.sh 2>/dev/null || true
else
    echo -e "${RED}  ✗${NC} hooks/ directory missing!"
fi

# Stage all files
echo -e "${BLUE}Staging files...${NC}"
git add -A
# Force add .abathur directory (may be ignored by git for some reason)
git add -f .abathur/ 2>/dev/null || true
echo -e "${GREEN}✓${NC} Files staged"

# Check if there are changes to commit
if git diff --cached --quiet; then
    echo -e "${BLUE}No changes to commit. Checking if push is needed...${NC}"

    # Try to fetch and check if we need to push
    if git ls-remote --exit-code origin "$BRANCH" > /dev/null 2>&1; then
        git fetch origin "$BRANCH"
        if git diff --quiet HEAD origin/"$BRANCH" 2>/dev/null; then
            echo -e "${GREEN}✓${NC} Repository is already up to date"
            echo ""
            echo -e "${BLUE}Template directory:${NC} $TEMP_DIR"
            echo -e "${BLUE}Remote repository:${NC} $REPO_URL"
            exit 0
        fi
    fi
fi

# Commit changes
echo -e "${BLUE}Creating commit...${NC}"
TIMESTAMP=$(date '+%Y-%m-%d %H:%M:%S')
git commit -m "Update Abathur template - $TIMESTAMP"
echo -e "${GREEN}✓${NC} Commit created"

# Push to remote
echo -e "${BLUE}Pushing to remote repository...${NC}"
git push -u origin "$BRANCH"
echo -e "${GREEN}✓${NC} Pushed to remote"

echo ""
echo -e "${GREEN}=== Publishing Complete ===${NC}"
echo ""
echo -e "${BLUE}Template directory:${NC} $TEMP_DIR"
echo -e "${BLUE}Remote repository:${NC} $REPO_URL"
echo -e "${BLUE}Branch:${NC} $BRANCH"
echo ""
echo -e "${BLUE}Note:${NC} Temporary directory will be cleaned up on next system reboot"
