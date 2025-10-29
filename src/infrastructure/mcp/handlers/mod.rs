//! MCP HTTP server handlers

pub mod memory;
pub mod tasks;

pub use memory::{handle_memory_request, MemoryAppState};
pub use tasks::{handle_tasks_request, TasksAppState};
