use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
<<<<<<< HEAD
use serde_json::Value;

/// Type of memory storage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Semantic memory - facts and knowledge
    Semantic,
    /// Episodic memory - events and experiences
    Episodic,
    /// Procedural memory - how-to knowledge and processes
    Procedural,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Semantic => write!(f, "semantic"),
            Self::Episodic => write!(f, "episodic"),
            Self::Procedural => write!(f, "procedural"),
        }
    }
}

impl std::str::FromStr for MemoryType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "semantic" => Ok(Self::Semantic),
            "episodic" => Ok(Self::Episodic),
            "procedural" => Ok(Self::Procedural),
            _ => Err(format!("Invalid memory type: {s}")),
=======
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
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
        }
    }
}

<<<<<<< HEAD
/// Memory entry with versioning and soft delete support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Auto-incrementing database ID
    pub id: i64,

    /// Hierarchical namespace (e.g., "user:alice:preferences")
    pub namespace: String,

    /// Unique key within namespace
    pub key: String,

    /// JSON value stored in memory
    pub value: Value,

    /// Type of memory
    pub memory_type: MemoryType,

    /// Version number (increments on updates)
    pub version: u32,

    /// Soft delete flag
    pub is_deleted: bool,

    /// Optional metadata as JSON
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,

    /// Creator identifier (user or agent)
    pub created_by: String,

    /// Last updater identifier (user or agent)
    pub updated_by: String,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
=======
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
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
    pub updated_at: DateTime<Utc>,
}

impl Memory {
<<<<<<< HEAD
    /// Create a new memory entry
    pub fn new(
        namespace: String,
        key: String,
        value: Value,
=======
    /// Create a new Memory instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: String,
        key: String,
        value: serde_json::Value,
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
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

<<<<<<< HEAD
    /// Create a new version of this memory with updated value
    #[must_use]
    pub fn with_new_version(&self, value: Value, updated_by: String) -> Self {
        Self {
            id: 0, // New entry in database
            namespace: self.namespace.clone(),
            key: self.key.clone(),
            value,
            memory_type: self.memory_type,
            version: self.version + 1,
            is_deleted: false,
            metadata: self.metadata.clone(),
            created_by: self.created_by.clone(),
            updated_by,
            created_at: self.created_at,
            updated_at: Utc::now(),
        }
=======
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
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
    }

    /// Mark this memory as deleted (soft delete)
    pub fn mark_deleted(&mut self) {
        self.is_deleted = true;
<<<<<<< HEAD
        self.updated_at = Utc::now();
    }

    /// Check if memory is active (not deleted)
    pub const fn is_active(&self) -> bool {
        !self.is_deleted
    }

    /// Get the full namespace path as a string
    pub fn namespace_path(&self) -> String {
        format!("{}:{}", self.namespace, self.key)
=======
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
<<<<<<< HEAD
=======
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
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
    fn test_memory_type_display() {
        assert_eq!(MemoryType::Semantic.to_string(), "semantic");
        assert_eq!(MemoryType::Episodic.to_string(), "episodic");
        assert_eq!(MemoryType::Procedural.to_string(), "procedural");
    }

    #[test]
<<<<<<< HEAD
    fn test_memory_type_from_str() {
        assert_eq!(
            "semantic".parse::<MemoryType>().unwrap(),
            MemoryType::Semantic
        );
        assert_eq!(
            "EPISODIC".parse::<MemoryType>().unwrap(),
            MemoryType::Episodic
        );
        assert_eq!(
            "Procedural".parse::<MemoryType>().unwrap(),
            MemoryType::Procedural
        );
        assert!("invalid".parse::<MemoryType>().is_err());
    }

    #[test]
    fn test_memory_new() {
        let memory = Memory::new(
            "test:namespace".to_string(),
            "key1".to_string(),
            json!({"data": "value"}),
            MemoryType::Semantic,
            "user1".to_string(),
        );

        assert_eq!(memory.namespace, "test:namespace");
        assert_eq!(memory.key, "key1");
        assert_eq!(memory.memory_type, MemoryType::Semantic);
        assert_eq!(memory.version, 1);
        assert!(!memory.is_deleted);
        assert_eq!(memory.created_by, "user1");
        assert_eq!(memory.updated_by, "user1");
    }

    #[test]
    fn test_memory_with_new_version() {
        let original = Memory::new(
            "test:namespace".to_string(),
            "key1".to_string(),
            json!({"data": "old"}),
            MemoryType::Semantic,
            "user1".to_string(),
        );

        let updated = original.with_new_version(json!({"data": "new"}), "user2".to_string());

        assert_eq!(updated.namespace, original.namespace);
        assert_eq!(updated.key, original.key);
        assert_eq!(updated.version, 2);
        assert_eq!(updated.value, json!({"data": "new"}));
        assert_eq!(updated.updated_by, "user2");
        assert_eq!(updated.created_by, "user1");
        assert!(!updated.is_deleted);
    }

    #[test]
    fn test_memory_mark_deleted() {
        let mut memory = Memory::new(
            "test:namespace".to_string(),
            "key1".to_string(),
            json!({"data": "value"}),
            MemoryType::Semantic,
            "user1".to_string(),
        );

        assert!(memory.is_active());
        memory.mark_deleted();
        assert!(!memory.is_active());
        assert!(memory.is_deleted);
    }

    #[test]
    fn test_memory_namespace_path() {
        let memory = Memory::new(
            "user:alice".to_string(),
            "preferences".to_string(),
            json!({}),
=======
    fn test_memory_creation() {
        let memory = Memory::new(
            "user:alice:preferences".to_string(),
            "theme".to_string(),
            json!({"color": "dark"}),
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
            MemoryType::Semantic,
            "alice".to_string(),
        );

<<<<<<< HEAD
        assert_eq!(memory.namespace_path(), "user:alice:preferences");
=======
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
>>>>>>> task_phase3-memory-repository_2025-10-25-23-00-04
    }
}
