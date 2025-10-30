---
name: git-branch
description: ALWAYS use this skill to create git branches/worktrees instead of manual git commands. Required for all branch creation by agents.
version: 3.0.0
---

## ⚠️ MANDATORY USAGE FOR ALL AGENTS

**ALL agents MUST use this skill when creating branches or worktrees.**

**NEVER use manual git commands** like:
- `git branch`
- `git checkout -b`
- `git worktree add`

**ALWAYS invoke this skill instead** using the Skill tool.

This skill is REQUIRED for:
- Creating feature branches (technical-requirements-specialist)
- Creating task branches (task-planner)
- Creating any isolated working directory
- Managing parallel development workflows

**How to use:**
```bash
# Instead of: git worktree add -b feature/my-feature .abathur/features/my-feature
# Do this:
Skill("git-branch")
# Then follow the skill's documented commands
```

# git-branch Skill

This skill provides standardized commands for creating and managing git worktrees. Worktrees enable parallel development on multiple branches without switching contexts or stashing changes.

## Branch Hierarchy

**CRITICAL: Understand the two-level branch hierarchy:**

```
main (production)
├── feature/feature-name-1 (forked from main)
│   ├── feature/feature-name-1/task/task-001/YYYY-MM-DD-HH-MM-SS (forked from feature branch)
│   ├── feature/feature-name-1/task/task-002/YYYY-MM-DD-HH-MM-SS (forked from feature branch)
│   └── feature/feature-name-1/task/task-003/YYYY-MM-DD-HH-MM-SS (forked from feature branch)
├── feature/feature-name-2 (forked from main)
│   ├── feature/feature-name-2/task/task-001/YYYY-MM-DD-HH-MM-SS (forked from feature branch)
│   └── feature/feature-name-2/task/task-002/YYYY-MM-DD-HH-MM-SS (forked from feature branch)
```

**Two types of branches:**

1. **Feature Branches** (created by technical-requirements-specialist)
   - Format: `feature/descriptive-name`
   - Fork from: `main` branch
   - Merge to: `main` branch
   - Purpose: Container for all work related to a feature
   - Location: `.abathur/features/descriptive-name`
   - Example: `feature/user-authentication` forked from `main`

2. **Task Branches** (created by task-planner)
   - Format: `feature/feature-name/task/task-name/YYYY-MM-DD-HH-MM-SS`
   - Fork from: The parent `feature/feature-name` branch (NOT main!)
   - Merge to: The parent `feature/feature-name` branch (NOT main!)
   - Purpose: Isolated work for a single atomic task
   - Location: `.abathur/worktrees/task-name`
   - Example: `feature/user-authentication/task/login-validation/2025-10-29-14-30-00` forked from `feature/user-authentication`

**Workflow:**
1. Feature branch is created from main
2. Multiple task branches are created from the feature branch
3. Task branches merge back into their feature branch
4. Feature branch merges into main when all tasks complete

**NEVER:**
- Create task branches from main (they must fork from feature branch)
- Merge task branches to main (they must merge to feature branch)
- Create feature branches from other feature branches

## When to Use This Skill

- Creating a new feature branch for isolated development
- Working on multiple features/bugs simultaneously
- Testing changes in isolation without affecting main development
- Reviewing PRs locally with full setup
- Experimenting with changes without affecting main branch

## Worktree Organization

All worktrees are created in the `.abathur/` directory with the following structure:

```
.abathur/
├── features/                  # Feature branch worktrees
│   ├── user-authentication/   # Feature worktree for entire feature
│   │   └── [project files]    # Full git working tree
│   └── api-redesign/          # Another feature worktree
│       └── [project files]
├── worktrees/                 # Task-specific worktrees
│   ├── task-001-domain-model/ # Individual task worktree
│   │   └── [project files]
│   └── task-002-api/          # Another task worktree
│       └── [project files]
└── [legacy single worktrees]  # Old-style worktrees (deprecated)
    ├── feature-auth/
    └── bugfix-memory-leak/
```

**Worktree Hierarchy:**
- **Feature Worktrees** (`.abathur/features/`): For entire features
  - Branch: `feature/feature-name`
  - Purpose: Main working directory for a feature
  - Contains all changes for the feature
  - Eventually merges to `main`

- **Task Worktrees** (`.abathur/worktrees/`): For individual tasks
  - Branch: `feature/{feature-name}/task/{task-name}/{YYYY-MM-DD-HH-MM-SS}`
  - Purpose: Isolated work for a single atomic task
  - Merges into the feature branch (not main)
  - Enables parallel task execution without conflicts

## Core Commands

### Create Feature Worktree (for entire feature)

Creates a feature branch as a git worktree **forked from main**:

```bash
# Create feature worktree FROM MAIN BRANCH
FEATURE_NAME="user-authentication"
FEATURE_BRANCH="feature/$FEATURE_NAME"
WORKTREE_PATH=".abathur/features/$FEATURE_NAME"

# Ensure .abathur/features directory exists
mkdir -p .abathur/features

# CRITICAL: Create feature worktree from main (no third argument = current branch, which should be main)
git worktree add -b "$FEATURE_BRANCH" "$WORKTREE_PATH"

# Verify creation
test -d "$WORKTREE_PATH" && echo "Feature worktree created at $WORKTREE_PATH"
git branch -vv | grep "$FEATURE_BRANCH"  # Should show it's based on main

# Navigate to worktree and start working
cd "$WORKTREE_PATH"
```

### Create Task Worktree (for individual task)

Creates a task-specific worktree **forked from the parent feature branch**:

```bash
# CRITICAL: Create task worktree from FEATURE BRANCH (not main!)
FEATURE_BRANCH="feature/user-authentication"  # The parent feature branch
TASK_NAME="domain-model"
TIMESTAMP=$(date +%Y-%m-%d-%H-%M-%S)
TASK_BRANCH="$FEATURE_BRANCH/task/$TASK_NAME/$TIMESTAMP"
WORKTREE_PATH=".abathur/worktrees/$TASK_NAME"

# Ensure .abathur/worktrees directory exists
mkdir -p .abathur/worktrees

# CRITICAL: Third argument specifies the source branch (feature branch, NOT main!)
git worktree add -b "$TASK_BRANCH" "$WORKTREE_PATH" "$FEATURE_BRANCH"

# Verify creation
test -d "$WORKTREE_PATH" && echo "Task worktree created at $WORKTREE_PATH"
git branch -vv | grep "$TASK_BRANCH"  # Should show it's based on feature branch

# Navigate to worktree and start working
cd "$WORKTREE_PATH"
```

### Create New Worktree (Legacy)

Creates a new git worktree with a new branch:

```bash
# Create worktree from current branch
BRANCH_NAME="feature/new-feature"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

# Create worktree and new branch
git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"

# Navigate to worktree
cd "$WORKTREE_PATH"
```

### Create Worktree from Existing Branch

```bash
# Checkout existing branch in new worktree
BRANCH_NAME="feature/existing-feature"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add "$WORKTREE_PATH" "$BRANCH_NAME"

# Navigate to worktree
cd "$WORKTREE_PATH"
```

### List All Worktrees

```bash
git worktree list
```

### Remove Worktree

```bash
# Remove worktree (use relative path or full path)
WORKTREE_PATH=".abathur/feature-new-feature"
git worktree remove "$WORKTREE_PATH"

# Or force remove if there are modifications
git worktree remove --force "$WORKTREE_PATH"
```

### Switch to Worktree

```bash
# Navigate to worktree
WORKTREE_PATH=".abathur/feature-new-feature"
cd "$WORKTREE_PATH"
```

### Prune Deleted Worktrees

```bash
# Clean up worktree administrative files
git worktree prune
```

## Complete Workflow Examples

### Creating a New Feature Branch

```bash
# Step 1: Create worktree with new branch
BRANCH_NAME="feature/user-authentication"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"

# Step 2: Navigate to worktree
cd "$WORKTREE_PATH"

# Step 3: Verify setup
git branch    # Should show feature branch
git status    # Check worktree state

# Now you can develop in isolation!
```

### Working on a Bug Fix

```bash
# Create feature worktree for bug fix (use feature/ prefix)
BRANCH_NAME="feature/memory-leak-fix"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"

# Navigate to worktree
cd "$WORKTREE_PATH"

# Work on the fix...
# Make changes, commit, test
```

### Testing Experimental Changes

```bash
# Create experimental worktree
BRANCH_NAME="experiment/new-approach"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"
cd "$WORKTREE_PATH"

# Make experimental changes...
git status
git add .
git commit -m "Experimental approach"

# If successful, merge. If not, just remove worktree!
```

### Reviewing a Pull Request

```bash
# Fetch PR branch
git fetch origin pull/123/head:pr-123

# Create worktree for review
WORKTREE_PATH=".abathur/pr-123"
git worktree add "$WORKTREE_PATH" pr-123

# Navigate and review
cd "$WORKTREE_PATH"
git status
# Review code, test changes, etc.

# When done, remove worktree
cd ../..
git worktree remove "$WORKTREE_PATH"
```

### Cleaning Up After Merge

```bash
# After merging a feature branch, clean up its worktree
WORKTREE_PATH=".abathur/feature-user-authentication"

# Make sure you're not in the worktree directory
cd "$(git rev-parse --show-toplevel)"

# Remove worktree
git worktree remove "$WORKTREE_PATH"

# Delete the merged branch (optional)
git branch -d feature/user-authentication
```

## Cleanup Workflows

### Remove Single Worktree

```bash
# Step 1: Navigate away from worktree directory
cd "$(git rev-parse --show-toplevel)"

# Step 2: Remove the worktree
WORKTREE_PATH=".abathur/feature-new-feature"
git worktree remove "$WORKTREE_PATH"

# Step 3 (Optional): Delete the branch if merged
git branch -d feature/new-feature

# If the branch is not merged yet but you want to delete it anyway
git branch -D feature/new-feature
```

### Force Remove Worktree (with uncommitted changes)

```bash
# If worktree has modifications and you want to remove anyway
WORKTREE_PATH=".abathur/feature-experimental"
git worktree remove --force "$WORKTREE_PATH"

# Then clean up the branch
git branch -D feature/experimental
```

### Remove Worktree That Was Manually Deleted

```bash
# If you deleted .abathur/some-feature manually (don't do this!)
# Git still thinks the worktree exists

# List worktrees to see the broken reference
git worktree list

# Clean up stale references
git worktree prune

# Then delete the branch if needed
git branch -D feature/some-feature
```

### Clean Up All Merged Worktrees

```bash
# List all worktrees
git worktree list

# For each merged branch, remove its worktree
# First, update your main branch
git checkout main
git pull origin main

# Check which branches are merged
git branch --merged main

# Remove worktrees for merged branches
for branch in $(git branch --merged main | grep -v "^\*" | grep -v "main"); do
    # Convert branch name to worktree path
    worktree_path=".abathur/${branch//\//-}"
    worktree_path="${worktree_path// /}"  # Remove spaces

    if [ -d "$worktree_path" ]; then
        echo "Removing worktree: $worktree_path"
        git worktree remove "$worktree_path" 2>/dev/null || git worktree remove --force "$worktree_path"
        git branch -d "$branch"
    fi
done

# Prune any stale references
git worktree prune
```

### Clean Up All Worktrees (Nuclear Option)

```bash
# List all worktrees first to see what will be removed
git worktree list

# Remove all worktrees in .abathur directory
for worktree in .abathur/*/; do
    if [ -d "$worktree" ]; then
        echo "Removing worktree: $worktree"
        git worktree remove "$worktree" 2>/dev/null || git worktree remove --force "$worktree"
    fi
done

# Clean up stale references
git worktree prune

# Optionally delete all feature branches
# WARNING: This will delete ALL unmerged branches too!
# git branch | grep -v "main" | grep -v "^\*" | xargs git branch -D
```

### Clean Up Disk Space

```bash
# Check how much space worktrees are using
echo "Worktree disk usage:"
du -sh .abathur/* 2>/dev/null | sort -h

# Total space used by all worktrees
du -sh .abathur

# Remove old worktrees by age (older than 30 days)
find .abathur -maxdepth 1 -type d -mtime +30 | while read dir; do
    if [ "$dir" != ".abathur" ]; then
        echo "Removing old worktree: $dir"
        git worktree remove "$dir" 2>/dev/null || git worktree remove --force "$dir"
    fi
done

git worktree prune
```

### Interactive Cleanup

```bash
# Show all worktrees with their status
echo "Current worktrees:"
git worktree list

echo -e "\nDisk usage:"
du -sh .abathur/* 2>/dev/null

echo -e "\nMerged branches:"
git branch --merged main | grep -v "^\*" | grep -v "main"

# Now manually remove the ones you don't need
# Example:
# git worktree remove .abathur/feature-old
# git branch -d feature/old
```

### Automation Script for Regular Cleanup

```bash
#!/bin/bash
# Save as: scripts/cleanup-worktrees.sh

echo "Cleaning up merged worktrees..."

# Update main branch
git checkout main
git pull origin main

# Find and remove worktrees for merged branches
removed_count=0
for branch in $(git branch --merged main | grep -v "^\*" | grep -v "main"); do
    worktree_path=".abathur/${branch//\//-}"
    worktree_path="${worktree_path// /}"

    if [ -d "$worktree_path" ]; then
        echo "  Removing: $worktree_path (branch: $branch)"
        git worktree remove "$worktree_path" 2>/dev/null || git worktree remove --force "$worktree_path"
        git branch -d "$branch"
        ((removed_count++))
    fi
done

# Prune stale references
git worktree prune

echo "Cleanup complete! Removed $removed_count worktrees."
echo ""
echo "Remaining worktrees:"
git worktree list
```

## Best Practices

1. **Naming Convention**: Use descriptive branch names that translate to clear directory names
   - Feature branches: `feature/user-auth` → `.abathur/features/user-auth`
   - Task branches: `feature/user-auth/task/add-validation/2025-10-23-14-30-00` → `.abathur/worktrees/add-validation`

2. **Cleanup Regularly**: Remove worktrees after merging branches to save disk space
   - Run `git worktree prune` weekly
   - Remove merged branch worktrees immediately after merging
   - Check disk usage periodically with `du -sh .abathur/*`

3. **Verify Before Merging**: Always check the worktree state before merging
   - Use `git status` to see uncommitted changes
   - Use `git branch` to verify you're on the correct branch

4. **Git Ignore**: The `.abathur/` directory should be in `.gitignore` to prevent committing worktrees

5. **Never Delete Manually**: Always use `git worktree remove`, not `rm -rf`
   - Manual deletion leaves stale git references
   - Use `git worktree prune` to clean up if you accidentally deleted manually

6. **Track Active Worktrees**: Use `git worktree list` regularly to see what's active

## Common Patterns

### Quick Feature Branch Setup (One Command)

```bash
BRANCH="feature/new-api" && WORKTREE=".abathur/${BRANCH//\//-}" && git worktree add -b "$BRANCH" "$WORKTREE" && cd "$WORKTREE" && echo "Worktree ready at $WORKTREE"
```

### Switch Between Worktrees

```bash
# From main repo to a worktree
cd .abathur/feature-auth

# Switch to different worktree
cd ../feature-api
```

### List All Active Worktrees

```bash
# See all worktrees with their branches
git worktree list

# See worktrees in .abathur directory
ls -la .abathur/
```

## Troubleshooting

### Worktree Already Exists

```bash
# Error: 'path/to/worktree' already exists
# Solution: Remove the old worktree first
git worktree remove .abathur/old-branch
git worktree prune
```

### Branch Already Checked Out

```bash
# Error: branch 'feature/x' is already checked out at 'path'
# Solution: Use a different path or remove the existing worktree
git worktree list  # Find where it's checked out
git worktree remove <path>
```

### Disk Space

```bash
# Worktrees can use significant disk space
# Check worktree sizes
du -sh .abathur/*

# Remove unused worktrees
git worktree list
git worktree remove .abathur/old-feature
```

## Git Operations in Worktree

```bash
# All git commands work normally in worktrees
cd .abathur/feature-auth
git status
git add .
git commit -m "Add authentication logic"
git push origin feature/user-auth

# Check current branch
git branch

# See differences
git diff
git diff --staged
```

## Notes

- Each worktree is a full working copy of the repository
- Worktrees share the same `.git` directory (efficient storage)
- You cannot check out the same branch in multiple worktrees
- The `.abathur/` directory should be in `.gitignore`
- Use `git worktree prune` periodically to clean up deleted worktree references

## Safety

- Never delete `.abathur/` directories manually without using `git worktree remove`
- Verify you're in the correct worktree before committing: `git branch`
- Backup important work before removing worktrees
- Each worktree is independent with its own working directory
