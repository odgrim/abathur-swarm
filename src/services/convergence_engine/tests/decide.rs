//! Tests for `convergence_engine::decide`.
//!
//! The proactive-decomposition tests that lived here exercised
//! `ConvergenceEngine::maybe_decompose_proactively` -- a helper that was
//! reachable only from the legacy `converge()` entrypoint. Both the helper
//! and its recursive `decompose_and_coordinate` backing were deleted when
//! the legacy path was removed (see PR 5 follow-up). Basin-width and
//! decomposition-budget behaviour is still covered by dedicated tests in
//! `domain::models::convergence::budget`.
