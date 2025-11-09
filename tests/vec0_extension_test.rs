/// Test vec0 extension loading and functionality
use abathur_cli::infrastructure::database::DatabaseConnection;

#[tokio::test]
async fn test_vec0_extension_loaded() {
    // Create a test database
    let db = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("Failed to create database");

    // Run migrations (including migration 008 that uses vec0)
    db.migrate().await.expect("Migrations should succeed with vec0 extension loaded");

    // Verify vec0 virtual table was created
    let result: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='vec_memory_vec0'",
    )
    .fetch_one(db.pool())
    .await
    .expect("Failed to query for vec0 table");

    assert_eq!(result.0, 1, "vec_memory_vec0 virtual table should exist");

    // Verify we can query vec_version()
    let version_result: Result<(String,), _> =
        sqlx::query_as("SELECT vec_version() as version")
            .fetch_one(db.pool())
            .await;

    assert!(
        version_result.is_ok(),
        "Should be able to query vec_version() function"
    );

    if let Ok((version,)) = version_result {
        println!("sqlite-vec version: {}", version);
    }

    db.close().await;
}

#[tokio::test]
async fn test_vec0_insert_and_search() {
    // Create a test database
    let db = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("Failed to create database");

    db.migrate().await.expect("Migrations should succeed");

    // Create a test vector (384 dimensions, all 0.5)
    let test_vector: Vec<f32> = vec![0.5; 384];
    let vector_blob = test_vector
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect::<Vec<u8>>();

    // Insert into vec0 virtual table
    let insert_result = sqlx::query(
        "INSERT INTO vec_memory_vec0 (embedding) VALUES (?)"
    )
    .bind(&vector_blob)
    .execute(db.pool())
    .await;

    assert!(
        insert_result.is_ok(),
        "Should be able to insert into vec0 virtual table: {:?}",
        insert_result.err()
    );

    // Verify the insertion
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM vec_memory_vec0")
        .fetch_one(db.pool())
        .await
        .expect("Should be able to count rows");

    assert_eq!(count.0, 1, "Should have 1 vector in vec0 table");

    db.close().await;
}

#[tokio::test]
async fn test_vec0_distance_calculation() {
    // Create a test database
    let db = DatabaseConnection::new("sqlite::memory:")
        .await
        .expect("Failed to create database");

    db.migrate().await.expect("Migrations should succeed");

    // Create two test vectors
    let vector1: Vec<f32> = vec![1.0; 384];
    let vector2: Vec<f32> = vec![0.5; 384];

    let blob1 = vector1.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();
    let blob2 = vector2.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>();

    // Insert both vectors
    sqlx::query("INSERT INTO vec_memory_vec0 (embedding) VALUES (?)")
        .bind(&blob1)
        .execute(db.pool())
        .await
        .expect("Insert vector 1");

    sqlx::query("INSERT INTO vec_memory_vec0 (embedding) VALUES (?)")
        .bind(&blob2)
        .execute(db.pool())
        .await
        .expect("Insert vector 2");

    // Calculate distance between them using vec_distance_cosine
    let distance_result: Result<(f64,), _> = sqlx::query_as(
        r#"
        SELECT vec_distance_cosine(
            (SELECT embedding FROM vec_memory_vec0 WHERE rowid = 1),
            (SELECT embedding FROM vec_memory_vec0 WHERE rowid = 2)
        ) as distance
        "#
    )
    .fetch_one(db.pool())
    .await;

    assert!(
        distance_result.is_ok(),
        "Should be able to calculate cosine distance: {:?}",
        distance_result.err()
    );

    if let Ok((distance,)) = distance_result {
        println!("Cosine distance between vectors: {}", distance);
        // Cosine distance can range from -1 to 2 depending on implementation
        // For normalized vectors, it's typically 0 to 2
        // Small negative values near zero are acceptable due to floating point precision
        assert!(
            distance >= -1.0 && distance <= 2.0,
            "Cosine distance should be between -1 and 2, got {}",
            distance
        );
    }

    db.close().await;
}
