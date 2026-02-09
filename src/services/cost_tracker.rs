//! Model-aware cost tracking with per-model pricing.
//!
//! Provides accurate cost estimation based on actual model used
//! and token counts (input, output, cache read, cache write).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Pricing per million tokens for a specific model.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    /// Cost per million input tokens (USD).
    pub input: f64,
    /// Cost per million output tokens (USD).
    pub output: f64,
    /// Cost per million cache read tokens (USD).
    pub cache_read: f64,
    /// Cost per million cache write tokens (USD).
    pub cache_write: f64,
}

/// Known model pricing table (costs in USD per million tokens).
const PRICING_TABLE: &[(&str, ModelPricing)] = &[
    (
        "claude-opus-4-6",
        ModelPricing { input: 15.0, output: 75.0, cache_read: 1.5, cache_write: 18.75 },
    ),
    (
        "opus",
        ModelPricing { input: 15.0, output: 75.0, cache_read: 1.5, cache_write: 18.75 },
    ),
    (
        "claude-sonnet-4-5",
        ModelPricing { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 },
    ),
    (
        "sonnet",
        ModelPricing { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 },
    ),
    (
        "claude-haiku-4-5",
        ModelPricing { input: 0.80, output: 4.0, cache_read: 0.08, cache_write: 1.0 },
    ),
    (
        "haiku",
        ModelPricing { input: 0.80, output: 4.0, cache_read: 0.08, cache_write: 1.0 },
    ),
];

/// Get pricing for a model by name or alias.
///
/// Matches against known model name substrings (e.g. "opus" matches
/// "claude-opus-4-6-20250616").
pub fn get_model_pricing(model: &str) -> Option<ModelPricing> {
    let model_lower = model.to_lowercase();
    PRICING_TABLE
        .iter()
        .find(|(name, _)| model_lower.contains(name))
        .map(|(_, pricing)| *pricing)
}

/// Estimate cost in USD for a given set of token counts.
pub fn estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let pricing = get_model_pricing(model)?;

    let cost = (input_tokens as f64 * pricing.input
        + output_tokens as f64 * pricing.output
        + cache_read_tokens as f64 * pricing.cache_read
        + cache_write_tokens as f64 * pricing.cache_write)
        / 1_000_000.0;

    Some(cost)
}

/// Estimate cost in cents.
pub fn estimate_cost_cents(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    estimate_cost(model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens)
        .map(|usd| usd * 100.0)
}

/// Summary of costs for a goal or execution run.
#[derive(Debug, Clone, Default)]
pub struct CostSummary {
    /// Total cost in USD.
    pub total_usd: f64,
    /// Breakdown by model.
    pub by_model: HashMap<String, f64>,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Total cache read tokens.
    pub total_cache_read_tokens: u64,
    /// Total cache write tokens.
    pub total_cache_write_tokens: u64,
    /// Number of tasks tracked.
    pub task_count: u32,
}

impl CostSummary {
    /// Total cost in cents.
    pub fn total_cents(&self) -> f64 {
        self.total_usd * 100.0
    }

    /// Add a task's cost to the summary.
    pub fn add_task(
        &mut self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) {
        self.total_input_tokens += input_tokens;
        self.total_output_tokens += output_tokens;
        self.total_cache_read_tokens += cache_read_tokens;
        self.total_cache_write_tokens += cache_write_tokens;
        self.task_count += 1;

        if let Some(cost) = estimate_cost(
            model,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        ) {
            self.total_usd += cost;
            *self.by_model.entry(model.to_string()).or_default() += cost;
        }
    }

    /// Format as a human-readable summary.
    pub fn format_summary(&self) -> String {
        let mut s = format!(
            "Cost: ${:.4} ({} tasks, {}K input, {}K output",
            self.total_usd,
            self.task_count,
            self.total_input_tokens / 1000,
            self.total_output_tokens / 1000,
        );

        if self.total_cache_read_tokens > 0 {
            s.push_str(&format!(", {}K cache_read", self.total_cache_read_tokens / 1000));
        }
        if self.total_cache_write_tokens > 0 {
            s.push_str(&format!(", {}K cache_write", self.total_cache_write_tokens / 1000));
        }
        s.push(')');

        if self.by_model.len() > 1 {
            s.push_str("\n  By model:");
            let mut models: Vec<_> = self.by_model.iter().collect();
            models.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            for (model, cost) in models {
                s.push_str(&format!("\n    {}: ${:.4}", model, cost));
            }
        }

        s
    }
}

/// Cost tracker that accumulates costs across tasks and goals.
#[derive(Debug, Clone)]
pub struct CostTracker {
    goal_costs: Arc<RwLock<HashMap<Uuid, CostSummary>>>,
    global: Arc<RwLock<CostSummary>>,
}

impl CostTracker {
    pub fn new() -> Self {
        Self {
            goal_costs: Arc::new(RwLock::new(HashMap::new())),
            global: Arc::new(RwLock::new(CostSummary::default())),
        }
    }

    /// Record a task's cost.
    pub async fn record_task(
        &self,
        goal_id: Option<Uuid>,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) {
        {
            let mut global = self.global.write().await;
            global.add_task(model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens);
        }

        if let Some(gid) = goal_id {
            let mut goals = self.goal_costs.write().await;
            goals
                .entry(gid)
                .or_default()
                .add_task(model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens);
        }
    }

    /// Get the global cost summary.
    pub async fn global_summary(&self) -> CostSummary {
        self.global.read().await.clone()
    }

    /// Get cost summary for a specific goal.
    pub async fn goal_summary(&self, goal_id: Uuid) -> Option<CostSummary> {
        self.goal_costs.read().await.get(&goal_id).cloned()
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_model_pricing_alias() {
        let pricing = get_model_pricing("opus").unwrap();
        assert_eq!(pricing.input, 15.0);
        assert_eq!(pricing.output, 75.0);
    }

    #[test]
    fn test_get_model_pricing_full_name() {
        let pricing = get_model_pricing("claude-opus-4-6-20250616").unwrap();
        assert_eq!(pricing.input, 15.0);
    }

    #[test]
    fn test_get_model_pricing_haiku() {
        let pricing = get_model_pricing("haiku").unwrap();
        assert_eq!(pricing.input, 0.80);
    }

    #[test]
    fn test_estimate_cost_input_only() {
        // 1M input tokens with opus = $15
        let cost = estimate_cost("opus", 1_000_000, 0, 0, 0).unwrap();
        assert!((cost - 15.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_output_only() {
        // 1M output tokens with opus = $75
        let cost = estimate_cost("opus", 0, 1_000_000, 0, 0).unwrap();
        assert!((cost - 75.0).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_cache_read() {
        // 1M cache read tokens with opus = $1.50 (10x cheaper than input)
        let cost = estimate_cost("opus", 0, 0, 1_000_000, 0).unwrap();
        assert!((cost - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_estimate_cost_cents() {
        let cents = estimate_cost_cents("opus", 1_000_000, 0, 0, 0).unwrap();
        assert!((cents - 1500.0).abs() < 0.1);
    }

    #[test]
    fn test_cost_summary_single_task() {
        let mut summary = CostSummary::default();
        summary.add_task("opus", 10_000, 5_000, 0, 0);

        assert_eq!(summary.task_count, 1);
        assert_eq!(summary.total_input_tokens, 10_000);
        assert_eq!(summary.total_output_tokens, 5_000);
        // 10K * 15 / 1M + 5K * 75 / 1M = 0.15 + 0.375 = 0.525
        assert!((summary.total_usd - 0.525).abs() < 0.001);
    }

    #[test]
    fn test_cost_summary_multiple_models() {
        let mut summary = CostSummary::default();
        summary.add_task("opus", 10_000, 5_000, 0, 0);
        summary.add_task("haiku", 100_000, 50_000, 0, 0);

        assert_eq!(summary.task_count, 2);
        assert_eq!(summary.by_model.len(), 2);
        let formatted = summary.format_summary();
        assert!(formatted.contains("By model:"));
    }

    #[test]
    fn test_unknown_model_returns_none() {
        assert!(get_model_pricing("unknown-model").is_none());
        assert!(estimate_cost("unknown-model", 1000, 1000, 0, 0).is_none());
    }

    #[tokio::test]
    async fn test_cost_tracker_global_and_goal() {
        let tracker = CostTracker::new();
        let goal_id = Uuid::new_v4();

        tracker.record_task(Some(goal_id), "opus", 10_000, 5_000, 0, 0).await;
        tracker.record_task(Some(goal_id), "haiku", 20_000, 10_000, 0, 0).await;
        tracker.record_task(None, "sonnet", 5_000, 2_000, 0, 0).await;

        let global = tracker.global_summary().await;
        assert_eq!(global.task_count, 3);
        assert_eq!(global.total_input_tokens, 35_000);

        let goal = tracker.goal_summary(goal_id).await.unwrap();
        assert_eq!(goal.task_count, 2);
        assert_eq!(goal.total_input_tokens, 30_000);
    }
}
