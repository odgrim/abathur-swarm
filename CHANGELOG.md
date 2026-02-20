# Changelog

All notable changes to Abathur are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
