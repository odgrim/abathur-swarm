#!/bin/bash

set -e  # Exit on error

# Configuration
TEMPLATE_DIR="./template"
REPO_URL="git@github.com:odgrim/abathur-claude-template.git"
TEMP_DIR=$(mktemp -d)
BRANCH="main"

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

echo -e "${GREEN}✓${NC} Found template directory"

# Copy template to temp directory
echo -e "${BLUE}Copying template to temporary directory...${NC}"
cp -R "$TEMPLATE_DIR"/* "$TEMP_DIR/"
cp -R "$TEMPLATE_DIR"/.* "$TEMP_DIR/" 2>/dev/null || true
echo -e "${GREEN}✓${NC} Template copied to: $TEMP_DIR"

# Navigate to temp directory
cd "$TEMP_DIR"

# Initialize git if not already a repo
if [ ! -d ".git" ]; then
    echo -e "${BLUE}Initializing git repository...${NC}"
    git clone $REPO_URL
    git checkout -b "$BRANCH" 2>/dev/null || git checkout "$BRANCH"
    echo -e "${GREEN}✓${NC} Git repository initialized"
else
    echo -e "${GREEN}✓${NC} Git repository already exists"
fi

# Add remote if not exists
if ! git remote | grep -q origin; then
    echo -e "${BLUE}Adding remote repository...${NC}"
    git remote add origin "$REPO_URL"
    echo -e "${GREEN}✓${NC} Remote added: $REPO_URL"
else
    echo -e "${BLUE}Updating remote repository URL...${NC}"
    git remote set-url origin "$REPO_URL"
    echo -e "${GREEN}✓${NC} Remote updated: $REPO_URL"
fi

# Stage all files
echo -e "${BLUE}Staging files...${NC}"
git add -A
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
#git push -u origin "$BRANCH" --force
echo -e "${GREEN}✓${NC} Pushed to remote"

echo ""
echo -e "${GREEN}=== Publishing Complete ===${NC}"
echo ""
echo -e "${BLUE}Template directory:${NC} $TEMP_DIR"
echo -e "${BLUE}Remote repository:${NC} $REPO_URL"
echo -e "${BLUE}Branch:${NC} $BRANCH"
echo ""
echo -e "${BLUE}Note:${NC} Temporary directory will be cleaned up on next system reboot"
