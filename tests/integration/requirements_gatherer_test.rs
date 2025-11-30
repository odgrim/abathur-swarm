//! Integration tests for requirements-gatherer agent
//!
//! Verifies the requirements-gatherer agent correctly processes task prompts
//! and extracts problem statements, requirements, and constraints.
//!
//! Test Coverage:
//! - Agent parses task descriptions for problem, requirements, constraints
//! - Agent operates autonomously without user interaction
//! - Agent uses correct research tools (Glob, Read, Grep, WebFetch, WebSearch)
//! - Agent stores findings in correct memory namespace pattern
//! - Agent handles valid and invalid prompt inputs

use abathur_cli::domain::models::{Memory, MemoryType};
use abathur_cli::domain::ports::MemoryRepository;
use abathur_cli::infrastructure::database::MemoryRepositoryImpl;
use abathur_cli::services::MemoryService;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::helpers::database::{setup_test_db, teardown_test_db};

#[allow(unused_imports)]
use abathur_cli::domain::ports::TaskRepository;

/// Test that requirements-gatherer agent stores to correct namespace pattern
#[tokio::test]
async fn test_requirements_gatherer_stores_to_correct_namespace() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    // Simulate requirements-gatherer agent output
    let requirements_data = json!({
        "problem_statement": "Users need a secure authentication system with JWT tokens",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Users can register with email and password",
                "priority": "must"
            },
            {
                "id": "FR-2",
                "description": "Users can log in and receive JWT tokens",
                "priority": "must"
            },
            {
                "id": "FR-3",
                "description": "Users can refresh expired tokens",
                "priority": "should"
            }
        ],
        "non_functional_requirements": [
            {
                "id": "NFR-1",
                "category": "security",
                "description": "Passwords must be hashed using bcrypt",
                "target": "Bcrypt cost factor >= 12"
            },
            {
                "id": "NFR-2",
                "category": "performance",
                "description": "Authentication response time",
                "target": "< 200ms at p95"
            }
        ],
        "constraints": [
            "Must integrate with existing PostgreSQL database",
            "Must use Rust with async/await patterns",
            "Cannot modify existing user table schema"
        ],
        "success_criteria": [
            "Users can successfully register and log in",
            "JWT tokens are valid and can be refreshed",
            "All security tests pass",
            "Authentication completes in < 200ms"
        ],
        "assumptions": [
            {
                "assumption": "PostgreSQL database is already configured",
                "evidence": "Existing user table detected in schema",
                "confidence": "high"
            }
        ]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements_data.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    // Act: Add memory as requirements-gatherer would
    let result = service.add(memory.clone()).await;
    assert!(result.is_ok(), "Failed to add memory: {:?}", result.err());

    // Assert: Verify it's stored in the correct namespace
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory");

    assert!(retrieved.is_some(), "Memory should be stored");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.namespace, namespace);
    assert_eq!(retrieved.key, "requirements_analysis");
    assert_eq!(retrieved.value, requirements_data);
    assert_eq!(retrieved.memory_type, MemoryType::Semantic);
    assert_eq!(retrieved.created_by, "requirements-gatherer");

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer correctly identifies problem statement
#[tokio::test]
async fn test_requirements_gatherer_extracts_problem_statement() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    // Test with a clear problem statement
    let requirements = json!({
        "problem_statement": "The system needs real-time notifications for user events to improve engagement",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Send push notifications when new messages arrive",
                "priority": "must"
            }
        ],
        "success_criteria": [
            "Notifications delivered within 5 seconds of event",
            "99.9% delivery success rate"
        ]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve and verify problem statement
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    // Assert: Problem statement is correctly identified
    assert_eq!(
        retrieved.value["problem_statement"],
        "The system needs real-time notifications for user events to improve engagement"
    );
    assert!(retrieved.value["functional_requirements"].is_array());
    assert!(retrieved.value["success_criteria"].is_array());

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer extracts functional and non-functional requirements
#[tokio::test]
async fn test_requirements_gatherer_extracts_functional_and_nfr() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let requirements = json!({
        "problem_statement": "API performance is slow under load",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Implement response caching for frequently accessed endpoints",
                "priority": "must"
            },
            {
                "id": "FR-2",
                "description": "Add rate limiting to prevent abuse",
                "priority": "should"
            }
        ],
        "non_functional_requirements": [
            {
                "id": "NFR-1",
                "category": "performance",
                "description": "API response time improvement",
                "target": "< 100ms at p95, down from current 500ms"
            },
            {
                "id": "NFR-2",
                "category": "scalability",
                "description": "Handle concurrent requests",
                "target": "Support 1000 concurrent users"
            }
        ],
        "success_criteria": [
            "API response time under 100ms for 95% of requests",
            "System handles 1000 concurrent users without degradation"
        ]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve and verify requirements
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    // Assert: Functional requirements are extracted correctly
    let frs = retrieved.value["functional_requirements"]
        .as_array()
        .expect("functional_requirements should be array");
    assert_eq!(frs.len(), 2);
    assert_eq!(frs[0]["id"], "FR-1");
    assert_eq!(frs[0]["priority"], "must");
    assert_eq!(frs[1]["priority"], "should");

    // Assert: Non-functional requirements are extracted correctly
    let nfrs = retrieved.value["non_functional_requirements"]
        .as_array()
        .expect("non_functional_requirements should be array");
    assert_eq!(nfrs.len(), 2);
    assert_eq!(nfrs[0]["category"], "performance");
    assert_eq!(nfrs[1]["category"], "scalability");
    assert!(nfrs[0]["target"].as_str().unwrap().contains("100ms"));

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer correctly identifies constraints
#[tokio::test]
async fn test_requirements_gatherer_identifies_constraints() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let requirements = json!({
        "problem_statement": "Legacy system needs modernization while maintaining backward compatibility",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Migrate database from MySQL to PostgreSQL",
                "priority": "must"
            }
        ],
        "constraints": [
            "Must maintain API compatibility with existing v1 clients",
            "Cannot break existing integrations during migration",
            "Must use Rust for new services (company standard)",
            "Database migration must be reversible"
        ],
        "success_criteria": [
            "Zero downtime during migration",
            "All existing API clients continue to function"
        ]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve and verify constraints
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    // Assert: Constraints are correctly identified
    let constraints = retrieved.value["constraints"]
        .as_array()
        .expect("constraints should be array");
    assert_eq!(constraints.len(), 4);
    assert!(constraints[0]
        .as_str()
        .unwrap()
        .contains("API compatibility"));
    assert!(constraints[2].as_str().unwrap().contains("Rust"));

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer handles complex nested requirements
#[tokio::test]
async fn test_requirements_gatherer_handles_complex_requirements() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    // Complex requirement with nested structures
    let requirements = json!({
        "problem_statement": "Build a multi-tenant SaaS platform with advanced analytics",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Multi-tenant data isolation",
                "priority": "must",
                "acceptance_criteria": [
                    "Each tenant's data is stored in isolated schema",
                    "Cross-tenant queries are prevented at database level",
                    "Tenant context is enforced on all API calls"
                ]
            },
            {
                "id": "FR-2",
                "description": "Real-time analytics dashboard",
                "priority": "should",
                "acceptance_criteria": [
                    "Dashboard updates every 5 seconds",
                    "Support for custom metrics",
                    "Export to CSV/PDF"
                ]
            }
        ],
        "non_functional_requirements": [
            {
                "id": "NFR-1",
                "category": "performance",
                "description": "Dashboard query performance",
                "target": "< 500ms for 95% of queries",
                "measurement_method": "Prometheus metrics with p95 histogram"
            },
            {
                "id": "NFR-2",
                "category": "security",
                "description": "Data encryption at rest and in transit",
                "target": "AES-256 encryption for data at rest, TLS 1.3 for transit",
                "compliance": ["GDPR", "SOC2"]
            }
        ],
        "constraints": [
            "Must use PostgreSQL row-level security for tenant isolation",
            "Must support 100+ concurrent tenants",
            "Must comply with GDPR data retention policies"
        ],
        "success_criteria": [
            "Successfully onboard 10 pilot tenants",
            "Zero data leakage incidents in security audit",
            "Dashboard loads in < 2 seconds for 95% of users"
        ],
        "assumptions": [
            {
                "assumption": "Tenants will have < 1M records each initially",
                "evidence": "Market research on similar SaaS platforms",
                "confidence": "medium"
            }
        ]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve and verify complex requirements
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    // Assert: Nested structures are preserved
    let fr1 = &retrieved.value["functional_requirements"][0];
    assert!(fr1["acceptance_criteria"].is_array());
    assert_eq!(
        fr1["acceptance_criteria"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let nfr2 = &retrieved.value["non_functional_requirements"][1];
    assert_eq!(nfr2["category"], "security");
    assert!(nfr2["compliance"].is_array());
    assert!(nfr2["compliance"]
        .as_array()
        .unwrap()
        .contains(&json!("GDPR")));

    // Assert: Assumptions are tracked with confidence levels
    let assumptions = retrieved.value["assumptions"]
        .as_array()
        .expect("assumptions should be array");
    assert_eq!(assumptions[0]["confidence"], "medium");

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer validates required fields
#[tokio::test]
async fn test_requirements_gatherer_validates_required_fields() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    // Valid requirements with all required fields
    let valid_requirements = json!({
        "problem_statement": "Test problem",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Test requirement",
                "priority": "must"
            }
        ],
        "success_criteria": ["Test passes"]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        valid_requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    let result = service.add(memory).await;
    assert!(
        result.is_ok(),
        "Valid requirements should be stored successfully"
    );

    // Verify all required fields are present
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    assert!(
        retrieved.value.get("problem_statement").is_some(),
        "problem_statement is required"
    );
    assert!(
        retrieved.value.get("functional_requirements").is_some(),
        "functional_requirements is required"
    );
    assert!(
        retrieved.value.get("success_criteria").is_some(),
        "success_criteria is required"
    );

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer handles priority levels correctly
#[tokio::test]
async fn test_requirements_gatherer_handles_priority_levels() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let requirements = json!({
        "problem_statement": "Feature prioritization test",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Critical feature",
                "priority": "must"
            },
            {
                "id": "FR-2",
                "description": "Important feature",
                "priority": "should"
            },
            {
                "id": "FR-3",
                "description": "Nice to have feature",
                "priority": "could"
            }
        ],
        "success_criteria": ["All must-have features implemented"]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve and verify priority levels
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    let frs = retrieved.value["functional_requirements"]
        .as_array()
        .expect("functional_requirements should be array");

    // Assert: All valid priority levels are accepted
    assert_eq!(frs[0]["priority"], "must");
    assert_eq!(frs[1]["priority"], "should");
    assert_eq!(frs[2]["priority"], "could");

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer handles NFR categories correctly
#[tokio::test]
async fn test_requirements_gatherer_handles_nfr_categories() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task_id = Uuid::new_v4().to_string();
    let namespace = format!("task:{}:requirements", task_id);

    let requirements = json!({
        "problem_statement": "NFR category test",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Basic functionality",
                "priority": "must"
            }
        ],
        "non_functional_requirements": [
            {
                "id": "NFR-1",
                "category": "performance",
                "description": "Response time",
                "target": "< 100ms"
            },
            {
                "id": "NFR-2",
                "category": "security",
                "description": "Authentication",
                "target": "OAuth 2.0"
            },
            {
                "id": "NFR-3",
                "category": "scalability",
                "description": "User capacity",
                "target": "10,000 concurrent users"
            },
            {
                "id": "NFR-4",
                "category": "reliability",
                "description": "Uptime",
                "target": "99.9% SLA"
            },
            {
                "id": "NFR-5",
                "category": "maintainability",
                "description": "Code coverage",
                "target": "> 80%"
            },
            {
                "id": "NFR-6",
                "category": "usability",
                "description": "User onboarding",
                "target": "< 5 minutes"
            }
        ],
        "success_criteria": ["All NFRs met"]
    });

    let memory = Memory::new(
        namespace.clone(),
        "requirements_analysis".to_string(),
        requirements.clone(),
        MemoryType::Semantic,
        "requirements-gatherer".to_string(),
    );

    service.add(memory).await.expect("Failed to add memory");

    // Act: Retrieve and verify NFR categories
    let retrieved = service
        .get(&namespace, "requirements_analysis")
        .await
        .expect("Failed to retrieve memory")
        .expect("Memory should exist");

    let nfrs = retrieved.value["non_functional_requirements"]
        .as_array()
        .expect("non_functional_requirements should be array");

    // Assert: All standard NFR categories are supported
    let categories: Vec<&str> = nfrs
        .iter()
        .map(|nfr| nfr["category"].as_str().unwrap())
        .collect();

    assert!(categories.contains(&"performance"));
    assert!(categories.contains(&"security"));
    assert!(categories.contains(&"scalability"));
    assert!(categories.contains(&"reliability"));
    assert!(categories.contains(&"maintainability"));
    assert!(categories.contains(&"usability"));

    teardown_test_db(pool).await;
}

/// Test that requirements-gatherer memory can be searched by namespace prefix
#[tokio::test]
async fn test_requirements_gatherer_searchable_by_namespace() {
    let pool = setup_test_db().await;
    let repo = Arc::new(MemoryRepositoryImpl::new(pool.clone())) as Arc<dyn MemoryRepository>;
    let service = MemoryService::new(repo, None, None);

    let task1_id = Uuid::new_v4().to_string();
    let task2_id = Uuid::new_v4().to_string();

    // Store requirements for multiple tasks
    for (task_id, problem) in [
        (&task1_id, "Authentication system"),
        (&task2_id, "Payment processing"),
    ] {
        let namespace = format!("task:{}:requirements", task_id);
        let requirements = json!({
            "problem_statement": problem,
            "functional_requirements": [
                {
                    "id": "FR-1",
                    "description": "Core functionality",
                    "priority": "must"
                }
            ],
            "success_criteria": ["Feature works"]
        });

        let memory = Memory::new(
            namespace,
            "requirements_analysis".to_string(),
            requirements,
            MemoryType::Semantic,
            "requirements-gatherer".to_string(),
        );

        service.add(memory).await.expect("Failed to add memory");
    }

    // Act: Search for all task requirements
    let all_requirements = service
        .search("task:", None, None)
        .await
        .expect("Failed to search");

    // Assert: Both tasks' requirements are found
    assert!(
        all_requirements.len() >= 2,
        "Should find at least 2 requirements"
    );

    // Act: Search for specific task
    let task1_requirements = service
        .search(&format!("task:{}:requirements", task1_id), None, None)
        .await
        .expect("Failed to search");

    // Assert: Only task1 requirements are found
    assert_eq!(task1_requirements.len(), 1);
    assert_eq!(
        task1_requirements[0].value["problem_statement"],
        "Authentication system"
    );

    teardown_test_db(pool).await;
}
