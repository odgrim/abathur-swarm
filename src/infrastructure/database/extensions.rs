//! SQLite extension registration
//!
//! This module handles registration of SQLite extensions that are statically
//! linked into the binary. Extensions must be registered before any database
//! connections are created.

use std::ffi::c_char;
use std::sync::Once;

/// Ensure extensions are only registered once
static INIT: Once = Once::new();

/// Register sqlite-vec extension with SQLite
///
/// This function uses `sqlite3_auto_extension()` to register the vec0
/// extension globally. The extension will be automatically loaded for
/// all SQLite connections created after this call.
///
/// # Safety
///
/// This function is safe to call multiple times - it will only register
/// the extension once due to the `Once` guard. However, it uses unsafe
/// code internally to:
/// - Cast the extension init function to the correct signature
/// - Call the FFI function `sqlite3_auto_extension()`
///
/// # Panics
///
/// Panics if the extension registration fails. This should only happen
/// if SQLite is in an invalid state.
pub fn register_sqlite_vec() {
    INIT.call_once(|| {
        unsafe {
            // Get the sqlite-vec init function
            // The function signature expected by sqlite3_auto_extension is:
            // unsafe extern "C" fn(*mut sqlite3, *mut *mut c_char, *const sqlite3_api_routines) -> i32
            let vec_init = sqlite_vec::sqlite3_vec_init as *const ();

            // Transmute to the correct function signature
            // Note: The second parameter is *mut *mut c_char (mutable pointer to mutable pointer)
            let vec_init_fn: unsafe extern "C" fn(
                *mut libsqlite3_sys::sqlite3,
                *mut *mut c_char,
                *const libsqlite3_sys::sqlite3_api_routines,
            ) -> i32 = std::mem::transmute(vec_init);

            // Register the extension to be loaded automatically for all connections
            let result = libsqlite3_sys::sqlite3_auto_extension(Some(vec_init_fn));

            if result != libsqlite3_sys::SQLITE_OK {
                panic!(
                    "Failed to register sqlite-vec extension: error code {}",
                    result
                );
            }

            tracing::info!("sqlite-vec extension registered successfully");
        }
    });
}

/// Check if the vec0 extension is available
///
/// This should be called after a database connection is established
/// to verify the extension is working properly.
///
/// # Arguments
/// * `pool` - SQLite connection pool
///
/// # Returns
/// * `true` if the vec0 extension is available
/// * `false` otherwise
pub async fn is_vec0_available(pool: &sqlx::SqlitePool) -> bool {
    // Try to query the vec_version function
    match sqlx::query("SELECT vec_version() as version")
        .fetch_optional(pool)
        .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_sqlite_vec_multiple_times() {
        // Should be safe to call multiple times
        register_sqlite_vec();
        register_sqlite_vec();
        register_sqlite_vec();
    }
}
