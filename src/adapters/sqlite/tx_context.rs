//! Task-local transaction context for transactional outbox atomicity.
//!
//! When the CommandBus dispatches a command with the outbox enabled, it begins
//! a SQLite transaction and sets it as the task-local context. All repository
//! operations within the handler's execution scope then use this transaction
//! instead of their own pool, ensuring that domain mutations and outbox event
//! inserts are committed atomically.
//!
//! # Atomicity guarantee
//!
//! The transaction is committed AFTER both the handler mutations and the outbox
//! inserts complete. If either fails, the entire transaction rolls back —
//! neither the mutation nor the outbox events are persisted.
//!
//! # At-least-once delivery
//!
//! The outbox poller reads unpublished events and publishes them to the EventBus.
//! If `mark_published` fails after a successful publish, the event will be
//! re-published on the next poll cycle. Downstream handlers MUST be idempotent
//! to tolerate duplicate delivery.

use std::sync::Arc;
use tokio::sync::Mutex;

/// Type alias for the shared transaction wrapped in Arc<Mutex> for task-local sharing.
pub type SharedTx = Arc<Mutex<sqlx::Transaction<'static, sqlx::Sqlite>>>;

tokio::task_local! {
    /// Task-local shared transaction context.
    ///
    /// When set, SQLite repository operations use this transaction instead of
    /// their pool, achieving transactional atomicity between handler mutations
    /// and outbox event inserts.
    pub static ACTIVE_TX: SharedTx;
}

/// Try to get the active task-local transaction, if one is set.
///
/// Returns `Some(Arc<Mutex<Transaction>>)` if a transaction scope is active,
/// or `None` if the current task is not within a transaction scope.
pub fn try_get_tx() -> Option<SharedTx> {
    ACTIVE_TX.try_with(|tx| tx.clone()).ok()
}

/// Run a future within a transaction scope.
///
/// All SQLite repository operations executed within the future will use
/// the provided transaction instead of their pool.
pub async fn run_in_tx_scope<F, T>(tx: SharedTx, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    ACTIVE_TX.scope(tx, f).await
}

/// Attempts to acquire exclusive ownership of a SharedTx for commit/rollback.
/// Retries up to 10 times with 5ms yields (~50ms budget) to give any spawned
/// holder a chance to drop their clone. On exhaustion, returns the SharedTx
/// back to the caller — dropping it will roll back the transaction cleanly.
pub async fn take_inner_tx(
    shared: SharedTx,
) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, SharedTx> {
    let mut shared = shared;
    for _ in 0..10 {
        match Arc::try_unwrap(shared) {
            Ok(m) => return Ok(m.into_inner()),
            Err(s) => {
                shared = s;
                tokio::task::yield_now().await;
                // tiny sleep to allow actual other tasks to run
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }
    }
    Err(shared)
}

/// Macro to execute a sqlx query using the active transaction if available,
/// otherwise the pool. Supports all common sqlx fetch methods.
///
/// # Usage
///
/// ```ignore
/// // Execute (INSERT, UPDATE, DELETE)
/// exec_tx!(pool, sqlx::query("INSERT ...").bind(val), execute)?;
///
/// // Fetch one row
/// let row: MyRow = exec_tx!(pool, sqlx::query_as("SELECT ..."), fetch_one)?;
///
/// // Fetch optional
/// let row: Option<MyRow> = exec_tx!(pool, sqlx::query_as("SELECT ..."), fetch_optional)?;
///
/// // Fetch all
/// let rows: Vec<MyRow> = exec_tx!(pool, sqlx::query_as("SELECT ..."), fetch_all)?;
/// ```
#[macro_export]
macro_rules! exec_tx {
    ($pool:expr, $query:expr, $method:ident) => {{
        let __tx_opt = $crate::adapters::sqlite::tx_context::try_get_tx();
        if let Some(__tx_arc) = __tx_opt {
            let mut __tx_guard = __tx_arc.lock().await;
            $query.$method(&mut **__tx_guard).await
        } else {
            $query.$method($pool).await
        }
    }};
}
