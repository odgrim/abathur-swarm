-- Migration: Standardize datetime format to RFC3339
-- Date: 2025-10-29
-- Description: Converts all datetime columns from SQLite default format to RFC3339 format
--
-- This migration handles datetime columns that may be in SQLite format (YYYY-MM-DD HH:MM:SS)
-- and converts them to RFC3339 format (YYYY-MM-DDTHH:MM:SSZ)
--
-- Why RFC3339?
-- - Standard format for datetime serialization
-- - Unambiguous timezone representation
-- - What the application code expects and writes
-- - Better portability across systems

-- Tasks table datetime columns
UPDATE tasks
SET submitted_at = REPLACE(submitted_at, ' ', 'T') || 'Z'
WHERE submitted_at NOT LIKE '%T%Z'
  AND submitted_at NOT LIKE '%+%'
  AND submitted_at NOT LIKE '%-__:__';

UPDATE tasks
SET started_at = REPLACE(started_at, ' ', 'T') || 'Z'
WHERE started_at IS NOT NULL
  AND started_at NOT LIKE '%T%Z'
  AND started_at NOT LIKE '%+%'
  AND started_at NOT LIKE '%-__:__';

UPDATE tasks
SET completed_at = REPLACE(completed_at, ' ', 'T') || 'Z'
WHERE completed_at IS NOT NULL
  AND completed_at NOT LIKE '%T%Z'
  AND completed_at NOT LIKE '%+%'
  AND completed_at NOT LIKE '%-__:__';

UPDATE tasks
SET last_updated_at = REPLACE(last_updated_at, ' ', 'T') || 'Z'
WHERE last_updated_at NOT LIKE '%T%Z'
  AND last_updated_at NOT LIKE '%+%'
  AND last_updated_at NOT LIKE '%-__:__';

UPDATE tasks
SET deadline = REPLACE(deadline, ' ', 'T') || 'Z'
WHERE deadline IS NOT NULL
  AND deadline NOT LIKE '%T%Z'
  AND deadline NOT LIKE '%+%'
  AND deadline NOT LIKE '%-__:__';

-- Agents table datetime columns
UPDATE agents
SET heartbeat_at = REPLACE(heartbeat_at, ' ', 'T') || 'Z'
WHERE heartbeat_at NOT LIKE '%T%Z'
  AND heartbeat_at NOT LIKE '%+%'
  AND heartbeat_at NOT LIKE '%-__:__';

UPDATE agents
SET created_at = REPLACE(created_at, ' ', 'T') || 'Z'
WHERE created_at NOT LIKE '%T%Z'
  AND created_at NOT LIKE '%+%'
  AND created_at NOT LIKE '%-__:__';

UPDATE agents
SET terminated_at = REPLACE(terminated_at, ' ', 'T') || 'Z'
WHERE terminated_at IS NOT NULL
  AND terminated_at NOT LIKE '%T%Z'
  AND terminated_at NOT LIKE '%+%'
  AND terminated_at NOT LIKE '%-__:__';

-- State table datetime columns
UPDATE state
SET updated_at = REPLACE(updated_at, ' ', 'T') || 'Z'
WHERE updated_at NOT LIKE '%T%Z'
  AND updated_at NOT LIKE '%+%'
  AND updated_at NOT LIKE '%-__:__';

-- Audit table datetime columns
UPDATE audit
SET timestamp = REPLACE(timestamp, ' ', 'T') || 'Z'
WHERE timestamp NOT LIKE '%T%Z'
  AND timestamp NOT LIKE '%+%'
  AND timestamp NOT LIKE '%-__:__';

-- Metrics table datetime columns
UPDATE metrics
SET timestamp = REPLACE(timestamp, ' ', 'T') || 'Z'
WHERE timestamp NOT LIKE '%T%Z'
  AND timestamp NOT LIKE '%+%'
  AND timestamp NOT LIKE '%-__:__';

-- Memories table datetime columns
UPDATE memories
SET created_at = REPLACE(created_at, ' ', 'T') || 'Z'
WHERE created_at NOT LIKE '%T%Z'
  AND created_at NOT LIKE '%+%'
  AND created_at NOT LIKE '%-__:__';

UPDATE memories
SET updated_at = REPLACE(updated_at, ' ', 'T') || 'Z'
WHERE updated_at NOT LIKE '%T%Z'
  AND updated_at NOT LIKE '%+%'
  AND updated_at NOT LIKE '%-__:__';

-- Sessions table datetime columns
UPDATE sessions
SET created_at = REPLACE(created_at, ' ', 'T') || 'Z'
WHERE created_at NOT LIKE '%T%Z'
  AND created_at NOT LIKE '%+%'
  AND created_at NOT LIKE '%-__:__';

UPDATE sessions
SET updated_at = REPLACE(updated_at, ' ', 'T') || 'Z'
WHERE updated_at NOT LIKE '%T%Z'
  AND updated_at NOT LIKE '%+%'
  AND updated_at NOT LIKE '%-__:__';

-- Session events table datetime columns
UPDATE session_events
SET timestamp = REPLACE(timestamp, ' ', 'T') || 'Z'
WHERE timestamp NOT LIKE '%T%Z'
  AND timestamp NOT LIKE '%+%'
  AND timestamp NOT LIKE '%-__:__';
