use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Type of memory: semantic (facts), episodic (events), or procedural (how-to)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Semantic,
    Episodic,
    Procedural,
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            MemoryType::Semantic => "semantic",
            MemoryType::Episodic => "episodic",
            MemoryType::Procedural => "procedural",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for MemoryType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "semantic" => Ok(MemoryType::Semantic),
            "episodic" => Ok(MemoryType::Episodic),
            "procedural" => Ok(MemoryType::Procedural),
            _ => Err(anyhow::anyhow!("invalid memory type: {}", s)),
        }
    }
}

/// Memory entry in the memory service storage
///
/// Supports hierarchical namespaces, versioning, and soft deletes.
/// Values are stored as JSON for flexibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Auto-incrementing ID
    pub id: i64,
    /// Hierarchical namespace (e.g., "user:alice:preferences")
    pub namespace: String,
    /// Unique key within namespace
    pub key: String,
    /// JSON value
    pub value: serde_json::Value,
    /// Type of memory
    pub memory_type: MemoryType,
    /// Version number (incremented on updates)
    pub version: u32,
    /// Soft delete flag (true = deleted)
    pub is_deleted: bool,
    /// Optional JSON metadata
    pub metadata: Option<serde_json::Value>,
    /// Creator (user or agent)
    pub created_by: String,
    /// Last updater (user or agent)
    pub updated_by: String,
    /// Creation timestamp (ISO 8601)
    pub created_at: DateTime<Utc>,
    /// Last update timestamp (ISO 8601)
    pub updated_at: DateTime<Utc>,
}

impl Memory {
    /// Create a new Memory instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: String,
        key: String,
        value: serde_json::Value,
        memory_type: MemoryType,
        created_by: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: 0, // Will be set by database
            namespace,
            key,
            value,
            memory_type,
            version: 1,
            is_deleted: false,
            metadata: None,
            created_by: created_by.clone(),
            updated_by: created_by,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new Memory instance with metadata
    #[allow(clippy::too_many_arguments)]
    pub fn with_metadata(
        namespace: String,
        key: String,
        value: serde_json::Value,
        memory_type: MemoryType,
        metadata: serde_json::Value,
        created_by: String,
    ) -> Self {
        let mut memory = Self::new(namespace, key, value, memory_type, created_by);
        memory.metadata = Some(metadata);
        memory
    }

    /// Check if this memory is deleted
    pub fn is_deleted(&self) -> bool {
        self.is_deleted
    }

    /// Mark this memory as deleted (soft delete)
    pub fn mark_deleted(&mut self) {
        self.is_deleted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_memory_type_from_str() {
        assert_eq!(
            MemoryType::from_str("semantic").unwrap(),
            MemoryType::Semantic
        );
        assert_eq!(
            MemoryType::from_str("episodic").unwrap(),
            MemoryType::Episodic
        );
        assert_eq!(
            MemoryType::from_str("procedural").unwrap(),
            MemoryType::Procedural
        );
        assert!(MemoryType::from_str("invalid").is_err());
    }

    #[test]
    fn test_memory_type_display() {
        assert_eq!(MemoryType::Semantic.to_string(), "semantic");
        assert_eq!(MemoryType::Episodic.to_string(), "episodic");
        assert_eq!(MemoryType::Procedural.to_string(), "procedural");
    }

    #[test]
    fn test_memory_creation() {
        let memory = Memory::new(
            "user:alice:preferences".to_string(),
            "theme".to_string(),
            json!({"color": "dark"}),
            MemoryType::Semantic,
            "alice".to_string(),
        );

        assert_eq!(memory.namespace, "user:alice:preferences");
        assert_eq!(memory.key, "theme");
        assert_eq!(memory.memory_type, MemoryType::Semantic);
        assert_eq!(memory.version, 1);
        assert!(!memory.is_deleted());
        assert_eq!(memory.created_by, "alice");
        assert_eq!(memory.updated_by, "alice");
    }

    #[test]
    fn test_memory_with_metadata() {
        let memory = Memory::with_metadata(
            "user:bob:history".to_string(),
            "last_login".to_string(),
            json!("2025-10-25T12:00:00Z"),
            MemoryType::Episodic,
            json!({"source": "web"}),
            "system".to_string(),
        );

        assert!(memory.metadata.is_some());
        assert_eq!(memory.metadata.unwrap(), json!({"source": "web"}));
    }

    #[test]
    fn test_memory_soft_delete() {
        let mut memory = Memory::new(
            "test:namespace".to_string(),
            "test_key".to_string(),
            json!({}),
            MemoryType::Semantic,
            "test".to_string(),
        );

        assert!(!memory.is_deleted());
        memory.mark_deleted();
        assert!(memory.is_deleted());
    }
}
