//! Startup reconciliation: loading persisted state and recovering in-progress refinements.

use super::EvolutionLoop;

impl EvolutionLoop {
    /// Load persisted template stats and version changes from the repository.
    ///
    /// Called on startup after `recover_in_progress_refinements()` to restore
    /// in-memory evolution state from the database so stats survive restarts.
    pub async fn load_persisted_state(&self) {
        let Some(ref repo) = self.refinement_repo else {
            return;
        };

        // Load template stats
        match repo.load_all_stats().await {
            Ok(all_stats) => {
                let mut state = self.state.write().await;
                for stats in all_stats {
                    // Only insert if not already present (in-memory takes precedence)
                    state
                        .stats
                        .entry(stats.template_name.clone())
                        .or_insert(stats);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted template stats: {}", e);
            }
        }

        // Load version changes to restore previous_version_stats and version_change_times
        match repo.load_version_changes().await {
            Ok(changes) => {
                let mut state = self.state.write().await;
                for change in changes {
                    // Only insert the most recent change per template (they are ordered DESC)
                    state
                        .previous_version_stats
                        .entry(change.template_name.clone())
                        .or_insert(change.previous_stats.clone());
                    state
                        .version_change_times
                        .entry(change.template_name.clone())
                        .or_insert((change.to_version, change.changed_at));
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load persisted version changes: {}", e);
            }
        }
    }

    /// Load pending refinement requests from the repository into in-memory state.
    ///
    /// Existing in-memory entries are preserved; only new IDs (from the DB) are added.
    /// This is called on startup after `recover_in_progress_refinements()` to hydrate
    /// the in-memory queue from persisted data.
    pub async fn load_from_repo(&self) {
        let Some(ref repo) = self.refinement_repo else {
            return;
        };

        match repo.get_pending().await {
            Ok(requests) => {
                let mut state = self.state.write().await;
                for request in requests {
                    if !state.refinement_queue.iter().any(|r| r.id == request.id) {
                        state.refinement_queue.push(request);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load refinement requests from repository: {}", e);
            }
        }
    }

    /// Recover InProgress refinements from a previous process run.
    ///
    /// During startup reconciliation, any refinement that was InProgress when the
    /// process died is reset to Pending in the DB, then all pending requests are
    /// loaded into the in-memory queue so the evolution loop can re-process them.
    pub async fn recover_in_progress_refinements(&self) {
        let Some(ref repo) = self.refinement_repo else {
            return;
        };

        match repo.reset_in_progress_to_pending().await {
            Ok(recovered) if !recovered.is_empty() => {
                tracing::info!(
                    "Startup recovery: reset {} InProgress refinement request(s) to Pending",
                    recovered.len()
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("Failed to recover InProgress refinements on startup: {}", e);
            }
        }

        // Load all pending (including any just-recovered ones) into memory
        self.load_from_repo().await;
    }
}
