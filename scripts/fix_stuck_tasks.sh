#!/bin/bash
# Fix tasks stuck in running state with error messages
#
# This script finds tasks that are:
# - Status = running
# - Have error_message set
# And updates their status to failed

set -e

DB_PATH="${1:-.mcp/abathur/tasks.db}"

if [ ! -f "$DB_PATH" ]; then
    echo "Error: Database not found at $DB_PATH"
    echo "Usage: $0 [path/to/tasks.db]"
    exit 1
fi

echo "Fixing stuck tasks in database: $DB_PATH"
echo

# Find stuck tasks
echo "Finding tasks stuck in running state with error messages..."
STUCK_TASKS=$(sqlite3 "$DB_PATH" "SELECT id, summary, error_message FROM tasks WHERE status = 'running' AND error_message IS NOT NULL AND error_message != ''")

if [ -z "$STUCK_TASKS" ]; then
    echo "No stuck tasks found!"
    exit 0
fi

echo "Found stuck tasks:"
echo "$STUCK_TASKS" | while IFS='|' read -r id summary error; do
    echo "  - $id: $summary"
    echo "    Error: ${error:0:100}..."
    echo
done

# Count stuck tasks
TASK_COUNT=$(echo "$STUCK_TASKS" | wc -l | tr -d ' ')
echo "Total: $TASK_COUNT stuck task(s)"
echo

# Confirm before fixing
read -p "Update these tasks to 'failed' status? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Cancelled."
    exit 0
fi

# Fix stuck tasks
echo "Updating task statuses..."
sqlite3 "$DB_PATH" <<SQL
UPDATE tasks
SET status = 'failed',
    last_updated_at = datetime('now')
WHERE status = 'running'
  AND error_message IS NOT NULL
  AND error_message != '';
SQL

echo "Done! Updated $TASK_COUNT task(s) to 'failed' status."
