mod helpers;

use abathur_cli::domain::models::{Session, SessionEvent};
use abathur_cli::domain::ports::SessionRepository;
use abathur_cli::infrastructure::database::SessionRepositoryImpl;
use serde_json::json;
use uuid::Uuid;

use helpers::database::{setup_test_db, teardown_test_db};

#[tokio::test]
async fn test_create_and_get_session() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    let session = Session::new(
        "test-app".to_string(),
        "user123".to_string(),
        Some("project456".to_string()),
    );
    let session_id = session.id;

    // Create session
    let created_id = repo
        .create(session.clone())
        .await
        .expect("failed to create session");
    assert_eq!(created_id, session_id);

    // Get session
    let retrieved = repo.get(session_id).await.expect("failed to get session");
    assert!(retrieved.is_some());

    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, session_id);
    assert_eq!(retrieved.app_name, "test-app");
    assert_eq!(retrieved.user_id, "user123");
    assert_eq!(retrieved.project_id, Some("project456".to_string()));
    assert_eq!(retrieved.state, json!({}));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_nonexistent_session() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    let nonexistent_id = Uuid::new_v4();
    let result = repo.get(nonexistent_id).await.expect("failed to query");
    assert!(result.is_none());

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_append_and_get_events() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session first
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    // Create and append events
    let event1 = SessionEvent::new(
        session.id,
        "user_message".to_string(),
        "user123".to_string(),
        json!({"message": "Hello"}),
    );

    let event2 = SessionEvent::new(
        session.id,
        "assistant_message".to_string(),
        "assistant".to_string(),
        json!({"message": "Hi there!"}),
    );

    repo.append_event(session.id, event1.clone())
        .await
        .expect("failed to append event 1");
    repo.append_event(session.id, event2.clone())
        .await
        .expect("failed to append event 2");

    // Get events
    let retrieved_events = repo
        .get_events(session.id)
        .await
        .expect("failed to get events");

    assert_eq!(retrieved_events.len(), 2);
    assert_eq!(retrieved_events[0].event_type, "user_message");
    assert_eq!(retrieved_events[0].actor, "user123");
    assert_eq!(retrieved_events[0].content, json!({"message": "Hello"}));

    assert_eq!(retrieved_events[1].event_type, "assistant_message");
    assert_eq!(retrieved_events[1].actor, "assistant");
    assert_eq!(retrieved_events[1].content, json!({"message": "Hi there!"}));

    // Verify events are ordered by timestamp
    assert!(retrieved_events[0].timestamp <= retrieved_events[1].timestamp);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_events_empty() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session with no events
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    let events = repo
        .get_events(session.id)
        .await
        .expect("failed to get events");
    assert_eq!(events.len(), 0);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_get_and_set_state() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    // Set state values
    repo.set_state(session.id, "theme", json!("dark"))
        .await
        .expect("failed to set theme");

    repo.set_state(session.id, "language", json!("en"))
        .await
        .expect("failed to set language");

    // Get state values
    let theme = repo
        .get_state(session.id, "theme")
        .await
        .expect("failed to get theme");
    assert_eq!(theme, Some(json!("dark")));

    let language = repo
        .get_state(session.id, "language")
        .await
        .expect("failed to get language");
    assert_eq!(language, Some(json!("en")));

    // Get nonexistent key
    let nonexistent = repo
        .get_state(session.id, "nonexistent")
        .await
        .expect("failed to query state");
    assert_eq!(nonexistent, None);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_set_state_merges_not_replaces() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    // Set multiple state values
    repo.set_state(session.id, "key1", json!("value1"))
        .await
        .expect("failed to set key1");

    repo.set_state(session.id, "key2", json!("value2"))
        .await
        .expect("failed to set key2");

    // Verify both keys exist (merge behavior)
    let key1 = repo
        .get_state(session.id, "key1")
        .await
        .expect("failed to get key1");
    assert_eq!(key1, Some(json!("value1")));

    let key2 = repo
        .get_state(session.id, "key2")
        .await
        .expect("failed to get key2");
    assert_eq!(key2, Some(json!("value2")));

    // Update key1
    repo.set_state(session.id, "key1", json!("updated_value1"))
        .await
        .expect("failed to update key1");

    // Verify key1 is updated and key2 still exists
    let key1 = repo
        .get_state(session.id, "key1")
        .await
        .expect("failed to get updated key1");
    assert_eq!(key1, Some(json!("updated_value1")));

    let key2 = repo
        .get_state(session.id, "key2")
        .await
        .expect("failed to get key2 after update");
    assert_eq!(key2, Some(json!("value2")));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_set_state_nonexistent_session() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    let nonexistent_id = Uuid::new_v4();
    let result = repo.set_state(nonexistent_id, "key", json!("value")).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Session not found"));

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_cascade_delete() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session with events
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    let event = SessionEvent::new(
        session.id,
        "test_event".to_string(),
        "user".to_string(),
        json!({}),
    );
    repo.append_event(session.id, event)
        .await
        .expect("failed to append event");

    // Verify event exists
    let events = repo
        .get_events(session.id)
        .await
        .expect("failed to get events");
    assert_eq!(events.len(), 1);

    // Delete session (using unchecked query since this is a test)
    let session_id_str = session.id.to_string();
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(session_id_str)
        .execute(&pool)
        .await
        .expect("failed to delete session");

    // Verify events are also deleted (cascade)
    let events_after = repo
        .get_events(session.id)
        .await
        .expect("failed to query events");
    assert_eq!(events_after.len(), 0);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_state_updated_at_changes() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    let original_updated_at = session.updated_at;
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    // Wait a moment to ensure timestamp difference
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Set state
    repo.set_state(session.id, "key", json!("value"))
        .await
        .expect("failed to set state");

    // Get session and verify updated_at changed
    let updated_session = repo
        .get(session.id)
        .await
        .expect("failed to get session")
        .unwrap();
    assert!(updated_session.updated_at > original_updated_at);

    teardown_test_db(pool).await;
}

#[tokio::test]
async fn test_complex_state_values() {
    let pool = setup_test_db().await;
    let repo = SessionRepositoryImpl::new(pool.clone());

    // Create session
    let session = Session::new("test-app".to_string(), "user123".to_string(), None);
    repo.create(session.clone())
        .await
        .expect("failed to create session");

    // Set complex nested state value
    let complex_value = json!({
        "preferences": {
            "theme": "dark",
            "fontSize": 14,
            "features": ["vim", "autocomplete"]
        },
        "history": [1, 2, 3, 4, 5]
    });

    repo.set_state(session.id, "user_prefs", complex_value.clone())
        .await
        .expect("failed to set complex state");

    // Retrieve and verify
    let retrieved = repo
        .get_state(session.id, "user_prefs")
        .await
        .expect("failed to get complex state");

    assert_eq!(retrieved, Some(complex_value));

    teardown_test_db(pool).await;
}
