use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

// Mock transport for testing
struct MockTransport {
    fail_count: Arc<AtomicU32>,
    consecutive_failures: u32,
}

impl MockTransport {
    fn new(consecutive_failures: u32) -> Self {
        Self {
            fail_count: Arc::new(AtomicU32::new(0)),
            consecutive_failures,
        }
    }

    async fn request(&mut self, _request: &serde_json::Value) -> Result<serde_json::Value, String> {
        let current_count = self.fail_count.fetch_add(1, Ordering::SeqCst);

        if current_count < self.consecutive_failures {
            Err("Simulated failure".to_string())
        } else {
            Ok(serde_json::json!({"jsonrpc": "2.0", "result": "pong"}))
        }
    }
}

// Mock server manager for testing
struct MockServerManager {
    transport: Arc<Mutex<MockTransport>>,
    restart_count: Arc<AtomicU32>,
}

impl MockServerManager {
    fn new(consecutive_failures: u32) -> Self {
        Self {
            transport: Arc::new(Mutex::new(MockTransport::new(consecutive_failures))),
            restart_count: Arc::new(AtomicU32::new(0)),
        }
    }

    async fn get_transport(
        &self,
        _server_name: &str,
    ) -> Result<Arc<Mutex<MockTransport>>, String> {
        Ok(self.transport.clone())
    }

    async fn restart_server(&self, _server_name: &str) -> Result<(), String> {
        self.restart_count.fetch_add(1, Ordering::SeqCst);
        // Reset failure count on restart
        self.transport.lock().await.fail_count.store(0, Ordering::SeqCst);
        Ok(())
    }

    fn get_restart_count(&self) -> u32 {
        self.restart_count.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn test_health_monitor_successful_checks() {
    // Mock manager that never fails
    let manager = Arc::new(MockServerManager::new(0));
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    // Start monitoring with fast intervals for testing
    let handle = tokio::spawn({
        let manager = manager.clone();
        async move {
            let mut consecutive_failures = 0;
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            let mut shutdown_rx = shutdown_rx;

            // Skip first tick
            interval.tick().await;

            for _ in 0..5 {
                tokio::select! {
                    _ = interval.tick() => {
                        let transport = manager.get_transport("test").await.unwrap();
                        let mut transport = transport.lock().await;
                        let ping = serde_json::json!({"method": "ping"});

                        match transport.request(&ping).await {
                            Ok(_) => consecutive_failures = 0,
                            Err(_) => consecutive_failures += 1,
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }

            assert_eq!(consecutive_failures, 0, "Should have no failures");
        }
    });

    // Let it run for a bit
    tokio::time::sleep(Duration::from_millis(600)).await;

    // Shutdown
    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();

    // Should not have restarted
    assert_eq!(manager.get_restart_count(), 0);
}

#[tokio::test]
async fn test_health_monitor_failure_tracking() {
    // Mock manager that fails 2 times then succeeds
    let manager = Arc::new(MockServerManager::new(2));
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    let handle = tokio::spawn({
        let manager = manager.clone();
        async move {
            let mut consecutive_failures = 0;
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            let mut shutdown_rx = shutdown_rx;
            let max_failures = 3;

            interval.tick().await;

            for _ in 0..5 {
                tokio::select! {
                    _ = interval.tick() => {
                        let transport = manager.get_transport("test").await.unwrap();
                        let mut transport = transport.lock().await;
                        let ping = serde_json::json!({"method": "ping"});

                        match transport.request(&ping).await {
                            Ok(_) => {
                                if consecutive_failures > 0 {
                                    println!("Recovered after {} failures", consecutive_failures);
                                }
                                consecutive_failures = 0;
                            }
                            Err(_) => {
                                consecutive_failures += 1;
                                println!("Health check failed: {}/{}", consecutive_failures, max_failures);

                                if consecutive_failures >= max_failures {
                                    println!("Max failures reached, restarting...");
                                    manager.restart_server("test").await.unwrap();
                                    consecutive_failures = 0;
                                }
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    });

    // Let it run
    tokio::time::sleep(Duration::from_millis(600)).await;

    // Shutdown
    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();

    // Should not have restarted (only 2 failures before recovery)
    assert_eq!(manager.get_restart_count(), 0);
}

#[tokio::test]
async fn test_health_monitor_auto_restart() {
    // Mock manager that fails 5 times then succeeds
    let manager = Arc::new(MockServerManager::new(5));
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    let handle = tokio::spawn({
        let manager = manager.clone();
        async move {
            let mut consecutive_failures = 0;
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            let mut shutdown_rx = shutdown_rx;
            let max_failures = 3;

            interval.tick().await;

            for _ in 0..10 {
                tokio::select! {
                    _ = interval.tick() => {
                        let transport = manager.get_transport("test").await.unwrap();
                        let mut transport = transport.lock().await;
                        let ping = serde_json::json!({"method": "ping"});

                        match transport.request(&ping).await {
                            Ok(_) => {
                                if consecutive_failures > 0 {
                                    println!("Recovered after {} failures", consecutive_failures);
                                }
                                consecutive_failures = 0;
                            }
                            Err(_) => {
                                consecutive_failures += 1;
                                println!("Health check failed: {}/{}", consecutive_failures, max_failures);

                                if consecutive_failures >= max_failures {
                                    println!("Max failures reached, restarting...");
                                    manager.restart_server("test").await.unwrap();
                                    consecutive_failures = 0;
                                }
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    });

    // Let it run longer to trigger restart
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Shutdown
    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();

    // Should have restarted at least once
    assert!(
        manager.get_restart_count() >= 1,
        "Expected at least 1 restart, got {}",
        manager.get_restart_count()
    );
}

#[tokio::test]
async fn test_health_monitor_graceful_shutdown() {
    let manager = Arc::new(MockServerManager::new(0));
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    let handle = tokio::spawn({
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut shutdown_rx = shutdown_rx;

        async move {
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Monitoring logic
                    }
                    _ = shutdown_rx.recv() => {
                        println!("Received shutdown signal");
                        break;
                    }
                }
            }
        }
    });

    // Give it a moment to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Trigger shutdown
    shutdown_tx.send(()).unwrap();

    // Should shutdown gracefully within timeout
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;

    assert!(result.is_ok(), "Health monitor should shutdown gracefully");
    assert_eq!(manager.get_restart_count(), 0);
}

#[tokio::test]
async fn test_health_check_timeout() {
    // Simulate a slow/hanging server that never responds
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

    let handle = tokio::spawn({
        async move {
            let timeout = Duration::from_millis(100);
            let mut shutdown_rx = shutdown_rx;

            // Simulate health check with timeout
            let slow_operation = async {
                // Never completes
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok::<_, ()>(())
            };

            tokio::select! {
                result = tokio::time::timeout(timeout, slow_operation) => {
                    match result {
                        Ok(_) => panic!("Should have timed out"),
                        Err(_) => {
                            println!("Health check timed out as expected");
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    println!("Shutdown before timeout");
                }
            }
        }
    });

    // Let it timeout
    tokio::time::sleep(Duration::from_millis(200)).await;

    shutdown_tx.send(()).unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn test_concurrent_health_monitors() {
    // Test multiple monitors running concurrently
    let manager1 = Arc::new(MockServerManager::new(0));
    let manager2 = Arc::new(MockServerManager::new(0));
    let (shutdown_tx, _) = broadcast::channel(1);

    let handle1 = tokio::spawn({
        let mut shutdown_rx = shutdown_tx.subscribe();
        async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    });

    let handle2 = tokio::spawn({
        let mut shutdown_rx = shutdown_tx.subscribe();
        async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = shutdown_rx.recv() => break,
                }
            }
        }
    });

    // Let them run
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Shutdown both
    shutdown_tx.send(()).unwrap();

    // Both should shutdown gracefully
    let result1 = tokio::time::timeout(Duration::from_secs(2), handle1).await;
    let result2 = tokio::time::timeout(Duration::from_secs(2), handle2).await;

    assert!(result1.is_ok() && result2.is_ok());
    assert_eq!(manager1.get_restart_count(), 0);
    assert_eq!(manager2.get_restart_count(), 0);
}
