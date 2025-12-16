//! Configuration management
//! 
//! Handles Umbrel-specific configuration and environment variables.

use std::env;
use std::path::PathBuf;

/// Get the Umbrel app data directory
/// 
/// Umbrel sets UMBREL_APP_DATA_DIR to the app's persistent data directory.
/// This directory is mounted as a volume and persists across container restarts.
pub fn get_data_dir() -> PathBuf {
    env::var("UMBREL_APP_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // Fallback for local development/testing
            PathBuf::from("./data")
        })
}

/// Get the Umbrel app ID
/// 
/// Umbrel sets UMBREL_APP_ID to identify the app instance.
pub fn get_app_id() -> Option<String> {
    env::var("UMBREL_APP_ID").ok()
}

