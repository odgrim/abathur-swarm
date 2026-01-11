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
