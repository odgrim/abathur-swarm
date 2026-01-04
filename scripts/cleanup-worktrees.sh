#!/bin/bash
# Cleanup script to remove all worktrees and branches except main
# Usage: ./scripts/cleanup-worktrees.sh

set -e

echo "=== Git Worktree & Branch Cleanup ==="
echo ""

# Get current branch
CURRENT_BRANCH=$(git branch --show-current)
if [ "$CURRENT_BRANCH" != "main" ]; then
    echo "Switching to main branch..."
    git checkout main
fi

# Remove all worktrees except the main one
echo "Removing worktrees..."
git worktree list --porcelain | grep "^worktree " | cut -d' ' -f2- | while read -r worktree_path; do
    # Skip the main worktree (the one without .abathur in the path)
    if [[ "$worktree_path" != *".abathur"* ]]; then
        echo "  Skipping main worktree: $worktree_path"
        continue
    fi

    echo "  Removing: $worktree_path"
    git worktree remove --force "$worktree_path" 2>/dev/null || true
done

# Prune any stale worktree references
echo "Pruning stale worktree references..."
git worktree prune

# Remove all branches except main/master
echo "Removing branches..."
git branch | grep -v '^\*' | grep -v 'main' | grep -v 'master' | while read -r branch; do
    branch=$(echo "$branch" | xargs)  # trim whitespace
    if [ -n "$branch" ]; then
        echo "  Deleting branch: $branch"
        git branch -D "$branch" 2>/dev/null || true
    fi
done

# Clean up the worktrees directory if it exists and is empty
if [ -d ".abathur/worktrees" ]; then
    if [ -z "$(ls -A .abathur/worktrees 2>/dev/null)" ]; then
        echo "Removing empty worktrees directory..."
        rmdir .abathur/worktrees
    fi
fi

echo ""
echo "=== Cleanup Complete ==="
echo ""
echo "Remaining worktrees:"
git worktree list
echo ""
echo "Remaining branches:"
git branch
