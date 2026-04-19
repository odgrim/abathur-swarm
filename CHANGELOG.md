# Changelog

All notable changes to Abathur are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **YAML workflows as single source of truth** — Six hardcoded Rust workflow builders removed; workflows now resolve exclusively from inline `[[workflows]]` entries in `abathur.toml` and YAML files in `workflows_dir`. `abathur init` scaffolds default `<name>.yaml` files (preserving any existing edits).
- **Quiet windows** — New cron-scheduled cost-control windows during which the swarm pauses dispatch (migration 014, `abathur quiet-window` management, IANA timezones). Combined with budget pressure, this scales the swarm to zero during expensive pricing windows.
- **Week-day/time gating** — Configurable working-hours schedule (`abathur.toml`) so Abathur runs only Monday–Friday during approved hours by default.
- **Federation (A2A protocol v0.3)** — Adopt standard Agent2Agent wire format: domain types, `HttpA2AClient`, `/.well-known/agent.json` discovery, federation-aware `tasks/send` routing, `/rpc/stream` SSE endpoint for `tasks/sendSubscribe`.
- **Federated goals** — `FederatedGoal` model with Pending→Delegated→Active→Converging→Converged state machine, `ConvergenceContract` signals (BuildPassing, TestsPassing, ConvergenceLevel, Custom), `SwarmOverseer`, child-side goal tracking, and cross-swarm dependency DAG executor (migration 010).
- **Federation priority middleware** — Dedicated orchestrator middleware stack (federation_priority, autoship, budget, circuit_breaker, guardrails, mcp_readiness, merge_queue, pull_request, quiet_window, route_task, subtask_merge, verification) replacing the prior helpers god-module.
- **ClickUp Federation Proxy (Human Cerebrate)** — Standalone `human-cerebrate` crate that speaks the federation JSON-RPC protocol and creates/polls ClickUp tasks, letting the overmind delegate real-world tasks to humans with zero changes to core federation code.
- **Pull-request ingestion** — GitHub Issues adapter now ingests and reviews PRs (not just issues), with a new "rejected" status for issues that are no longer valid.
- **Event outbox** — Transactional outbox pattern (migration 008) so events are persisted within the same transaction as domain mutations and published by a background poller, closing the persist/publish gap.
- **Merge requests schema** — Persisted two-stage merge queue (migration 009) so conflict records survive across ephemeral `MergeQueue` instances.
- **Template stats persistence** — Evolution loop's `TemplateStats`, `TaskExecution`, and version-change history now survive process restarts (migration 013).
- **Task FK constraints & composite indexes** — Added cascade-delete FKs across `convergence_trajectories`, `worktrees`, `merge_requests`, `agent_instances`, and `events` (migration 011); added `idx_tasks_status_priority` and deadline/goal partial indexes (migration 012) to eliminate full-table scans in `get_ready_tasks`.
- **Cron triggers** — Schedule a prompt to run on crontab-style cadence (`abathur trigger`/`abathur cron`).
- **`abathur task prune`** — Filter-based prune subcommand (status, age, agent) that respects the DAG and refuses to delete tasks in an active dependency tree unless `--force`.
- **`abathur task submit -f <FILE>`** — Read the prompt from a file, with the same treatment as inline `<PROMPT>`.
- **Configurable max turns per agent role** — New `[max_turns.<role>]` config with example fixtures.
- **Encrypted webhook secrets at rest** — `webhook_subscriptions.secret` is encrypted with AES-256-GCM when `ABATHUR_MASTER_KEY` is set (32-byte key).
- **Recursive blocking cascade** — `TaskFailedBlockHandler` now iteratively blocks the entire dependent subtree (not just direct dependents) on retry exhaustion or cancellation.
- **Watermark-safe event pruning** — Event pruning retains events at or above the minimum handler watermark to prevent deletion before all handlers have processed them.
- **Global budget-pressure gate** — The convergence loop now terminates early with `BudgetDenied` when the tracker reports critical pressure (>95% consumed) instead of burning the last dollar on one task.
- **Guardrails at task submission** — HTTP `submit_task` runs a pre-flight `check_task_creation()` and returns 400 `GUARDRAIL_BLOCKED` over concurrent-task limits.
- **Critical handler designation** — Handlers can now opt into fast-retry and alerting semantics.
- **Optimistic locking on `task_repo.update()`** — `WHERE version = ?` enforces concurrent-update detection; callers now handle `ConcurrencyConflict` with retry.
- **Workflow phase failure recovery** — New `WorkflowPhaseFailureHandler` recovers from failed phase subtasks instead of leaving parent tasks Running forever.
- **Inter-functional contract documentation** — Full catalog of event-bus, task-lifecycle, workflow, convergence, and service-dependency contracts plus shortcomings analysis.

### Changed

- **Default workflows directory** — `workflows_dir` moved from the repo root to `.abathur/workflows`; `.gitignore` updated; embedded template paths and tests follow suit.
- **CLI verb consistency** — Unified verbs across goal/task/agent/event/memory/schedule/trigger/workflow/adapter (e.g. `goals set` vs `task submit`), new display formatting module, and a UX pass over every human operator command.
- **Agent-type plumbing** — Agent type now threaded end-to-end through paths that previously discarded it.
- **`abathur task list`** — Dropped priority column, added agent-type column, aligned formatting; `-l`/`--limit` flag now actually limits results; end-to-end CLI tests added.
- **Execution-mode classification weights** — Agent-role signal rebalanced from ±5 to ±2 so complexity, description, anti-pattern hints, and parent inheritance matter again.
- **Convergence threshold default** — YAML `ConvergenceLevel.min_level` default is now 0.8 (matching `SwarmOverseer`), not an impractical 1.0.
- **`ANTHROPIC_API_KEY` validation** — No longer an unconditional startup check; only required by substrates that need it (`anthropic_api`).
- **Branch-name validation in merge queue** — Rejects empty names, names starting with non-alphanumerics (`-flag` injection), and anything outside `[a-zA-Z0-9/_.\-]`.
- **Structured database errors** — `DatabaseError(String)` replaced with a categorized enum so handlers can react correctly (conflict vs timeout vs real failure).
- **Dual-publish guard** — When both `TaskService.event_bus` and `CommandBus` outbox are wired, `TaskService` skips direct publish inside the transaction scope to avoid duplicates.
- **Atomic retry-or-block** — The separate retry and block handlers on `TaskFailed` are merged into a single atomic handler to remove the race between them.

### Fixed

- **Memory timestamps** — Removed `datetime('now')` defaults on `memories.created_at`/`updated_at`/`last_accessed_at` (they emitted non-RFC3339 strings that poisoned `DateTime::parse_from_rfc3339`); callers must now supply explicit RFC3339 values.
- **Federation priority** — Federation work is now correctly prioritized by the orchestrator middleware chain.
- **Domain inference false positives** — Exclude `token budget`/`token limit`/`token count` from security classification and `mcp server`/`language server`/`lsp server` from backend classification; add `jwt` as a security keyword; regression tests added.
- **`converge_parallel()` never detecting `OverseerConverged`** — The `LoopControl` returned by `iterate_once()` was being discarded in both phases, making `OverseerConverged` unreachable; dead `best_converged: Option<usize>` removed.
- **Circuit-breaker test assertions** — `test_circuit_breaker_proceeds_with_fewer_than_three_tasks` corrected to assert `Reaction::None` (which the handler always returns) and verify behavior by checking for a newly created convergence-check record.
- **Memory-service merge conflicts** — Resolved long-standing conflicts in `memory_service.rs` and brought migration 013 into place cleanly.
- **Worktree merge fixes** — Multiple merge-path corrections, including switching `check_merge_conflicts` to the `git merge-tree --write-tree` invocation, distinguishing exit code 1 (conflicts) from ≥2 (errors), and broadening `extract_conflict_files` to all `CONFLICT` line formats (Issue #45).
- **`try_auto_ship` concurrency** — Added a `tokio::sync::Mutex` to serialize auto-ship so concurrent squash-merges stop racing on the shared working directory.
- **Federation contract violations** — Closed the convergence pipeline so `FederatedGoalConverged`/`Failed` actually emit; switched `task_to_federated_goal` to `String` keys (A2A task IDs aren't always UUIDs); fixed 13 pre-existing event-bus contract violations.
- **Convergence FK violation** — `prepare()` now threads the real `task_id` instead of `Uuid::new_v4()`, fixing SQLite error 787 against the migration-011 FK on `convergence_trajectories`.
- **Priority inversion** — `WorkflowSubtaskCompletionHandler` moved from HIGH to SYSTEM priority so workflow advancement runs alongside readiness cascade (S3).
- **Event-bus silent drops** — Hardened event delivery so dropped `TaskReady` events no longer starve tasks that are Ready in the DB but never scheduled (S5).
- **SQLite `prune_older_than()`** — Fixed COALESCE fallback so pruning still works when no watermarks exist.
- **Hourly guardrail reset** — `Guardrails::reset_hourly()` is now actually called so the hourly token limit resets (Issue #42).
- **TOCTOU race in token-limit guardrail** — Check-and-record made atomic (Issue #41).
- **Guardrail file-path matching** — `.env`/secrets patterns now match the real paths they were supposed to (Issue #48).
- **N+1 query in `calculate_depth`/`find_root_task`** — Unbounded DB queries per task collapsed into bounded traversal (Issue #43).
- **`goal_id` in `TaskSubmitted`** — No longer uses `parent_id` or fabricates a UUID (Issue #44).
- **Worktree path traversal** — Worktree paths validated before being used as git workdirs (Issue #39).
- **Convergence trajectory cascade-delete** — No more orphaned rows when a task is deleted (Issue #61).
- **Memory-store version conflicts** — Auto-increment memory version on duplicate `(namespace, key)` instead of hitting UNIQUE violations; Jaccard test data adjusted for `>0.9` similarity threshold.
- **Startup triage prompt** — The triage agent now has its real task ID injected into the user prompt instead of guessing `{{task_id}}` and failing UUID parse.
- **Unbounded prompt growth** — `heuristic_refinement_prompt` now strips existing "Refinement Notes" blocks before appending new ones, preventing 16-block accumulation in long-lived agents.
- **Workflow validating state** — `WorkflowEngine` correctly advances through the validating state.
- **Workflow subtask guard** — Workflow tasks no longer spawn untracked subtasks.
- **Reconciliation coverage** — `FastReconciliationHandler` now detects zombie Pending tasks (dependencies Blocked/Failed/Canceled) and mismatched states (S10).

#### Quality-sprint fixes (in-session)

- Tightened workflow engine state transitions — `let _ = ... .await?;` refactored; invalid subtask transitions now fail loudly.
- Hardened `extract_json_from_response` — added test coverage and removed nested `if && if` chains.
- Added diagnostic logging when `git reset --hard` fails during auto-ship recovery.
- `push_with_retry` now bails out on non-rejection push failures instead of wastefully fetch+rebase-looping.
- Mock `A2AClient` panics now carry diagnostic context via `unreachable!` instead of bare `unimplemented!`.
- Removed tautological test assertions and replaced runtime constant assertions with compile-time `const _: () = assert!(...)` checks.

## [0.3.0] - 2026-02-20

### Added

#### Core Orchestration
- **SwarmOrchestrator**: Central coordinator running the task dispatch loop, agent lifecycle management, and event-driven processing via an internal broadcast channel
- **ConvergenceEngine**: Iterative task execution lifecycle (SETUP→PREPARE→DECIDE→ITERATE→RESOLVE) with Thompson sampling bandit strategy selection and attractor classification (FixedPoint, LimitCycle, Chaotic, Diverging)
- **OverseerCluster**: Multi-phase quality gate pipeline — cheap compilation/type-check overseers run first; expensive test and acceptance overseers run last to minimize cost
- **DAG execution**: Tasks form a directed acyclic graph with parallel execution across independent branches and dependency-gated sequencing

#### Agent Evolution
- **EvolutionLoop**: Tracks per-template success rates with configurable evaluation windows; triggers minor/major refinement or immediate revert based on configurable thresholds
- **RefinementRequest persistence**: Evolution state survives process restarts via `refinement_requests` table (migration 007)
- **Auto-revert on regression**: When a refined template performs worse than its predecessor, the exact previous version is restored
- **Distinct-accessor promotion**: Memory tier promotion (Working→Episodic→Semantic) requires multiple distinct agent accessors, preventing single-agent runaway promotion

#### Memory System
- **Three-tier memory**: Working (1-hour TTL, 30-min half-life), Episodic (7-day TTL, 24-hour half-life), Semantic (no expiry, 1-week half-life)
- **MemoryDecayDaemon**: Background maintenance daemon applying configurable decay and pruning below-threshold entries
- **Full-text search** (FTS5): `memories_fts` virtual table for keyword-based memory search
- **Budget-aware context loading**: `load_context_with_budget` respects token budget and prioritizes high-relevance memories first
- **Relevance scoring**: 50% semantic similarity (TF-IDF + Jaccard + bigram) + 30% decay factor + 20% importance weight
- **Optional vector embeddings**: OpenAI embedding provider for cosine similarity search

#### Goal System
- **Convergence checks**: Periodic LLM-powered evaluation of each active goal with idempotency keys (4-hour buckets)
- **Goal context injection**: Domain inference matches active goals to tasks and injects relevant constraints into agent prompts
- **No-stagnation tracking**: `last_convergence_check_at` tracked per goal (migration 006)

#### External Adapters
- **GitHub Issues adapter**: Bidirectional — polls open issues, creates Abathur tasks; posts comments and updates issue status (requires `ABATHUR_GITHUB_TOKEN`)
- **ClickUp adapter**: Bidirectional — polls ClickUp tasks, creates items, updates status (requires `CLICKUP_API_KEY`)

#### CLI Commands
- `abathur trigger` — Manage event-driven trigger rules (add, list, delete, enable, disable)
- `abathur schedule` — Manage periodic task schedules (add, list, delete, enable, disable)
- `abathur event` — Query and inspect the event store (list, show, tail)
- `abathur workflow` — Manage workflow templates (list, show, create, apply)
- `abathur adapter` — Manage adapter plugins (list, enable, disable, status, sync)
- All 13 commands support `--json` for machine-readable output and `--config <path>`

#### MCP Integration
- **MCP stdio server**: `abathur mcp stdio` exposes swarm tools to Claude Code sessions; auto-registered in `.claude/settings.json` on `abathur init`
- **MCP HTTP servers**: Dedicated HTTP servers — memory (port 9100), tasks (port 9101), events (port 9102)
- **A2A federation gateway**: Agent-to-agent cross-swarm communication via HMAC-authenticated HTTP (port 8080, disabled by default)
- **MCP tools exposed**: `task_submit`, `task_list`, `task_get`, `task_update_status`, `task_wait`, `agent_create`, `agent_list`, `agent_get`, `memory_search`, `memory_store`, `memory_get`, `goals_list`, `adapter_list`, `egress_publish`

#### Infrastructure
- **Budget tracker**: Token/cost budget pressure tracking with configurable caution/warning/critical thresholds and per-level agent concurrency limits
- **Circuit breaker**: Failure isolation to prevent cascading failures across the swarm
- **Cold-start service**: Detects project type and populates initial memory on first `abathur swarm start`
- **Bundled SQLite**: No system SQLite required — `libsqlite3-sys` bundles SQLite 3 into the binary
- **Audit log**: Structured audit trail for all system actions stored in `audit_log` table

#### Database Migrations (applied automatically on `abathur init`)
- `001_initial_schema.sql` — Core tables: goals, tasks, task_dependencies, memories (with FTS5), agent_templates, agent_instances, worktrees, audit_log, events, trigger_rules, scheduled_events, convergence_trajectories
- `002_workflow_schema.sql` — workflow_definitions, workflow_instances, phase_instances
- `003_task_schedule_schema.sql` — task_schedules
- `004_task_type.sql` — `task_type` column on tasks (`standard | verification | research | review`)
- `005_distinct_accessors.sql` — `distinct_accessors` JSON on memories (promotion integrity)
- `006_goal_convergence_check_at.sql` — `last_convergence_check_at` on goals
- `007_refinement_requests.sql` — `refinement_requests` table for evolution loop persistence

#### Cross-compilation & Release
- Release builds for `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- `Cross.toml` configuration for Linux targets via `cross` tool
- GitHub Actions release workflow on `v*` tags with auto-generated release notes
- `install.sh` curl-pipe installer script

[Unreleased]: https://github.com/odgrim/abathur-swarm/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/odgrim/abathur-swarm/releases/tag/v0.3.0
