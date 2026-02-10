//! Overseer service implementations for the convergence engine.
//!
//! This module provides the concrete overseer implementations that measure
//! artifact quality through external verification tools. Each overseer wraps
//! a subprocess command (compilation, type checking, linting, testing, etc.)
//! and parses its output into domain-layer signal types.
//!
//! ## Architecture
//!
//! The domain layer (`domain::models::convergence::overseer`) defines the
//! [`Overseer`] trait, [`OverseerCost`], [`OverseerCluster`], and all signal
//! types. This service layer provides:
//!
//! 1. **Concrete implementations** of the [`Overseer`] trait that execute
//!    subprocess commands via `tokio::process::Command`.
//! 2. **[`OverseerClusterService`]** -- a service-layer wrapper around
//!    [`OverseerCluster`] that adds per-overseer timing and structured logging.
//! 3. **[`OverseerMeasurement`]** -- a service-layer type that pairs an
//!    [`OverseerResult`] with timing metadata.
//!
//! ## Overseer Cost Tiers
//!
//! | Cost     | Overseers                                | Phase |
//! |----------|------------------------------------------|-------|
//! | Cheap    | Compilation, TypeCheck, Build             | 1     |
//! | Moderate | Lint, SecurityScan                        | 2     |
//! | Expensive| TestSuite, AcceptanceTest                 | 3     |
//!
//! ## Usage
//!
//! ```ignore
//! use crate::services::overseers::*;
//!
//! let mut cluster = OverseerClusterService::new();
//! cluster.add(Box::new(CompilationOverseer::cargo_check()));
//! cluster.add(Box::new(TypeCheckOverseer::cargo_check()));
//! cluster.add(Box::new(BuildOverseer::cargo_build()));
//! cluster.add(Box::new(LintOverseer::cargo_clippy()));
//! cluster.add(Box::new(SecurityScanOverseer::cargo_audit()));
//! cluster.add(Box::new(TestSuiteOverseer::cargo_test()));
//! cluster.add(Box::new(AcceptanceTestOverseer::cargo_test(vec!["acceptance".into()])));
//!
//! let signals = cluster.measure(&artifact, &policy).await;
//! ```

pub mod acceptance_test;
pub mod build;
pub mod cluster;
pub mod compilation;
pub mod lint;
pub mod security_scan;
pub mod test_suite;
pub mod traits;
pub mod type_check;

pub use acceptance_test::AcceptanceTestOverseer;
pub use build::BuildOverseer;
pub use cluster::OverseerClusterService;
pub use compilation::CompilationOverseer;
pub use lint::LintOverseer;
pub use security_scan::SecurityScanOverseer;
pub use test_suite::TestSuiteOverseer;
pub use traits::{apply_signal_update, has_blocking_failures, OverseerMeasurement};
pub use type_check::TypeCheckOverseer;
