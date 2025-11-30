//! Integration tests for memory storage namespace conventions
//!
//! This test suite verifies that memory storage follows the correct
//! namespace patterns, particularly for task-related memories:
//! - task:{task_id}:requirements
//! - task:{task_id}:technical_specs
//! - task:{task_id}:results
//!
//! Test Coverage:
//! - memory_add stores to correct namespace pattern
//! - memory_get retrieves from task:{task_id}:requirements
//! - JSON value serialization/deserialization
//! - Soft-delete behavior
//! - created_by and updated_by field tracking

use abathur_cli::domain::models::{Memory, MemoryType};
use abathur_cli::domain::ports::MemoryRepository;
use abathur_cli::infrastructure::database::MemoryRepositoryImpl;
use abathur_cli::services::MemoryService;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::helpers::database::{setup_test_db, teardown_test_db};

/// Test that memory_add stores to the correct namespace pattern
#[tokio::test]
async fn test_memory_add_stores_to_correct_namespace_pattern() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let requirements_data = json!({
        "problem_statement": "User needs authentication system",
        "functional_requirements": [
            "User registration with email/password",
            "Login with JWT tokens",
            "Password reset functionality"
        ],
        "non_functional_requirements": {
            "security": "Passwords must be hashed with bcrypt",
            "performance": "Login response time < 200ms"
        }
    });

    let memory = Memory::new(
        namespace.clone(),
        "gathered_requirements".to_string(),
        requirements_data.clone(),
        MemoryType::Episodic,
        "requirements-gatherer".to_string(),
    );

    // Act: Add memory
    let result = service.add(memory.clone()).await;
    assert!(result.is_ok(), "Failed to add memory: {:?}", result.err());

    // Assert: Verify it's stored in the correct namespace
    let retrieved = service
        .get(&namespace, "gathered_requirements")
        .await
        .expect("Failed to retrieve memory");

    assert!(retrieved.is_some(), "Memory should be stored");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.namespace, namespace);
    assert_eq!(retrieved.key, "gathered_requirements");
    assert_eq!(retrieved.value, requirements_data);
    assert_eq!(retrieved.memory_type, MemoryType::Episodic);

    teardown_test_db(pool).await;
}

/// Test that memory_get retrieves from task:{task_id}:requirements namespace
#[tokio::test]
async fn test_memory_get_retrieves_from_task_requirements_namespace() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let requirements = json!({
        "problem": "Implement user authentication",
        "constraints": ["Must use existing database schema"]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements".to_string(),
        requirements.clone(),
        MemoryType::Episodic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve from specific namespace
    let retrieved = service
        .get(&namespace, "requirements")
        .await
        .expect("Failed to retrieve memory");

    // Assert: Verify correct retrieval
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.namespace, namespace);
    assert_eq!(retrieved.value, requirements);

    teardown_test_db(pool).await;
}

/// Test JSON value serialization and deserialization
#[tokio::test]
async fn test_json_value_serialization_deserialization() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    // Complex nested JSON structure
    let complex_data = json!({
        "functional_requirements": [
            {
                "id": "FR-001",
                "description": "User authentication",
                "priority": "high",
                "acceptance_criteria": [
                    "Users can register with email",
                    "Users can login with credentials",
                    "Session expires after 24 hours"
                ]
            },
            {
                "id": "FR-002",
                "description": "User profile management",
                "priority": "medium",
                "acceptance_criteria": [
                    "Users can update profile information",
                    "Profile changes are validated"
                ]
            }
        ],
        "non_functional_requirements": {
            "performance": {
                "response_time_p95": "200ms",
                "throughput": "1000 req/s"
            },
            "security": {
                "encryption": "AES-256",
                "password_hashing": "bcrypt with cost 12"
            },
            "scalability": {
                "concurrent_users": 10000,
                "horizontal_scaling": true
            }
        },
        "constraints": [
            "Must use PostgreSQL",
            "Must be deployable on AWS",
            "Must comply with GDPR"
        ],
        "metadata": {
            "version": "1.0.0",
            "last_updated": "2025-11-29T00:00:00Z",
            "confidence_score": 0.95
        }
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_v1".to_string(),
        complex_data.clone(),
        MemoryType::Episodic,
        "requirements-gatherer".to_string(),
    );

    // Act: Store and retrieve complex JSON
    service.add(memory).await.expect("Failed to add memory");
    let retrieved = service
        .get(&namespace, "requirements_v1")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    // Assert: Verify exact JSON match
    assert_eq!(retrieved.value, complex_data);

    // Verify nested values are accessible
    assert_eq!(
        retrieved.value["functional_requirements"][0]["id"],
        "FR-001"
    );
    assert_eq!(
        retrieved.value["non_functional_requirements"]["performance"]["response_time_p95"],
        "200ms"
    );
    assert_eq!(retrieved.value["constraints"][0], "Must use PostgreSQL");
    assert_eq!(retrieved.value["metadata"]["confidence_score"], 0.95);

    teardown_test_db(pool).await;
}

/// Test soft-delete behavior
#[tokio::test]
async fn test_soft_delete_behavior() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let memory = Memory::new(
        namespace.clone(),
        "to_be_deleted".to_string(),
        json!({"test": "data"}),
        MemoryType::Episodic,
        "test".to_string(),
    );

    // Add memory
    service.add(memory).await.expect("Failed to add memory");

    // Verify it exists
    let before_delete = service
        .get(&namespace, "to_be_deleted")
        .await
        .expect("Failed to get memory");
    assert!(before_delete.is_some());
    assert!(before_delete.unwrap().is_active());

    // Act: Soft delete
    service
        .delete(&namespace, "to_be_deleted")
        .await
        .expect("Failed to delete memory");

    // Assert: Memory should not be retrievable after soft delete
    let after_delete = service
        .get(&namespace, "to_be_deleted")
        .await
        .expect("Failed to get memory");
    assert!(
        after_delete.is_none(),
        "Soft-deleted memory should not be retrievable"
    );

    // Verify it doesn't appear in search results
    let search_results = service
        .search(&namespace, None, None)
        .await
        .expect("Failed to search");
    assert_eq!(
        search_results.len(),
        0,
        "Soft-deleted memory should not appear in search"
    );

    teardown_test_db(pool).await;
}

/// Test created_by and updated_by field tracking
#[tokio::test]
async fn test_created_by_and_updated_by_tracking() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let memory = Memory::new(
        namespace.clone(),
        "requirements".to_string(),
        json!({"version": 1}),
        MemoryType::Episodic,
        "requirements-gatherer".to_string(),
    );

    // Act: Add memory
    service.add(memory).await.expect("Failed to add memory");

    // Assert: Verify created_by is set correctly
    let retrieved = service
        .get(&namespace, "requirements")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    assert_eq!(retrieved.created_by, "requirements-gatherer");
    assert_eq!(
        retrieved.updated_by, "requirements-gatherer",
        "updated_by should match created_by initially"
    );

    // Act: Update memory with different user
    service
        .update(
            &namespace,
            "requirements",
            json!({"version": 2}),
            "technical-architect",
        )
        .await
        .expect("Failed to update memory");

    // Assert: Verify updated_by changes but created_by stays the same
    let after_update = service
        .get(&namespace, "requirements")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    assert_eq!(
        after_update.created_by, "requirements-gatherer",
        "created_by should not change"
    );
    assert_eq!(
        after_update.updated_by, "technical-architect",
        "updated_by should reflect the updater"
    );
    assert_eq!(after_update.value, json!({"version": 2}));

    teardown_test_db(pool).await;
}

/// Test multiple namespace patterns for different task stages
#[tokio::test]
async fn test_multiple_namespace_patterns_for_task_stages() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();

    // Add memories for different stages
    let stages = vec![
        (
            format!("task:{}:requirements", task_id),
            "gathered_requirements",
            json!({"problem": "Authentication needed"}),
            "requirements-gatherer",
        ),
        (
            format!("task:{}:technical_specs", task_id),
            "architecture",
            json!({"pattern": "Hexagonal Architecture"}),
            "technical-architect",
        ),
        (
            format!("task:{}:results", task_id),
            "implementation_results",
            json!({"status": "completed", "tests_passed": true}),
            "rust-implementation-specialist",
        ),
    ];

    for (namespace, key, value, created_by) in stages.iter() {
        let memory = Memory::new(
            namespace.clone(),
            key.to_string(),
            value.clone(),
            MemoryType::Episodic,
            created_by.to_string(),
        );
        service
            .add(memory)
            .await
            .expect("Failed to add memory for stage");
    }

    // Act: Search for all task-related memories
    let all_task_memories = service
        .search(&format!("task:{}", task_id), None, None)
        .await
        .expect("Failed to search task memories");

    // Assert: All three stages should be present
    assert_eq!(
        all_task_memories.len(),
        3,
        "Should have memories for all three stages"
    );

    // Verify each namespace is distinct
    let namespaces: Vec<String> = all_task_memories
        .iter()
        .map(|m| m.namespace.clone())
        .collect();
    assert!(namespaces.contains(&format!("task:{}:requirements", task_id)));
    assert!(namespaces.contains(&format!("task:{}:technical_specs", task_id)));
    assert!(namespaces.contains(&format!("task:{}:results", task_id)));

    // Verify we can retrieve each stage specifically
    for (namespace, key, expected_value, _) in stages.iter() {
        let retrieved = service
            .get(namespace, key)
            .await
            .expect("Failed to retrieve stage memory")
            .expect("Stage memory should exist");
        assert_eq!(&retrieved.value, expected_value);
    }

    teardown_test_db(pool).await;
}

/// Test namespace search filtering
#[tokio::test]
async fn test_namespace_search_filtering() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task1_id = Uuid::new_v4().to_string();
    let task2_id = Uuid::new_v4().to_string();

    // Add memories for two different tasks
    let memories = vec![
        (
            format!("task:{}:requirements", task1_id),
            "req1",
            json!({"task": "task1"}),
        ),
        (
            format!("task:{}:requirements", task1_id),
            "req2",
            json!({"task": "task1"}),
        ),
        (
            format!("task:{}:requirements", task2_id),
            "req1",
            json!({"task": "task2"}),
        ),
        (
            format!("task:{}:technical_specs", task1_id),
            "arch",
            json!({"task": "task1"}),
        ),
    ];

    for (namespace, key, value) in memories.iter() {
        let memory = Memory::new(
            namespace.clone(),
            key.to_string(),
            value.clone(),
            MemoryType::Episodic,
            "test".to_string(),
        );
        service.add(memory).await.expect("Failed to add memory");
    }

    // Act: Search for task1 requirements only
    let task1_requirements = service
        .search(&format!("task:{}:requirements", task1_id), None, None)
        .await
        .expect("Failed to search");

    // Assert: Should only get task1 requirements
    assert_eq!(task1_requirements.len(), 2);
    for mem in task1_requirements {
        assert!(mem.namespace.starts_with(&format!("task:{}", task1_id)));
        assert!(mem.namespace.ends_with(":requirements"));
    }

    // Act: Search for all task1 memories
    let all_task1 = service
        .search(&format!("task:{}", task1_id), None, None)
        .await
        .expect("Failed to search");

    // Assert: Should get both requirements and technical_specs
    assert_eq!(all_task1.len(), 3);

    // Act: Search for task2 requirements
    let task2_requirements = service
        .search(&format!("task:{}:requirements", task2_id), None, None)
        .await
        .expect("Failed to search");

    // Assert: Should only get task2 requirement
    assert_eq!(task2_requirements.len(), 1);
    assert_eq!(task2_requirements[0].namespace, format!("task:{}:requirements", task2_id));

    teardown_test_db(pool).await;
}

/// Test updating memory preserves namespace
#[tokio::test]
async fn test_update_preserves_namespace() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let memory = Memory::new(
        namespace.clone(),
        "requirements".to_string(),
        json!({"version": 1, "data": "initial"}),
        MemoryType::Episodic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Update memory multiple times
    for version in 2..=5 {
        service
            .update(
                &namespace,
                "requirements",
                json!({"version": version, "data": format!("updated_{}", version)}),
                "technical-architect",
            )
            .await
            .expect("Failed to update memory");
    }

    // Assert: Namespace should remain unchanged
    let final_memory = service
        .get(&namespace, "requirements")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    assert_eq!(final_memory.namespace, namespace);
    assert_eq!(final_memory.value["version"], 5);
    assert_eq!(final_memory.value["data"], "updated_5");

    teardown_test_db(pool).await;
}

/// Test that attempting to update a deleted memory fails
#[tokio::test]
async fn test_update_deleted_memory_fails() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let memory = Memory::new(
        namespace.clone(),
        "requirements".to_string(),
        json!({"data": "original"}),
        MemoryType::Episodic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Delete the memory
    service
        .delete(&namespace, "requirements")
        .await
        .expect("Failed to delete memory");

    // Act: Try to update deleted memory
    let result = service
        .update(&namespace, "requirements", json!({"data": "updated"}), "test")
        .await;

    // Assert: Update should fail
    assert!(
        result.is_err(),
        "Updating deleted memory should fail"
    );
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("not found") || error_msg.contains("deleted"),
        "Error message should indicate memory is not found or deleted"
    );

    teardown_test_db(pool).await;
}
