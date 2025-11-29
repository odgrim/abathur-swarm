// Integration tests for logging functionality
// Note: These tests must be run separately as they initialize global state
// Run with: cargo test --test logging_integration_test -- --test-threads=1

use abathur_cli::logging::{info, instrument, LogConfig, LogFormat, LoggerImpl, RotationPolicy};
use std::fs;
use tempfile::TempDir;

/// Main integration test that covers multiple scenarios
#[test]
fn test_logging_comprehensive() {
    let temp_dir = TempDir::new().unwrap();

    // Initialize logger with file output
    let config = LogConfig {
        level: "info".to_string(),
        format: LogFormat::Json,
        log_dir: Some(temp_dir.path().to_path_buf()),
        enable_stdout: false,
        rotation: RotationPolicy::Daily,
        retention_days: 30,
    };

    let _logger = LoggerImpl::init(&config).unwrap();

    // Test basic logging
    info!("Test message 1");
    info!(key = "value", "Test message with fields");

    // Test instrumented function
    let result = instrumented_add(5, 7);
    assert_eq!(result, 12);

    // Test async instrumentation
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        async_function("test").await;
    });

    // Give time for async writes
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Verify log file was created
    let log_files: Vec<_> = fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.contains("abathur.log"))
                .unwrap_or(false)
        })
        .collect();

    assert!(!log_files.is_empty(), "Log file should be created");

    // Read and verify log contents
    let log_file = &log_files[0];
    let contents = fs::read_to_string(log_file.path()).unwrap();

    // Verify basic messages
    assert!(
        contents.contains("Test message 1"),
        "Log should contain basic message"
    );
    assert!(
        contents.contains("Test message with fields"),
        "Log should contain message with fields"
    );

    // Verify instrumentation worked
    assert!(
        contents.contains("instrumented") || contents.contains("entering instrumented function"),
        "Log should contain instrumented function traces"
    );
}

#[instrument]
fn instrumented_add(a: i32, b: i32) -> i32 {
    info!("entering instrumented function");
    a + b
}

#[instrument]
async fn async_function(param: &str) {
    info!(param, "async function called");
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
}
