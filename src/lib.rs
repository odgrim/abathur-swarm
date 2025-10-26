pub mod domain;
pub mod infrastructure;

pub use domain::models::{DependencyType, Task, TaskSource, TaskStatus};
pub use domain::ports::{DatabaseError, TaskFilters, TaskRepository};
pub use infrastructure::database::{DatabaseConnection, TaskRepositoryImpl};
