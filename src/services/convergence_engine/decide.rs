//! Convergence engine -- 9.x decide phase (intentionally empty).
//!
//! # Why this module is empty
//!
//! The decide phase used to host the recursive
//! `decompose_and_coordinate` / `run_integration_trajectory` pair and the
//! `maybe_decompose_proactively` helper that fed them. Those lived here
//! because the legacy `converge()` entrypoint drove decomposition from
//! inside the engine. When that entrypoint was deleted in favour of the
//! ports-driven `run()` / `run_with_ports()` design, decomposition stopped
//! being an engine-internal concern: the engine now surfaces the decision
//! to its caller as `ConvergenceRunOutcome::Decomposed(_)` and the
//! orchestrator coordinates child convergence. With nothing left to
//! decide locally, the module emptied out.
//!
//! It is kept (rather than deleted) so the per-phase file layout stays
//! symmetric with the spec's `decide` / `iterate` / `resolve` sections,
//! and so the paired `tests/decide.rs` file has an obvious home to point
//! at. `mod decide;` in `mod.rs` and `pub mod decide;` in `tests/mod.rs`
//! both still reference this path.
//!
//! # What would eventually live here
//!
//! Anything that is a *pure, engine-local* decision about how the current
//! convergence run should proceed -- i.e. logic that needs the engine's
//! state but does not require coordinating sibling/child runs. Plausible
//! future inhabitants:
//!
//! - A structured "next action" selector that picks between iterate,
//!   resolve, decompose, or abort based on budget, basin width, and
//!   trajectory signals, replacing ad-hoc branching in `run_with_ports`.
//! - Heuristics for *when* to emit `ConvergenceRunOutcome::Decomposed`
//!   (the "should we ask the orchestrator to split?" predicate), lifted
//!   out of whatever call site currently owns them.
//! - Policy hooks for pluggable decide strategies (e.g. cost-aware vs.
//!   latency-aware), if convergence ever grows multiple decision modes.
//!
//! # When logic would come back
//!
//! This module should stay empty as long as decomposition and phase
//! transitions remain the orchestrator's responsibility and the engine's
//! `run_with_ports` body stays small enough to read top-to-bottom.
//! Re-populate it when either:
//!
//! 1. `run_with_ports` accretes enough phase-selection branching that
//!    extracting a `decide` step improves readability, or
//! 2. A new engine-internal decision (not just "decompose yes/no") needs
//!    to be made per iteration and deserves its own testable surface.
//!
//! Until one of those triggers, leave this file as a deliberate
//! placeholder -- the emptiness is the signal that the decision lives
//! elsewhere.
