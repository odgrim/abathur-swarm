//! Convergence engine -- 9.x decide phase.
//!
//! The recursive `decompose_and_coordinate` + `run_integration_trajectory`
//! pair, and the `maybe_decompose_proactively` helper that fed them, were
//! removed when the legacy `converge()` entrypoint was deleted. Under the
//! ports-driven `run()` / `run_with_ports()` entrypoint, the engine surfaces
//! decomposition to its caller via `ConvergenceRunOutcome::Decomposed(_)`
//! and the caller (orchestrator) coordinates child convergence. This module
//! intentionally has no remaining engine-phase members; it is kept as a
//! placeholder so the per-phase file layout stays symmetric with the spec's
//! decide / iterate / resolve sections.
