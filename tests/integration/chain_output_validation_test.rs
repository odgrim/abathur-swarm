//! Integration tests for validating chain output against OutputValidator
//!
//! Tests the complete workflow of JSON output validation for prompt chain results,
//! specifically focusing on the requirements-gatherer and technical-architect
//! chain steps that produce structured JSON output.

use abathur_cli::domain::models::prompt_chain::OutputFormat;
use abathur_cli::infrastructure::validators::OutputValidator;
use serde_json::json;

/// Test that valid architecture JSON output passes validation
#[test]
fn test_valid_architecture_output_passes_validation() {
    let validator = OutputValidator::new();

    // Valid architecture output matching the schema from technical_feature_workflow.yaml
    let valid_output = json!({
        "feature_name": "user-authentication",
        "architecture_overview": "JWT-based authentication system with refresh tokens",
        "components": [
            {
                "name": "auth_service",
                "responsibility": "Handle authentication and token generation",
                "interfaces": ["POST /api/auth/login", "POST /api/auth/refresh"],
                "dependencies": ["user_repository", "token_service"]
            },
            {
                "name": "token_service",
                "responsibility": "Generate and validate JWT tokens",
                "interfaces": ["generate_token", "validate_token"],
                "dependencies": ["jwt_library"]
            }
        ],
        "technology_stack": [
            {
                "layer": "backend",
                "technology": "Rust with Actix-web",
                "justification": "High performance async web framework with strong typing"
            },
            {
                "layer": "database",
                "technology": "PostgreSQL",
                "justification": "ACID compliance required for user data"
            }
        ],
        "decomposition": {
            "strategy": "single",
            "subprojects": [],
            "rationale": "Small enough to implement as single feature"
        }
    });

    let schema = json!({
        "type": "object",
        "properties": {
            "feature_name": {
                "type": "string"
            },
            "architecture_overview": {
                "type": "string"
            },
            "components": {
                "type": "array"
            },
            "technology_stack": {
                "type": "array"
            },
            "decomposition": {
                "type": "object",
                "properties": {
                    "strategy": {
                        "type": "string",
                        "enum": ["single", "multiple"]
                    }
                },
                "required": ["strategy"]
            }
        },
        "required": [
            "feature_name",
            "architecture_overview",
            "components",
            "decomposition"
        ]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(&valid_output.to_string(), &output_format);
    assert!(
        result.is_ok(),
        "Valid architecture output should pass validation: {:?}",
        result.err()
    );
}

/// Test that architecture output missing required fields fails validation
#[test]
fn test_missing_required_fields_fails_validation() {
    let validator = OutputValidator::new();

    // Missing 'feature_name' and 'decomposition' required fields
    let invalid_output = json!({
        "architecture_overview": "Some overview",
        "components": [
            {
                "name": "test_component",
                "responsibility": "test"
            }
        ]
    });

    let schema = json!({
        "type": "object",
        "properties": {
            "feature_name": {
                "type": "string"
            },
            "architecture_overview": {
                "type": "string"
            },
            "components": {
                "type": "array"
            },
            "decomposition": {
                "type": "object"
            }
        },
        "required": [
            "feature_name",
            "architecture_overview",
            "components",
            "decomposition"
        ]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(&invalid_output.to_string(), &output_format);
    assert!(
        result.is_err(),
        "Output missing required fields should fail validation"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("feature_name") || error_message.contains("decomposition"),
        "Error should mention missing required field(s)"
    );
}

/// Test that invalid JSON format fails validation
#[test]
fn test_invalid_json_format_fails_validation() {
    let validator = OutputValidator::new();

    // Malformed JSON - missing closing brace
    let invalid_json = r#"{"feature_name": "test", "components": ["#;

    let schema = json!({
        "type": "object",
        "properties": {
            "feature_name": {"type": "string"}
        }
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(invalid_json, &output_format);
    assert!(
        result.is_err(),
        "Invalid JSON format should fail validation"
    );

    let error_message = result.unwrap_err().to_string();
    assert!(
        error_message.contains("JSON") || error_message.contains("parse"),
        "Error should indicate JSON parsing failure"
    );
}

/// Test that schema validation catches type errors
#[test]
fn test_schema_validation_catches_type_errors() {
    let validator = OutputValidator::new();

    // 'feature_name' should be string but is number
    // 'components' should be array but is string
    let output_with_type_errors = json!({
        "feature_name": 12345,  // Wrong type: number instead of string
        "architecture_overview": "Overview text",
        "components": "not an array",  // Wrong type: string instead of array
        "decomposition": {
            "strategy": "single"
        }
    });

    let schema = json!({
        "type": "object",
        "properties": {
            "feature_name": {
                "type": "string"
            },
            "architecture_overview": {
                "type": "string"
            },
            "components": {
                "type": "array"
            },
            "decomposition": {
                "type": "object"
            }
        },
        "required": ["feature_name", "components"]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(&output_with_type_errors.to_string(), &output_format);
    assert!(
        result.is_err(),
        "Schema validation should catch type errors"
    );

    let error_message = result.unwrap_err().to_string();
    // Should mention at least one of the type mismatches
    assert!(
        error_message.contains("feature_name") || error_message.contains("components"),
        "Error should mention fields with type errors"
    );
}

/// Test DecompositionStrategy enum value validation
#[test]
fn test_decomposition_strategy_enum_validation() {
    let validator = OutputValidator::new();

    // Test valid enum values: "single" and "multiple"
    for valid_strategy in &["single", "multiple"] {
        let valid_output = json!({
            "feature_name": "test-feature",
            "architecture_overview": "Test architecture",
            "components": [],
            "decomposition": {
                "strategy": valid_strategy
            }
        });

        let schema = json!({
            "type": "object",
            "properties": {
                "feature_name": {"type": "string"},
                "architecture_overview": {"type": "string"},
                "components": {"type": "array"},
                "decomposition": {
                    "type": "object",
                    "properties": {
                        "strategy": {
                            "type": "string",
                            "enum": ["single", "multiple"]
                        }
                    },
                    "required": ["strategy"]
                }
            },
            "required": ["feature_name", "architecture_overview", "components", "decomposition"]
        });

        let output_format = OutputFormat::Json {
            schema: Some(schema),
        };

        let result = validator.validate(&valid_output.to_string(), &output_format);
        assert!(
            result.is_ok(),
            "DecompositionStrategy '{}' should be valid: {:?}",
            valid_strategy,
            result.err()
        );
    }

    // Test invalid enum value
    let invalid_output = json!({
        "feature_name": "test-feature",
        "architecture_overview": "Test architecture",
        "components": [],
        "decomposition": {
            "strategy": "invalid_strategy"  // Not in enum
        }
    });

    let schema = json!({
        "type": "object",
        "properties": {
            "feature_name": {"type": "string"},
            "architecture_overview": {"type": "string"},
            "components": {"type": "array"},
            "decomposition": {
                "type": "object",
                "properties": {
                    "strategy": {
                        "type": "string",
                        "enum": ["single", "multiple"]
                    }
                },
                "required": ["strategy"]
            }
        },
        "required": ["feature_name", "architecture_overview", "components", "decomposition"]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(&invalid_output.to_string(), &output_format);
    assert!(
        result.is_err(),
        "Invalid DecompositionStrategy should fail validation"
    );
}

/// Test that JSON wrapped in markdown code blocks is properly validated
#[test]
fn test_json_wrapped_in_markdown_code_blocks() {
    let validator = OutputValidator::new();

    // Simulate LLM output with markdown code block (common behavior)
    let output_with_markdown = r#"```json
{
    "feature_name": "payment-integration",
    "architecture_overview": "Stripe payment integration with webhook handling",
    "components": [
        {
            "name": "payment_service",
            "responsibility": "Process payments via Stripe API"
        }
    ],
    "decomposition": {
        "strategy": "single"
    }
}
```"#;

    let schema = json!({
        "type": "object",
        "properties": {
            "feature_name": {"type": "string"},
            "architecture_overview": {"type": "string"},
            "components": {"type": "array"},
            "decomposition": {
                "type": "object",
                "properties": {
                    "strategy": {
                        "type": "string",
                        "enum": ["single", "multiple"]
                    }
                },
                "required": ["strategy"]
            }
        },
        "required": ["feature_name", "architecture_overview", "components", "decomposition"]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    // Should pass validation because OutputValidator strips markdown blocks
    let result = validator.validate(output_with_markdown, &output_format);
    assert!(
        result.is_ok(),
        "JSON wrapped in markdown code blocks should be validated successfully after stripping: {:?}",
        result.err()
    );
}

/// Test requirements output validation (first step of chain)
#[test]
fn test_valid_requirements_output_passes_validation() {
    let validator = OutputValidator::new();

    let valid_requirements = json!({
        "problem_statement": "Users need to securely authenticate to access protected resources",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Users can log in with email and password",
                "priority": "must"
            },
            {
                "id": "FR-2",
                "description": "System generates JWT tokens upon successful login",
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
                "target": "All passwords stored with bcrypt cost factor >= 12"
            },
            {
                "id": "NFR-2",
                "category": "performance",
                "description": "Login endpoint response time",
                "target": "< 200ms at p95"
            }
        ],
        "constraints": [
            "Must integrate with existing PostgreSQL database",
            "Cannot modify existing user table schema"
        ],
        "success_criteria": [
            "Users can log in and receive valid JWT tokens",
            "Token refresh works without re-authentication",
            "All security tests pass"
        ],
        "dependencies": [
            "user_repository",
            "database_connection_pool"
        ]
    });

    let schema = json!({
        "type": "object",
        "properties": {
            "problem_statement": {"type": "string"},
            "functional_requirements": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "description": {"type": "string"},
                        "priority": {
                            "type": "string",
                            "enum": ["must", "should", "could"]
                        }
                    },
                    "required": ["id", "description", "priority"]
                }
            },
            "non_functional_requirements": {"type": "array"},
            "constraints": {"type": "array"},
            "success_criteria": {"type": "array"},
            "dependencies": {"type": "array"}
        },
        "required": ["problem_statement", "functional_requirements", "success_criteria"]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(&valid_requirements.to_string(), &output_format);
    assert!(
        result.is_ok(),
        "Valid requirements output should pass validation: {:?}",
        result.err()
    );
}

/// Test that requirements with invalid priority enum values fail validation
#[test]
fn test_requirements_invalid_priority_fails_validation() {
    let validator = OutputValidator::new();

    let invalid_requirements = json!({
        "problem_statement": "Test problem",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Test requirement",
                "priority": "high"  // Invalid: should be "must", "should", or "could"
            }
        ],
        "success_criteria": ["Test criteria"]
    });

    let schema = json!({
        "type": "object",
        "properties": {
            "problem_statement": {"type": "string"},
            "functional_requirements": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "description": {"type": "string"},
                        "priority": {
                            "type": "string",
                            "enum": ["must", "should", "could"]
                        }
                    },
                    "required": ["id", "description", "priority"]
                }
            },
            "success_criteria": {"type": "array"}
        },
        "required": ["problem_statement", "functional_requirements", "success_criteria"]
    });

    let output_format = OutputFormat::Json {
        schema: Some(schema),
    };

    let result = validator.validate(&invalid_requirements.to_string(), &output_format);
    assert!(
        result.is_err(),
        "Requirements with invalid priority enum should fail validation"
    );
}

/// Test validation without schema (just JSON parsing)
#[test]
fn test_validation_without_schema_only_checks_json_validity() {
    let validator = OutputValidator::new();

    // Valid JSON but doesn't match any particular schema
    let valid_json = json!({
        "random_field": "value",
        "another_field": 123,
        "nested": {
            "data": true
        }
    });

    let output_format = OutputFormat::Json { schema: None };

    let result = validator.validate(&valid_json.to_string(), &output_format);
    assert!(
        result.is_ok(),
        "Valid JSON should pass validation without schema"
    );

    // Invalid JSON
    let invalid_json = r#"{"unclosed": "object""#;

    let result = validator.validate(invalid_json, &output_format);
    assert!(
        result.is_err(),
        "Invalid JSON should fail validation even without schema"
    );
}

/// Test markdown code block stripping edge cases
#[test]
fn test_markdown_code_block_stripping_edge_cases() {
    let validator = OutputValidator::new();

    // Test 1: Code block with language identifier
    let with_language = r#"```json
{"test": "value"}
```"#;

    let result = validator.validate(with_language, &OutputFormat::Json { schema: None });
    assert!(result.is_ok(), "Should strip code block with language identifier");

    // Test 2: Code block without language identifier
    let without_language = r#"```
{"test": "value"}
```"#;

    let result = validator.validate(without_language, &OutputFormat::Json { schema: None });
    assert!(result.is_ok(), "Should strip code block without language identifier");

    // Test 3: Text before code block
    let text_before = r#"Here is the result:
```json
{"test": "value"}
```"#;

    let result = validator.validate(text_before, &OutputFormat::Json { schema: None });
    assert!(result.is_ok(), "Should find and strip code block even with text before");

    // Test 4: Multiple lines in JSON
    let multiline_json = r#"```json
{
    "test": "value",
    "nested": {
        "field": 123
    }
}
```"#;

    let result = validator.validate(multiline_json, &OutputFormat::Json { schema: None });
    assert!(result.is_ok(), "Should handle multiline JSON in code blocks");

    // Test 5: No code block - just JSON
    let just_json = r#"{"test": "value"}"#;

    let result = validator.validate(just_json, &OutputFormat::Json { schema: None });
    assert!(result.is_ok(), "Should handle JSON without code blocks");
}

/// Integration test: Complete requirements-gatherer -> technical-architect flow
#[test]
fn test_complete_chain_output_validation_flow() {
    let validator = OutputValidator::new();

    // Step 1: Requirements output
    let requirements_output = json!({
        "problem_statement": "Need real-time notifications for user events",
        "functional_requirements": [
            {
                "id": "FR-1",
                "description": "Send push notifications on new messages",
                "priority": "must"
            }
        ],
        "success_criteria": [
            "Notifications delivered within 5 seconds",
            "99.9% delivery success rate"
        ]
    });

    let requirements_schema = json!({
        "type": "object",
        "required": ["problem_statement", "functional_requirements", "success_criteria"]
    });

    let req_result = validator.validate(
        &requirements_output.to_string(),
        &OutputFormat::Json {
            schema: Some(requirements_schema),
        },
    );
    assert!(req_result.is_ok(), "Requirements validation should pass");

    // Step 2: Architecture output (depends on requirements)
    let architecture_output = json!({
        "feature_name": "real-time-notifications",
        "architecture_overview": "WebSocket-based push notification system",
        "components": [
            {
                "name": "notification_service",
                "responsibility": "Manage notification delivery"
            }
        ],
        "decomposition": {
            "strategy": "single"
        }
    });

    let architecture_schema = json!({
        "type": "object",
        "required": ["feature_name", "architecture_overview", "components", "decomposition"]
    });

    let arch_result = validator.validate(
        &architecture_output.to_string(),
        &OutputFormat::Json {
            schema: Some(architecture_schema),
        },
    );
    assert!(arch_result.is_ok(), "Architecture validation should pass");
}
