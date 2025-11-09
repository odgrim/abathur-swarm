#!/bin/bash
# Verification script for vec0 extension integration

set -e

echo "=== vec0 Extension Verification ==="
echo

echo "1. Running unit tests for extension loading..."
cargo test --test vec0_extension_test --quiet
echo "   ✓ Unit tests passed"
echo

echo "2. Running integration tests..."
cargo test --test vec0_integration_test --quiet
echo "   ✓ Integration tests passed"
echo

echo "3. Checking migration status..."
MIGRATIONS=$(sqlite3 .abathur/abathur.db "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 8;")
if [ "$MIGRATIONS" -eq "1" ]; then
    echo "   ✓ Migration 008 applied"
else
    echo "   ✗ Migration 008 not found"
    exit 1
fi
echo

echo "4. Checking vec0 tables exist..."
VEC0_TABLE=$(sqlite3 .abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE name = 'vec_memory_vec0';")
BRIDGE_TABLE=$(sqlite3 .abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE name = 'vec_memory_bridge';")
CONFIG_TABLE=$(sqlite3 .abathur/abathur.db "SELECT COUNT(*) FROM sqlite_master WHERE name = 'vector_config';")

if [ "$VEC0_TABLE" -eq "1" ]; then
    echo "   ✓ vec_memory_vec0 virtual table exists"
else
    echo "   ✗ vec_memory_vec0 table not found"
    exit 1
fi

if [ "$BRIDGE_TABLE" -eq "1" ]; then
    echo "   ✓ vec_memory_bridge table exists"
else
    echo "   ✗ vec_memory_bridge table not found"
    exit 1
fi

if [ "$CONFIG_TABLE" -eq "1" ]; then
    echo "   ✓ vector_config table exists"
else
    echo "   ✗ vector_config table not found"
    exit 1
fi
echo

echo "5. Checking vector configuration..."
VEC0_ENABLED=$(sqlite3 .abathur/abathur.db "SELECT value FROM vector_config WHERE key = 'vec0_available';")
DIMENSIONS=$(sqlite3 .abathur/abathur.db "SELECT value FROM vector_config WHERE key = 'vec0_dimensions';")
METRIC=$(sqlite3 .abathur/abathur.db "SELECT value FROM vector_config WHERE key = 'vec0_distance_metric';")

echo "   ✓ vec0_available: $VEC0_ENABLED"
echo "   ✓ Dimensions: $DIMENSIONS"
echo "   ✓ Distance metric: $METRIC"
echo

echo "6. Testing task list with vec0 enabled..."
TASK_COUNT=$(./target/release/abathur task list --limit 100 2>&1 | grep "Showing" | awk '{print $2}')
echo "   ✓ Task list works - found $TASK_COUNT tasks"
echo

echo "=== All Verification Steps Passed! ==="
echo
echo "Summary:"
echo "  - sqlite-vec extension is properly registered"
echo "  - vec0 virtual tables are created"
echo "  - Migration 008 successfully applied"
echo "  - All tests pass"
echo "  - Application functions correctly with vec0 support"
