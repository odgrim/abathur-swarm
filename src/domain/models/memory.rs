//! Memory domain model.
//!
//! Three-tier memory system:
//! - Working: Ephemeral, session-scoped scratch space
//! - Episodic: Short-term memories with decay
//! - Semantic: Long-term extracted patterns and knowledge

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Memory tier classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Ephemeral, session-scoped
    Working,
    /// Short-term with decay
    Episodic,
    /// Long-term patterns
    Semantic,
}

impl Default for MemoryTier {
    fn default() -> Self {
        Self::Working
    }
}

impl MemoryTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "working" => Some(Self::Working),
            "episodic" => Some(Self::Episodic),
            "semantic" => Some(Self::Semantic),
            _ => None,
        }
    }

    /// Get default TTL for this tier.
    pub fn default_ttl(&self) -> Option<Duration> {
        match self {
            Self::Working => Some(Duration::hours(1)),
            Self::Episodic => Some(Duration::days(7)),
            Self::Semantic => None, // No expiry
        }
    }
}

/// Type of memory content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// Raw text/fact
    Fact,
    /// Code snippet
    Code,
    /// Decision or choice made
    Decision,
    /// Error or failure
    Error,
    /// Pattern or insight
    Pattern,
    /// Reference to external resource
    Reference,
    /// Agent interaction context
    Context,
}

impl Default for MemoryType {
    fn default() -> Self {
        Self::Fact
    }
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Code => "code",
            Self::Decision => "decision",
            Self::Error => "error",
            Self::Pattern => "pattern",
            Self::Reference => "reference",
            Self::Context => "context",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "fact" => Some(Self::Fact),
            "code" => Some(Self::Code),
            "decision" => Some(Self::Decision),
            "error" => Some(Self::Error),
            "pattern" => Some(Self::Pattern),
            "reference" => Some(Self::Reference),
            "context" => Some(Self::Context),
            _ => None,
        }
    }
}

/// Memory entry metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryMetadata {
    /// Source of the memory (agent, task, user)
    pub source: Option<String>,
    /// Associated task ID
    pub task_id: Option<Uuid>,
    /// Associated goal ID
    pub goal_id: Option<Uuid>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Relevance score (0.0-1.0)
    pub relevance: f32,
    /// Custom key-value pairs
    pub custom: std::collections::HashMap<String, serde_json::Value>,
}

/// A memory entry in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique identifier
    pub id: Uuid,
    /// Memory key (for lookup)
    pub key: String,
    /// Namespace for organization
    pub namespace: String,
    /// Memory content
    pub content: String,
    /// Memory tier
    pub tier: MemoryTier,
    /// Memory type
    pub memory_type: MemoryType,
    /// Metadata
    pub metadata: MemoryMetadata,
    /// Access count (for decay calculation)
    pub access_count: u32,
    /// Last access time
    pub last_accessed: DateTime<Utc>,
    /// When created
    pub created_at: DateTime<Utc>,
    /// When updated
    pub updated_at: DateTime<Utc>,
    /// Expiration time (None = never expires)
    pub expires_at: Option<DateTime<Utc>>,
    /// Version for optimistic locking
    pub version: u64,
}

impl Memory {
    /// Create a new working memory.
    pub fn working(key: impl Into<String>, content: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            key: key.into(),
            namespace: "default".to_string(),
            content: content.into(),
            tier: MemoryTier::Working,
            memory_type: MemoryType::Fact,
            metadata: MemoryMetadata::default(),
            access_count: 0,
            last_accessed: now,
            created_at: now,
            updated_at: now,
            expires_at: Some(now + Duration::hours(1)),
            version: 1,
        }
    }

    /// Create a new episodic memory.
    pub fn episodic(key: impl Into<String>, content: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            key: key.into(),
            namespace: "default".to_string(),
            content: content.into(),
            tier: MemoryTier::Episodic,
            memory_type: MemoryType::Fact,
            metadata: MemoryMetadata::default(),
            access_count: 0,
            last_accessed: now,
            created_at: now,
            updated_at: now,
            expires_at: Some(now + Duration::days(7)),
            version: 1,
        }
    }

    /// Create a new semantic memory (no expiry).
    pub fn semantic(key: impl Into<String>, content: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            key: key.into(),
            namespace: "default".to_string(),
            content: content.into(),
            tier: MemoryTier::Semantic,
            memory_type: MemoryType::Pattern,
            metadata: MemoryMetadata::default(),
            access_count: 0,
            last_accessed: now,
            created_at: now,
            updated_at: now,
            expires_at: None,
            version: 1,
        }
    }

    /// Set namespace.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = namespace.into();
        self
    }

    /// Set memory type.
    pub fn with_type(mut self, memory_type: MemoryType) -> Self {
        self.memory_type = memory_type;
        self
    }

    /// Set source.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.metadata.source = Some(source.into());
        self
    }

    /// Set associated task.
    pub fn with_task(mut self, task_id: Uuid) -> Self {
        self.metadata.task_id = Some(task_id);
        self
    }

    /// Set associated goal.
    pub fn with_goal(mut self, goal_id: Uuid) -> Self {
        self.metadata.goal_id = Some(goal_id);
        self
    }

    /// Add a tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.metadata.tags.push(tag.into());
        self
    }

    /// Set TTL from now.
    pub fn with_ttl(mut self, duration: Duration) -> Self {
        self.expires_at = Some(Utc::now() + duration);
        self
    }

    /// Check if memory is expired.
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() > exp,
            None => false,
        }
    }

    /// Record an access (updates access count and last_accessed).
    pub fn record_access(&mut self) {
        self.access_count += 1;
        self.last_accessed = Utc::now();
        self.updated_at = Utc::now();
        self.version += 1;
    }

    /// Calculate decay factor (0.0 = fully decayed, 1.0 = fresh).
    /// Uses exponential decay based on time since last access.
    pub fn decay_factor(&self) -> f32 {
        let age = Utc::now() - self.last_accessed;
        let hours = age.num_hours() as f32;

        // Half-life depends on tier
        let half_life_hours = match self.tier {
            MemoryTier::Working => 0.5,   // 30 minutes
            MemoryTier::Episodic => 24.0, // 1 day
            MemoryTier::Semantic => 168.0, // 1 week (but never expires)
        };

        // Exponential decay: factor = 2^(-age/half_life)
        // Access count slows decay
        let access_bonus = (self.access_count as f32).ln_1p() * 0.1;
        let effective_age = (hours - access_bonus).max(0.0);

        0.5_f32.powf(effective_age / half_life_hours)
    }

    /// Promote memory to higher tier.
    pub fn promote(&mut self) -> Result<(), String> {
        match self.tier {
            MemoryTier::Working => {
                self.tier = MemoryTier::Episodic;
                self.expires_at = Some(Utc::now() + Duration::days(7));
            }
            MemoryTier::Episodic => {
                self.tier = MemoryTier::Semantic;
                self.expires_at = None;
            }
            MemoryTier::Semantic => {
                return Err("Cannot promote semantic memory".to_string());
            }
        }
        self.updated_at = Utc::now();
        self.version += 1;
        Ok(())
    }

    /// Calculate importance score (0.0-1.0) based on access patterns and tier.
    ///
    /// Importance combines:
    /// - Tier weight: Semantic > Episodic > Working
    /// - Access frequency: More accesses = more important
    /// - Explicit relevance: User-assigned relevance score from metadata
    pub fn importance_score(&self) -> f32 {
        let tier_weight = match self.tier {
            MemoryTier::Semantic => 0.8,
            MemoryTier::Episodic => 0.5,
            MemoryTier::Working => 0.2,
        };

        // Access frequency factor: logarithmic scaling to prevent runaway values
        // ln(1 + access_count) / ln(1 + 100) gives 0.0 - 1.0 range for 0-100 accesses
        let access_factor = (1.0 + self.access_count as f32).ln() / (1.0 + 100.0_f32).ln();
        let access_factor = access_factor.min(1.0);

        // Combine: 40% tier, 30% access frequency, 30% explicit relevance
        let explicit_relevance = self.metadata.relevance;
        0.4 * tier_weight + 0.3 * access_factor + 0.3 * explicit_relevance
    }

    /// Compute multi-factor relevance score for this memory given a text query.
    ///
    /// Implements the research-recommended scoring formula:
    ///   score = w_semantic * text_similarity + w_decay * decay_factor + w_importance * importance
    ///
    /// Where text_similarity is Jaccard coefficient between query words and memory content.
    pub fn relevance_score(&self, query: &str, weights: &RelevanceWeights) -> ScoredMemory {
        let weights = weights.normalized();

        // Semantic score: word-overlap similarity with query
        let semantic_score = if query.is_empty() {
            0.5 // Neutral score when no query provided
        } else {
            Self::text_similarity(query, &self.content)
        };

        // Also check key and tags for relevance boost
        let key_boost = if !query.is_empty() {
            let key_sim = Self::text_similarity(query, &self.key);
            let tag_text = self.metadata.tags.join(" ");
            let tag_sim = Self::text_similarity(query, &tag_text);
            (key_sim * 0.3 + tag_sim * 0.2).min(0.3) // Max 0.3 boost from key/tags
        } else {
            0.0
        };
        let semantic_score = (semantic_score + key_boost).min(1.0);

        let decay_score = self.decay_factor();
        let importance_score = self.importance_score();

        let composite = weights.semantic_weight * semantic_score
            + weights.decay_weight * decay_score
            + weights.importance_weight * importance_score;

        ScoredMemory {
            memory: self.clone(),
            score: composite.min(1.0),
            score_breakdown: ScoreBreakdown {
                semantic_score,
                decay_score,
                importance_score,
            },
        }
    }

    /// Compute Jaccard word-overlap similarity between two text strings.
    fn text_similarity(text_a: &str, text_b: &str) -> f32 {
        if text_a.is_empty() && text_b.is_empty() {
            return 1.0;
        }

        let lower_a = text_a.to_lowercase();
        let lower_b = text_b.to_lowercase();
        let words_a: std::collections::HashSet<&str> = lower_a.split_whitespace().collect();
        let words_b: std::collections::HashSet<&str> = lower_b.split_whitespace().collect();

        if words_a.is_empty() && words_b.is_empty() {
            return 1.0;
        }

        let intersection = words_a.intersection(&words_b).count() as f32;
        let union = words_a.union(&words_b).count() as f32;

        if union == 0.0 {
            return 1.0;
        }

        intersection / union
    }

    /// Estimate the token count of this memory's content.
    /// Uses a rough heuristic of ~4 characters per token.
    pub fn estimated_tokens(&self) -> usize {
        // Rough approximation: 1 token ≈ 4 characters for English text
        (self.content.len() + 3) / 4
    }

    /// Validate memory.
    pub fn validate(&self) -> Result<(), String> {
        if self.key.is_empty() {
            return Err("Memory key cannot be empty".to_string());
        }
        if self.namespace.is_empty() {
            return Err("Memory namespace cannot be empty".to_string());
        }
        if self.content.is_empty() {
            return Err("Memory content cannot be empty".to_string());
        }
        Ok(())
    }
}

/// Configuration for multi-factor relevance scoring.
///
/// Based on research from DynTaskMAS and Manus AI context engineering,
/// memory retrieval should use a weighted combination of:
/// - Semantic relevance (text match quality)
/// - Temporal decay (recency of access)
/// - Importance (access frequency + tier weight)
///
/// Formula: score = w_relevance * semantic + w_decay * decay + w_importance * importance
#[derive(Debug, Clone)]
pub struct RelevanceWeights {
    /// Weight for semantic/text relevance (0.0-1.0)
    pub semantic_weight: f32,
    /// Weight for temporal decay factor (0.0-1.0)
    pub decay_weight: f32,
    /// Weight for importance/access-frequency factor (0.0-1.0)
    pub importance_weight: f32,
}

impl Default for RelevanceWeights {
    fn default() -> Self {
        Self {
            semantic_weight: 0.5,
            decay_weight: 0.3,
            importance_weight: 0.2,
        }
    }
}

impl RelevanceWeights {
    /// Create weights biased towards semantic relevance (good for search queries).
    pub fn semantic_biased() -> Self {
        Self {
            semantic_weight: 0.7,
            decay_weight: 0.15,
            importance_weight: 0.15,
        }
    }

    /// Create weights biased towards recency (good for session context).
    pub fn recency_biased() -> Self {
        Self {
            semantic_weight: 0.2,
            decay_weight: 0.6,
            importance_weight: 0.2,
        }
    }

    /// Create weights biased towards importance (good for stable knowledge).
    pub fn importance_biased() -> Self {
        Self {
            semantic_weight: 0.2,
            decay_weight: 0.2,
            importance_weight: 0.6,
        }
    }

    /// Normalize weights so they sum to 1.0.
    pub fn normalized(&self) -> Self {
        let sum = self.semantic_weight + self.decay_weight + self.importance_weight;
        if sum == 0.0 {
            return Self::default();
        }
        Self {
            semantic_weight: self.semantic_weight / sum,
            decay_weight: self.decay_weight / sum,
            importance_weight: self.importance_weight / sum,
        }
    }
}

/// A scored memory entry with its composite relevance score.
#[derive(Debug, Clone)]
pub struct ScoredMemory {
    /// The memory entry.
    pub memory: Memory,
    /// Composite relevance score (0.0-1.0).
    pub score: f32,
    /// Breakdown of score components.
    pub score_breakdown: ScoreBreakdown,
}

/// Breakdown of how a memory's relevance score was computed.
#[derive(Debug, Clone, Default)]
pub struct ScoreBreakdown {
    /// Semantic/text relevance component (0.0-1.0).
    pub semantic_score: f32,
    /// Temporal decay component (0.0-1.0).
    pub decay_score: f32,
    /// Importance component (0.0-1.0).
    pub importance_score: f32,
}

/// Query specification for memory retrieval.
#[derive(Debug, Clone, Default)]
pub struct MemoryQuery {
    /// Key pattern (supports wildcards)
    pub key_pattern: Option<String>,
    /// Namespace filter
    pub namespace: Option<String>,
    /// Tier filter
    pub tier: Option<MemoryTier>,
    /// Type filter
    pub memory_type: Option<MemoryType>,
    /// Tag filter (any match)
    pub tags: Vec<String>,
    /// Minimum decay factor
    pub min_decay: Option<f32>,
    /// Associated task
    pub task_id: Option<Uuid>,
    /// Associated goal
    pub goal_id: Option<Uuid>,
    /// Full-text search query
    pub search_query: Option<String>,
    /// Maximum results
    pub limit: Option<usize>,
    /// Relevance weights for multi-factor scoring (if None, no scoring applied).
    pub relevance_weights: Option<RelevanceWeights>,
    /// Minimum relevance score threshold (only return memories above this score).
    pub min_relevance_score: Option<f32>,
}

impl MemoryQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn tier(mut self, tier: MemoryTier) -> Self {
        self.tier = Some(tier);
        self
    }

    pub fn key_like(mut self, pattern: impl Into<String>) -> Self {
        self.key_pattern = Some(pattern.into());
        self
    }

    pub fn search(mut self, query: impl Into<String>) -> Self {
        self.search_query = Some(query.into());
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn for_task(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub fn for_goal(mut self, goal_id: Uuid) -> Self {
        self.goal_id = Some(goal_id);
        self
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_working_memory() {
        let mem = Memory::working("test_key", "test content");
        assert_eq!(mem.tier, MemoryTier::Working);
        assert!(mem.expires_at.is_some());
        assert!(!mem.is_expired());
    }

    #[test]
    fn test_semantic_memory() {
        let mem = Memory::semantic("pattern_key", "learned pattern");
        assert_eq!(mem.tier, MemoryTier::Semantic);
        assert!(mem.expires_at.is_none());
        assert!(!mem.is_expired());
    }

    #[test]
    fn test_memory_promotion() {
        let mut mem = Memory::working("key", "content");
        assert_eq!(mem.tier, MemoryTier::Working);

        mem.promote().unwrap();
        assert_eq!(mem.tier, MemoryTier::Episodic);

        mem.promote().unwrap();
        assert_eq!(mem.tier, MemoryTier::Semantic);

        assert!(mem.promote().is_err()); // Cannot promote further
    }

    #[test]
    fn test_decay_factor() {
        let mem = Memory::working("key", "content");
        // Fresh memory should have high decay factor
        assert!(mem.decay_factor() > 0.9);
    }

    #[test]
    fn test_record_access() {
        let mut mem = Memory::working("key", "content");
        assert_eq!(mem.access_count, 0);

        mem.record_access();
        assert_eq!(mem.access_count, 1);
        assert!(mem.version > 1);
    }

    #[test]
    fn test_memory_validation() {
        let mem = Memory::working("", "content");
        assert!(mem.validate().is_err());

        let mem = Memory::working("key", "");
        assert!(mem.validate().is_err());

        let mem = Memory::working("key", "content");
        assert!(mem.validate().is_ok());
    }

    #[test]
    fn test_importance_score() {
        let working = Memory::working("key", "content");
        let episodic = Memory::episodic("key", "content");
        let semantic = Memory::semantic("key", "content");

        // Semantic memories should have higher importance than episodic than working
        assert!(semantic.importance_score() > episodic.importance_score());
        assert!(episodic.importance_score() > working.importance_score());
    }

    #[test]
    fn test_importance_increases_with_access() {
        let mut mem = Memory::working("key", "content");
        let score_before = mem.importance_score();

        // Simulate many accesses
        for _ in 0..20 {
            mem.record_access();
        }

        let score_after = mem.importance_score();
        assert!(score_after > score_before, "Importance should increase with access count");
    }

    #[test]
    fn test_relevance_score_with_matching_query() {
        let mem = Memory::working("rust_patterns", "Common patterns in Rust programming include iterators and closures");
        let weights = RelevanceWeights::semantic_biased();

        let scored = mem.relevance_score("Rust patterns iterators", &weights);
        let scored_unrelated = mem.relevance_score("python database migrations", &weights);

        assert!(scored.score > scored_unrelated.score,
            "Matching query should score higher: {} vs {}",
            scored.score, scored_unrelated.score);
    }

    #[test]
    fn test_relevance_score_breakdown() {
        let mem = Memory::semantic("key", "test content for scoring");
        let weights = RelevanceWeights::default();

        let scored = mem.relevance_score("test content", &weights);

        // All breakdown components should be between 0 and 1
        assert!(scored.score_breakdown.semantic_score >= 0.0 && scored.score_breakdown.semantic_score <= 1.0);
        assert!(scored.score_breakdown.decay_score >= 0.0 && scored.score_breakdown.decay_score <= 1.0);
        assert!(scored.score_breakdown.importance_score >= 0.0 && scored.score_breakdown.importance_score <= 1.0);

        // Composite score should be between 0 and 1
        assert!(scored.score >= 0.0 && scored.score <= 1.0);
    }

    #[test]
    fn test_relevance_weights_normalization() {
        let weights = RelevanceWeights {
            semantic_weight: 2.0,
            decay_weight: 1.0,
            importance_weight: 1.0,
        };
        let normalized = weights.normalized();
        let sum = normalized.semantic_weight + normalized.decay_weight + normalized.importance_weight;
        assert!((sum - 1.0).abs() < 0.001, "Normalized weights should sum to 1.0, got {}", sum);
    }

    #[test]
    fn test_estimated_tokens() {
        let mem = Memory::working("key", "This is a test memory with some content for estimation.");
        let tokens = mem.estimated_tokens();
        assert!(tokens > 0);
        // ~56 chars / 4 = ~14 tokens
        assert!(tokens > 10 && tokens < 20, "Expected ~14 tokens, got {}", tokens);
    }

    #[test]
    fn test_text_similarity() {
        let sim = Memory::text_similarity("hello world", "hello world");
        assert!((sim - 1.0).abs() < 0.001, "Identical strings should have similarity 1.0");

        let sim = Memory::text_similarity("hello world", "goodbye universe");
        assert!(sim < 0.1, "Completely different strings should have low similarity");

        let sim = Memory::text_similarity("rust programming patterns", "rust patterns iterators");
        assert!(sim > 0.3, "Partially overlapping strings should have moderate similarity");
    }

    #[test]
    fn test_memory_query_builder() {
        let query = MemoryQuery::new()
            .namespace("agents")
            .tier(MemoryTier::Semantic)
            .search("pattern")
            .limit(10);

        assert_eq!(query.namespace, Some("agents".to_string()));
        assert_eq!(query.tier, Some(MemoryTier::Semantic));
        assert_eq!(query.search_query, Some("pattern".to_string()));
        assert_eq!(query.limit, Some(10));
    }
}
