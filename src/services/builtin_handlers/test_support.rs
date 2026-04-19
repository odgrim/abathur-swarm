//! Shared test fixtures for built-in handler tests.
//!
//! This module is retained for backward compatibility and simply re-exports
//! helpers from `crate::adapters::sqlite::test_support`, which is the canonical
//! location for SQLite-backed test helpers.

#![allow(unused_imports)]
#![allow(dead_code)]

pub use crate::adapters::sqlite::test_support::{make_task_service, setup_task_repo};
