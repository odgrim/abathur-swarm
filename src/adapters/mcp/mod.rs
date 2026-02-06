//! MCP (Model Context Protocol) HTTP server adapters.
//!
//! These servers expose Abathur's capabilities via HTTP for Claude Code
//! agents to interact with memory, task, and agent systems.
//!
//! Also includes the A2A (Agent-to-Agent) HTTP gateway for inter-agent
//! communication following the A2A protocol specification.

pub mod a2a_http;
pub mod agents_http;
pub mod events_http;
pub mod memory_http;
pub mod tasks_http;

pub use a2a_http::{A2AHttpConfig, A2AHttpGateway, A2AState, A2ATaskState, FederationClient};
pub use agents_http::{AgentsHttpConfig, AgentsHttpServer};
pub use events_http::{EventsHttpConfig, EventsHttpServer, EventsState};
pub use memory_http::{MemoryHttpConfig, MemoryHttpServer};
pub use tasks_http::{TasksHttpConfig, TasksHttpServer};
