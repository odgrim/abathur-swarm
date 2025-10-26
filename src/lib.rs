pub mod domain;
pub mod infrastructure;

// Re-export commonly used types
pub use domain::models::{Agent, AgentStatus};
pub use domain::ports::{AgentRepository, DatabaseError};
pub use infrastructure::database::{AgentRepositoryImpl, DatabaseConnection};
