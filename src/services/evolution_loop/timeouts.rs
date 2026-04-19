//! Stale refinement timeout detection and expiration.

use chrono::{Duration, Utc};
use uuid::Uuid;

use super::{EvolutionLoop, RefinementStatus};

impl EvolutionLoop {
    /// Expire stale refinement requests that have been Pending or InProgress
    /// for longer than `stale_refinement_timeout_hours`.
    ///
    /// Returns the number of requests that were expired.
    /// Returns 0 immediately if the timeout is configured as 0 (disabled).
    pub async fn expire_stale_refinements(&self) -> Vec<(String, u32, Uuid)> {
        if self.config.stale_refinement_timeout_hours == 0 {
            return Vec::new();
        }

        let cutoff = Utc::now() - Duration::hours(self.config.stale_refinement_timeout_hours);
        let mut expired: Vec<(String, u32, Uuid)> = Vec::new();

        {
            let mut state = self.state.write().await;
            for request in &mut state.refinement_queue {
                if matches!(
                    request.status,
                    RefinementStatus::Pending | RefinementStatus::InProgress
                ) && request.created_at < cutoff
                {
                    request.status = RefinementStatus::Failed;
                    expired.push((
                        request.template_name.clone(),
                        request.template_version,
                        request.id,
                    ));
                }
            }
        }

        if let Some(ref repo) = self.refinement_repo {
            for (_, _, id) in &expired {
                if let Err(e) = repo.update_status(*id, RefinementStatus::Failed).await {
                    tracing::warn!(
                        "Failed to persist Failed status for stale refinement {}: {}",
                        id,
                        e
                    );
                }
            }
        }

        if !expired.is_empty() {
            tracing::info!(
                "Expired {} stale refinement request(s) older than {}h",
                expired.len(),
                self.config.stale_refinement_timeout_hours
            );
        }

        expired
    }
}
