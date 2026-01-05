-- Migration: Add needs_worktree column to tasks table
-- This column controls whether a task needs its own worktree
-- When NULL or true: task gets its own worktree if feature_branch is set
-- When false: task does NOT get its own worktree (uses feature branch worktree directly)

ALTER TABLE tasks ADD COLUMN needs_worktree INTEGER;
