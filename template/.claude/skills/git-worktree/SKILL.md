---
name: git-worktree
description: Create and manage git worktrees in .abathur directory with isolated virtual environments for parallel development
version: 1.0.0
---

# git-worktree Skill

This skill provides standardized commands for creating and managing git worktrees with isolated Python virtual environments. Worktrees enable parallel development on multiple branches without switching contexts or stashing changes.

## When to Use This Skill

- Creating a new feature branch for isolated development
- Working on multiple features/bugs simultaneously
- Testing changes in isolation without affecting main development environment
- Reviewing PRs locally with full environment setup
- Experimenting with dependency changes without affecting main venv

## Worktree Organization

All worktrees are created in the `.abathur/` directory with the following structure:

```
.abathur/
├── features/                  # Feature branch worktrees (created by technical-requirements-specialist)
│   ├── task-queue-enhancements/   # Feature worktree for entire feature
│   │   ├── src/               # Source code (git worktree)
│   │   ├── tests/             # Tests
│   │   └── pyproject.toml     # Dependencies
│   └── memory-service/        # Another feature worktree
│       └── ...
├── worktrees/                 # Task-specific worktrees (created by task-planner)
│   ├── task-001-domain-model/ # Individual task worktree
│   │   ├── src/               # Source code (git worktree)
│   │   └── ...
│   └── task-002-api/          # Another task worktree
│       └── ...
└── [legacy single worktrees]  # Old-style worktrees (deprecated)
    ├── feature-auth/
    └── bugfix-memory-leak/
```

**Worktree Hierarchy:**
- **Feature Worktrees** (`.abathur/features/`): Created by `technical-requirements-specialist` for entire features
  - Branch: `feature/feature-name`
  - Purpose: Main working directory for a feature
  - Contains all changes for the feature
  - Eventually merges to `main`

- **Task Worktrees** (`.abathur/worktrees/`): Created by `task-planner` for individual tasks
  - Branch: `feature/{feature-name}/task/{task-name}/{YYYY-MM-DD-HH-MM-SS}`
  - Purpose: Isolated work for a single atomic task
  - Merges into the feature branch (not main)
  - Enables parallel task execution without conflicts

## Core Commands

### Create Feature Worktree (for entire feature)

Creates a feature branch as a git worktree for all work related to a feature:

```bash
# Create feature worktree
FEATURE_NAME="task-queue-enhancements"
FEATURE_BRANCH="feature/$FEATURE_NAME"
WORKTREE_PATH=".abathur/features/$FEATURE_NAME"

# Ensure .abathur/features directory exists
mkdir -p .abathur/features

# Create feature worktree from main
git worktree add -b "$FEATURE_BRANCH" "$WORKTREE_PATH"

# Verify creation
test -d "$WORKTREE_PATH" && echo "Feature worktree created at $WORKTREE_PATH"
```

### Create Task Worktree (for individual task)

Creates a task-specific worktree that branches from a feature branch:

```bash
# Create task worktree from feature branch
FEATURE_BRANCH="feature/task-queue-enhancements"
TASK_NAME="domain-model"
TIMESTAMP=$(date +%Y-%m-%d-%H-%M-%S)
TASK_BRANCH="$FEATURE_BRANCH/task/$TASK_NAME/$TIMESTAMP"
WORKTREE_PATH=".abathur/worktrees/$TASK_NAME"

# Ensure .abathur/worktrees directory exists
mkdir -p .abathur/worktrees

# Create task worktree from feature branch (not main!)
git worktree add -b "$TASK_BRANCH" "$WORKTREE_PATH" "$FEATURE_BRANCH"

# Verify creation
test -d "$WORKTREE_PATH" && echo "Task worktree created at $WORKTREE_PATH"
```

### Create New Worktree with Virtual Environment (Legacy)

Creates a new git worktree and sets up an isolated Python virtual environment:

```bash
# Create worktree from current branch
BRANCH_NAME="feature/new-feature"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

# Create worktree and new branch
git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"

# Create isolated virtualenv
python3 -m venv "$WORKTREE_PATH/venv"

# Activate and install dependencies
source "$WORKTREE_PATH/venv/bin/activate"
cd "$WORKTREE_PATH"
pip install --upgrade pip
poetry install
```

### Create Worktree from Existing Branch

```bash
# Checkout existing branch in new worktree
BRANCH_NAME="feature/existing-feature"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add "$WORKTREE_PATH" "$BRANCH_NAME"

# Set up virtualenv
python3 -m venv "$WORKTREE_PATH/venv"
source "$WORKTREE_PATH/venv/bin/activate"
cd "$WORKTREE_PATH"
pip install --upgrade pip
poetry install
```

### List All Worktrees

```bash
git worktree list
```

### Remove Worktree

```bash
# Deactivate virtualenv first if active
deactivate 2>/dev/null || true

# Remove worktree (use relative path or full path)
WORKTREE_PATH=".abathur/feature-new-feature"
git worktree remove "$WORKTREE_PATH"

# Or force remove if there are modifications
git worktree remove --force "$WORKTREE_PATH"
```

### Switch to Worktree

```bash
# Navigate to worktree and activate its virtualenv
WORKTREE_PATH=".abathur/feature-new-feature"
cd "$WORKTREE_PATH"
source venv/bin/activate
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

# Step 2: Set up isolated environment
python3 -m venv "$WORKTREE_PATH/venv"
source "$WORKTREE_PATH/venv/bin/activate"
cd "$WORKTREE_PATH"

# Step 3: Install dependencies
pip install --upgrade pip
poetry install

# Step 4: Verify setup
which python  # Should point to worktree venv
git branch    # Should show feature branch
pytest -n auto tests/unit/  # Run tests to verify setup

# Now you can develop in isolation!
```

### Working on a Bug Fix

```bash
# Create feature worktree for bug fix (use feature/ prefix)
BRANCH_NAME="feature/memory-leak-fix"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"
python3 -m venv "$WORKTREE_PATH/venv"

# Activate and setup
cd "$WORKTREE_PATH"
source venv/bin/activate
pip install --upgrade pip
poetry install

# Install additional debug tools in this venv only
pip install memory-profiler pytest-memray

# Work on the fix...
```

### Testing Dependency Updates

```bash
# Create experimental worktree
BRANCH_NAME="experiment/upgrade-pydantic"
WORKTREE_PATH=".abathur/${BRANCH_NAME//\//-}"

git worktree add -b "$BRANCH_NAME" "$WORKTREE_PATH"
cd "$WORKTREE_PATH"
python3 -m venv venv
source venv/bin/activate

# Install with updated dependencies
pip install --upgrade pip
poetry add pydantic@^3.0.0  # Experimental upgrade

# Test if everything works
pytest -n auto

# If successful, merge. If not, just remove worktree!
```

### Reviewing a Pull Request

```bash
# Fetch PR branch
git fetch origin pull/123/head:pr-123

# Create worktree for review
WORKTREE_PATH=".abathur/pr-123"
git worktree add "$WORKTREE_PATH" pr-123

# Setup environment
cd "$WORKTREE_PATH"
python3 -m venv venv
source venv/bin/activate
pip install --upgrade pip
poetry install

# Review and test the PR
pytest -n auto
# ... review code, run manual tests, etc.

# When done, remove worktree
cd ../..
git worktree remove "$WORKTREE_PATH"
```

### Cleaning Up After Merge

```bash
# After merging a feature branch, clean up its worktree
WORKTREE_PATH=".abathur/feature-user-authentication"

# Make sure you're not in the worktree directory
cd /Users/odgrim/dev/home/agentics/abathur

# Remove worktree
git worktree remove "$WORKTREE_PATH"

# Delete the merged branch (optional)
git branch -d feature/user-authentication
```

## Cleanup Workflows

### Remove Single Worktree

```bash
# Step 1: Deactivate virtualenv if currently active
deactivate 2>/dev/null || true

# Step 2: Navigate away from worktree directory
cd /Users/odgrim/dev/home/agentics/abathur

# Step 3: Remove the worktree
WORKTREE_PATH=".abathur/feature-new-feature"
git worktree remove "$WORKTREE_PATH"

# Step 4 (Optional): Delete the branch if merged
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

echo "🧹 Cleaning up merged worktrees..."

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

echo "✅ Cleanup complete! Removed $removed_count worktrees."
echo ""
echo "Remaining worktrees:"
git worktree list
```

## Best Practices

1. **Naming Convention**: Use descriptive branch names that translate to clear directory names
   - Feature branches: `feature/user-auth` → `.abathur/features/user-auth`
   - Task branches: `feature/user-auth/task/add-validation/2025-10-23-14-30-00` → `.abathur/worktrees/add-validation`

2. **Always Create Virtualenv**: Each worktree should have its own `venv/` for true isolation

3. **Use Poetry Install**: Run `poetry install` in each worktree to ensure dependencies match `pyproject.toml`

4. **Check Active Environment**: Use `which python` to verify you're in the correct virtualenv

5. **Cleanup Regularly**: Remove worktrees after merging branches to save disk space
   - Run `git worktree prune` weekly
   - Remove merged branch worktrees immediately after merging
   - Check disk usage periodically with `du -sh .abathur/*`

6. **Test Before Merging**: Always run the test suite in the worktree before merging

7. **Deactivate Before Removing**: Always deactivate the virtualenv before removing a worktree

8. **Git Ignore**: The `.abathur/` directory should be in `.gitignore` to prevent committing worktrees

9. **Never Delete Manually**: Always use `git worktree remove`, not `rm -rf`
   - Manual deletion leaves stale git references
   - Use `git worktree prune` to clean up if you accidentally deleted manually

10. **Track Active Worktrees**: Use `git worktree list` regularly to see what's active

## Common Patterns

### Quick Feature Branch Setup (One Command)

```bash
BRANCH="feature/new-api" && WORKTREE=".abathur/${BRANCH//\//-}" && git worktree add -b "$BRANCH" "$WORKTREE" && python3 -m venv "$WORKTREE/venv" && cd "$WORKTREE" && source venv/bin/activate && pip install --upgrade pip && poetry install && echo "✓ Worktree ready at $WORKTREE"
```

### Switch Between Worktrees

```bash
# From main repo
cd .abathur/feature-auth && source venv/bin/activate

# Switch to different worktree
deactivate
cd ../bugfix-memory && source venv/bin/activate
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

### Virtualenv Issues

```bash
# If virtualenv fails to activate
# Solution: Recreate it
rm -rf .abathur/feature-name/venv
python3 -m venv .abathur/feature-name/venv
source .abathur/feature-name/venv/bin/activate
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

## Integration with Other Skills

### Running Tests in Worktree

```bash
# After setting up worktree, use pytest skill
cd .abathur/feature-new-api
source venv/bin/activate
pytest -n auto tests/unit/  # Runs in isolated environment
```

### Git Operations in Worktree

```bash
# All git commands work normally in worktrees
cd .abathur/feature-auth
git status
git add .
git commit -m "Add authentication logic"
git push origin feature/user-auth
```

## Notes

- Each worktree is a full working copy of the repository
- Worktrees share the same `.git` directory (efficient storage)
- You cannot check out the same branch in multiple worktrees
- The `.abathur/` directory should be in `.gitignore`
- Virtualenvs are isolated per worktree, allowing different package versions
- Use `git worktree prune` periodically to clean up deleted worktree references

## Safety

- Always deactivate virtualenv before removing worktree: `deactivate`
- Never delete `.abathur/` directories manually without using `git worktree remove`
- Verify you're in the correct worktree before committing: `git branch` and `which python`
- Backup important work before removing worktrees
