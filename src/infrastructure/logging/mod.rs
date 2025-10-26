//! Logging infrastructure for Abathur
//!
//! Provides:
//! - Audit logging for security-relevant operations
//! - Log rotation based on size and time
//! - Structured JSON logging with tracing

pub mod audit;
pub mod rotation;

pub use audit::{AuditEvent, AuditEventType, AuditLogger, AuditOutcome};
pub use rotation::LogRotator;
