pub mod output;

// Re-export commonly used items
pub use output::progress::{
    MultiProgressManager, ProgressBarExt, create_progress_bar, create_spinner,
};
