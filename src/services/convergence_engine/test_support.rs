//! Shared test fixtures and mocks for the convergence engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::errors::DomainResult;
use crate::domain::models::convergence::*;
use crate::domain::models::task::Complexity;
use crate::domain::models::{Memory, MemoryQuery, MemoryTier};
use crate::domain::ports::{MemoryRepository, TrajectoryRepository};

use super::{ConvergenceEngine, OverseerMeasurer};

// -----------------------------------------------------------------------
// Mock TrajectoryRepository
// -----------------------------------------------------------------------

pub struct MockTrajectoryRepo {
    pub trajectories: Mutex<HashMap<String, Trajectory>>,
}

impl MockTrajectoryRepo {
    pub fn new() -> Self {
        Self {
            trajectories: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl TrajectoryRepository for MockTrajectoryRepo {
    async fn save(&self, trajectory: &Trajectory) -> DomainResult<()> {
        let mut map = self.trajectories.lock().unwrap();
        map.insert(trajectory.id.to_string(), trajectory.clone());
        Ok(())
    }

    async fn get(&self, trajectory_id: &str) -> DomainResult<Option<Trajectory>> {
        let map = self.trajectories.lock().unwrap();
        Ok(map.get(trajectory_id).cloned())
    }

    async fn get_by_task(&self, _task_id: &str) -> DomainResult<Vec<Trajectory>> {
        Ok(vec![])
    }

    async fn get_by_goal(&self, _goal_id: &str) -> DomainResult<Vec<Trajectory>> {
        Ok(vec![])
    }

    async fn get_recent(&self, _limit: usize) -> DomainResult<Vec<Trajectory>> {
        Ok(vec![])
    }

    async fn get_successful_strategies(
        &self,
        _attractor_type: &AttractorType,
        _limit: usize,
    ) -> DomainResult<Vec<StrategyEntry>> {
        Ok(vec![])
    }

    async fn delete(&self, _trajectory_id: &str) -> DomainResult<()> {
        Ok(())
    }

    async fn avg_iterations_by_complexity(&self, _complexity: Complexity) -> DomainResult<f64> {
        Ok(0.0)
    }

    async fn strategy_effectiveness(
        &self,
        _strategy: StrategyKind,
    ) -> DomainResult<crate::domain::ports::trajectory_repository::StrategyStats> {
        Ok(crate::domain::ports::trajectory_repository::StrategyStats {
            strategy: String::new(),
            total_uses: 0,
            success_count: 0,
            average_delta: 0.0,
            average_tokens: 0,
        })
    }

    async fn attractor_distribution(&self) -> DomainResult<HashMap<String, u32>> {
        Ok(HashMap::new())
    }

    async fn convergence_rate_by_task_type(&self, _category: &str) -> DomainResult<f64> {
        Ok(0.0)
    }

    async fn get_similar_trajectories(
        &self,
        _description: &str,
        _tags: &[String],
        _limit: usize,
    ) -> DomainResult<Vec<Trajectory>> {
        Ok(vec![])
    }
}

// -----------------------------------------------------------------------
// Mock MemoryRepository
// -----------------------------------------------------------------------

pub struct MockMemoryRepo {
    pub memories: Mutex<Vec<Memory>>,
}

impl MockMemoryRepo {
    pub fn new() -> Self {
        Self {
            memories: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MemoryRepository for MockMemoryRepo {
    async fn store(&self, memory: &Memory) -> DomainResult<()> {
        let mut memories = self.memories.lock().unwrap();
        memories.push(memory.clone());
        Ok(())
    }

    async fn get(&self, _id: Uuid) -> DomainResult<Option<Memory>> {
        Ok(None)
    }

    async fn get_by_key(&self, key: &str, namespace: &str) -> DomainResult<Option<Memory>> {
        let memories = self.memories.lock().unwrap();
        let found = memories
            .iter()
            .filter(|m| m.key == key && m.namespace == namespace)
            .max_by_key(|m| m.version)
            .cloned();
        Ok(found)
    }

    async fn update(&self, memory: &Memory) -> DomainResult<()> {
        let mut memories = self.memories.lock().unwrap();
        if let Some(existing) = memories.iter_mut().find(|m| m.id == memory.id) {
            *existing = memory.clone();
        }
        Ok(())
    }

    async fn delete(&self, _id: Uuid) -> DomainResult<()> {
        Ok(())
    }

    async fn query(&self, query: MemoryQuery) -> DomainResult<Vec<Memory>> {
        let memories = self.memories.lock().unwrap();
        let results: Vec<Memory> = memories
            .iter()
            .filter(|m| {
                if !query.tags.is_empty() {
                    query.tags.iter().any(|t| m.metadata.tags.contains(t))
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        Ok(results)
    }

    async fn search(
        &self,
        _query: &str,
        _namespace: Option<&str>,
        _limit: usize,
    ) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn list_by_tier(&self, _tier: MemoryTier) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn list_by_namespace(&self, _namespace: &str) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn get_expired(&self) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn prune_expired(&self) -> DomainResult<u64> {
        Ok(0)
    }

    async fn get_decayed(&self, _threshold: f32) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn get_for_task(&self, _task_id: Uuid) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn get_for_goal(&self, _goal_id: Uuid) -> DomainResult<Vec<Memory>> {
        Ok(vec![])
    }

    async fn count_by_tier(&self) -> DomainResult<std::collections::HashMap<MemoryTier, u64>> {
        Ok(HashMap::new())
    }
}

// -----------------------------------------------------------------------
// Mock OverseerMeasurer
// -----------------------------------------------------------------------

pub struct MockOverseerMeasurer {
    signals: Mutex<OverseerSignals>,
}

impl MockOverseerMeasurer {
    pub fn new() -> Self {
        Self {
            signals: Mutex::new(OverseerSignals::default()),
        }
    }

    pub fn with_signals(signals: OverseerSignals) -> Self {
        Self {
            signals: Mutex::new(signals),
        }
    }
}

#[async_trait]
impl OverseerMeasurer for MockOverseerMeasurer {
    async fn measure(
        &self,
        _artifact: &ArtifactReference,
        _policy: &ConvergencePolicy,
    ) -> DomainResult<OverseerSignals> {
        Ok(self.signals.lock().unwrap().clone())
    }
}

// -----------------------------------------------------------------------
// Test helpers
// -----------------------------------------------------------------------

pub fn test_config() -> ConvergenceEngineConfig {
    ConvergenceEngineConfig {
        default_policy: ConvergencePolicy::default(),
        max_parallel_trajectories: 3,
        enable_proactive_decomposition: false,
        memory_enabled: true,
        event_emission_enabled: false,
    }
}

pub fn build_test_engine()
-> ConvergenceEngine<MockTrajectoryRepo, MockMemoryRepo, MockOverseerMeasurer> {
    ConvergenceEngine::new(
        Arc::new(MockTrajectoryRepo::new()),
        Arc::new(MockMemoryRepo::new()),
        Arc::new(MockOverseerMeasurer::new()),
        test_config(),
    )
}
