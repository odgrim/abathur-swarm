//! Task outcome recording and version-change tracking.

use chrono::{DateTime, Utc};

use super::{EvolutionLoop, TaskExecution, TemplateStats};

impl EvolutionLoop {
    /// Record a task execution.
    pub async fn record_execution(&self, execution: TaskExecution) {
        // Capture data needed for persistence before taking the lock
        let mut version_change_info: Option<(String, u32, u32, TemplateStats, DateTime<Utc>)> =
            None;

        let updated_stats = {
            let mut state = self.state.write().await;

            // Check if we need to handle version change first
            let needs_version_reset = if let Some(stats) = state.stats.get(&execution.template_name)
            {
                stats.template_version != execution.template_version
            } else {
                false
            };

            if needs_version_reset {
                // Clone previous stats for regression detection
                if let Some(prev_stats) = state.stats.get(&execution.template_name).cloned() {
                    let change_time = Utc::now();
                    version_change_info = Some((
                        execution.template_name.clone(),
                        prev_stats.template_version,
                        execution.template_version,
                        prev_stats.clone(),
                        change_time,
                    ));
                    state
                        .previous_version_stats
                        .insert(execution.template_name.clone(), prev_stats);
                }
                state.version_change_times.insert(
                    execution.template_name.clone(),
                    (execution.template_version, Utc::now()),
                );
                // Remove old stats so we can insert fresh ones
                state.stats.remove(&execution.template_name);
            }

            // Update or create stats
            let stats = state
                .stats
                .entry(execution.template_name.clone())
                .or_insert_with(|| {
                    TemplateStats::new(execution.template_name.clone(), execution.template_version)
                });

            stats.update(&execution);
            let updated = stats.clone();

            // Store execution
            state
                .executions
                .entry(execution.template_name.clone())
                .or_default()
                .push(execution.clone());

            updated
        };

        // Persist to DB (fire-and-forget with warning on error)
        if let Some(ref repo) = self.refinement_repo {
            if let Err(e) = repo.save_execution(&execution).await {
                tracing::warn!(
                    "Failed to persist execution for {}: {}",
                    execution.template_name,
                    e
                );
            }
            if let Err(e) = repo.save_stats(&updated_stats).await {
                tracing::warn!(
                    "Failed to persist stats for {}: {}",
                    execution.template_name,
                    e
                );
            }
            if let Some((ref name, from_v, to_v, ref prev_stats, changed_at)) = version_change_info
                && let Err(e) = repo
                    .save_version_change(name, from_v, to_v, prev_stats, changed_at)
                    .await
            {
                tracing::warn!("Failed to persist version change for {}: {}", name, e);
            }
        }
    }

    /// Record a version change for a template.
    pub async fn record_version_change(&self, template_name: &str, new_version: u32) {
        let mut state = self.state.write().await;

        // Store current stats as previous version
        let prev_stats = state.stats.get(template_name).cloned();
        if let Some(stats) = prev_stats {
            state
                .previous_version_stats
                .insert(template_name.to_string(), stats);
        }

        // Record version change time
        state
            .version_change_times
            .insert(template_name.to_string(), (new_version, Utc::now()));
    }
}
