# Changelog

All notable changes to Abathur will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **Task Summary Field**: Optional summary field for tasks (max 500 characters) to provide quick, human-readable task identification
  - Added `summary` parameter to `task_enqueue` MCP tool
  - Added `summary` field to Task domain model with Pydantic validation
  - Added `summary` column to tasks database table with idempotent migration
  - Added `summary` to task serialization in MCP responses
  - Fully backward compatible - existing code continues to work without modification
  - See documentation: `docs/features/summary-field.md`

---

## [0.1.0] - 2025-10-09

### Added
- Initial release of Abathur Hivemind Swarm Management System
- Task queue management with priority-based scheduling
- Concurrent agent swarm execution (10+ agents)
- Iterative refinement loops with convergence detection
- MCP server integration for task management
- Resource monitoring and failure recovery
- Comprehensive CLI with 20+ commands
- SQLite persistence with WAL mode
- Structured logging with audit trails
- Clean Architecture implementation
- Comprehensive test suite with >80% coverage

### Core Features
- **Task Queue**: Priority-based queue with ACID-compliant persistence
- **Agent Swarms**: Dynamic lifecycle management with health monitoring
- **Loop Execution**: Iterative refinement with checkpointing
- **MCP Integration**: Full server lifecycle management
- **Observability**: Rich CLI output with structured logging
- **Failure Recovery**: Exponential backoff with dead letter queue

---

## Release Notes

### Version 0.1.0 (2025-10-09)
Initial production-ready release of Abathur. This version includes all core functionality for task queue management, concurrent agent swarms, and iterative refinement loops. The system is built on Clean Architecture principles with comprehensive testing and observability.

**Key Highlights:**
- Production-ready task queue with ACID guarantees
- Support for 10+ concurrent Claude agents
- Multiple convergence strategies for iterative refinement
- Full MCP server integration
- Rich CLI with beautiful terminal output
- Comprehensive documentation and test coverage

**Known Issues:**
- CLI entry point may require workaround: `python -m abathur.cli.main <command>`
- See README.md for details

---

## Contributing

When adding entries to this changelog:
1. Add new changes under `[Unreleased]` section
2. Follow the format: `- **Feature Name**: Description of the change`
3. Use categories: Added, Changed, Deprecated, Removed, Fixed, Security
4. Link to relevant documentation when applicable
5. Note any breaking changes prominently

---

## Links

- [GitHub Repository](https://github.com/yourorg/abathur)
- [Documentation](docs/)
- [Issues](https://github.com/yourorg/abathur/issues)
- [Releases](https://github.com/yourorg/abathur/releases)
