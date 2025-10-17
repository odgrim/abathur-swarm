# Integration Test Notes: Task Limit Enforcement

## Test Suite: test_task_limit_real_queue.py

### Purpose
Integration tests for SwarmOrchestrator task_limit feature with real task queue operations.
Validates Phase 3 (Integration Testing) for task limit bug fix.

### Test Scenarios

1. **Scenario 1: Basic Task Limit (task_limit=5)**
   - Creates 10 tasks, runs with task_limit=5
   - Expects exactly 5 tasks to complete
   - Verifies remaining 5 stay in READY state

2. **Scenario 2: Graceful Shutdown with Active Tasks**
   - Creates 8 tasks with slow execution (100ms)
   - Runs with task_limit=5, max_concurrent=3
   - Verifies at least 5 tasks complete (may be more if spawned before limit)
   - Confirms graceful shutdown waits for active tasks

3. **Scenario 3: Indefinite Mode (task_limit=None)**
   - Creates 20 tasks
   - Runs with task_limit=None
   - Verifies all 20 tasks complete (backward compatibility)

4. **Scenario 4: Zero Limit (task_limit=0)**
   - Creates 10 tasks
   - Runs with task_limit=0
   - Verifies 0 tasks processed, immediate exit

5. **Scenario 5: Failed Tasks Count Toward Limit**
   - Creates 10 tasks with failing executor
   - Runs with task_limit=5
   - Verifies exactly 5 tasks processed (all failures)

6. **Scenario 5b: Mixed Success and Failure**
   - Creates 10 tasks with alternating success/failure
   - Runs with task_limit=7
   - Verifies exactly 7 tasks processed (mixed results)

### CRITICAL: Code Dependencies

**These tests require the FIXED code from Task 001 (Code Modification).**

**Fixed Code Location:** `/Users/odgrim/dev/home/agentics/abathur/.abathur/worktrees/task-001-code-modification/`

**Key Fix (line 90):**
```python
# OLD CODE (main branch) - counts at spawn time
if task_limit is not None and tasks_processed >= task_limit:

# FIXED CODE (task-001 worktree) - counts at completion time
if task_limit is not None and len(self.results) >= task_limit:
```

### Test Execution Against Main Branch (UNFIXED)

When tests run against the main branch (current state), they will FAIL because:
- Main branch counts tasks at spawn time (incorrect behavior)
- With task_limit=5 and max_concurrent=5, swarm may spawn 6 tasks before checking limit
- Tests expect exactly 5 completions, but get 6-7 due to timing

**Example Failure Output:**
```
AssertionError: Expected exactly 5 results, got 7
```

This is EXPECTED behavior - tests are designed for the fixed code.

### Test Execution Against Fixed Code (CORRECT)

To run tests against the fixed code:

#### Option 1: Copy Fixed File to Main Branch (for testing)
```bash
cp .abathur/worktrees/task-001-code-modification/src/abathur/application/swarm_orchestrator.py \
   src/abathur/application/swarm_orchestrator.py

pytest tests/integration/test_task_limit_real_queue.py -v --asyncio-mode=auto
```

#### Option 2: Run Tests from Worktree
```bash
cd .abathur/worktrees/task-001-code-modification

pytest tests/integration/test_task_limit_real_queue.py -v --asyncio-mode=auto
```

#### Option 3: Set PYTHONPATH (Recommended for CI/CD)
```bash
PYTHONPATH=/Users/odgrim/dev/home/agentics/abathur/.abathur/worktrees/task-001-code-modification \
pytest tests/integration/test_task_limit_real_queue.py -v --asyncio-mode=auto
```

### Expected Results (with Fixed Code)

All 6 scenarios should PASS:
```
tests/integration/test_task_limit_real_queue.py::test_scenario_1_basic_task_limit_exactly_5_tasks PASSED
tests/integration/test_task_limit_real_queue.py::test_scenario_2_graceful_shutdown_with_active_tasks PASSED
tests/integration/test_task_limit_real_queue.py::test_scenario_3_indefinite_mode_task_limit_none PASSED
tests/integration/test_task_limit_real_queue.py::test_scenario_4_zero_limit_exits_immediately PASSED
tests/integration/test_task_limit_real_queue.py::test_scenario_5_failed_tasks_count_toward_limit PASSED
tests/integration/test_task_limit_real_queue.py::test_scenario_5b_mixed_success_and_failure_tasks PASSED
```

### Test Coverage

**Tested Components:**
- SwarmOrchestrator.start_swarm() with task_limit parameter
- TaskQueueService.enqueue_task() for creating test tasks
- Database.get_task() for verifying task status
- Real task queue integration (not mocked)

**Tested Scenarios:**
- ✅ Exact count enforcement (task_limit=5 → exactly 5 completions)
- ✅ Zero limit edge case (task_limit=0 → no tasks processed)
- ✅ Indefinite mode (task_limit=None → all tasks processed)
- ✅ Graceful shutdown with concurrent tasks
- ✅ Failed tasks counting toward limit
- ✅ Mixed success/failure scenarios

### Integration Test Checklist

- [x] Scenario 1: Basic limit (task_limit=5)
- [x] Scenario 2: Graceful shutdown
- [x] Scenario 3: Indefinite mode (task_limit=None)
- [x] Scenario 4: Zero limit (task_limit=0)
- [x] Scenario 5: Failed tasks count
- [x] Scenario 5b: Mixed success/failure
- [ ] Run against fixed code in worktree (USER ACTION REQUIRED)
- [ ] Verify all scenarios pass with fixed code
- [ ] Document performance metrics

### Performance Expectations

With mock executor (0.01s per task):
- Scenario 1 (10 tasks, limit=5): ~0.05s
- Scenario 2 (8 tasks, limit=5, slow): ~0.5s
- Scenario 3 (20 tasks, no limit): ~0.5s
- Scenario 4 (10 tasks, limit=0): <0.1s (immediate exit)
- Scenario 5 (10 tasks, limit=5, failures): ~0.05s
- Scenario 5b (10 tasks, limit=7, mixed): ~0.07s

**Total Test Suite Runtime:** ~1-2 seconds

### Next Steps

1. **Apply Fix from Task 001** - Copy fixed code to main branch
2. **Re-run Tests** - Verify all 6 scenarios pass
3. **Run Unit Tests** - Ensure no regressions (tests/unit/test_swarm_task_limit.py)
4. **Run Integration Tests** - Verify CLI integration (tests/integration/test_cli_task_limit.py)
5. **Measure Performance** - Confirm <10ms per task target met
6. **Merge to Main** - After all tests pass

### Related Files

- **Test File:** `tests/integration/test_task_limit_real_queue.py`
- **Fixed Code:** `.abathur/worktrees/task-001-code-modification/src/abathur/application/swarm_orchestrator.py`
- **Main Code:** `src/abathur/application/swarm_orchestrator.py` (needs fix applied)
- **Unit Tests:** `tests/unit/test_swarm_task_limit.py`
- **CLI Tests:** `tests/integration/test_cli_task_limit.py`

### Contact

For questions about these tests or task limit implementation:
- Reference: Task 003 - Integration Testing with Real Task Queue
- Technical Spec Task ID: 8b108359-ec07-4d33-a7c0-c9cc70009c1c
- Agent: python-testing-specialist
