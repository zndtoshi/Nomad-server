//! Nostr identity management
//! 
//! Handles generation and persistence of Nostr keypairs for the Umbrel node.

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use std::path::{Path, PathBuf};
use std::fs;

const KEYS_FILENAME: &str = "nostr_keys.json";

/// Manages the Umbrel node's Nostr identity
pub struct IdentityManager {
    keys: Keys,
    keys_path: PathBuf,
}

impl IdentityManager {
    /// Initialize identity manager, loading or creating keys
    /// 
    /// The data_dir should be the Umbrel app data directory where
    /// persistent data is stored (typically /umbrel/app-data/[app-id]/data)
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let keys_path = data_dir.join(KEYS_FILENAME);
        
        // Ensure data directory exists (Umbrel should create it, but be safe)
        fs::create_dir_all(data_dir)
            .context("Failed to create/access Umbrel data directory")?;
        
        let keys = if keys_path.exists() {
            Self::load_keys(&keys_path)?
        } else {
            let new_keys = Self::generate_keys()?;
            Self::save_keys(&keys_path, &new_keys)?;
            new_keys
        };
        
        Ok(Self {
            keys,
            keys_path,
        })
    }
    
    /// Get the Nostr keys
    pub fn keys(&self) -> &Keys {
        &self.keys
    }
    
    /// Get the public key as hex string
    pub fn public_key_hex(&self) -> String {
        self.keys.public_key().to_hex()
    }
    
    fn generate_keys() -> Result<Keys> {
        let keys = Keys::generate();
        Ok(keys)
    }
    
    fn load_keys(path: &Path) -> Result<Keys> {
        let content = fs::read_to_string(path)
            .context("Failed to read keys file")?;
        
        let json: serde_json::Value = serde_json::from_str(&content)
            .context("Invalid keys file format")?;
        
        let secret_key_hex = json.get("secret_key")
            .and_then(|v| v.as_str())
            .context("Missing secret_key in keys file")?;
        
        let secret_key = SecretKey::from_hex(secret_key_hex)
            .context("Invalid secret key format")?;
        
        let keys = Keys::new(secret_key);
        
        Ok(keys)
    }
    
    fn save_keys(path: &Path, keys: &Keys) -> Result<()> {
        let json = serde_json::json!({
            "secret_key": keys.secret_key().to_secret_hex(),
            "public_key": keys.public_key().to_hex(),
        });
        
        let content = serde_json::to_string_pretty(&json)
            .context("Failed to serialize keys")?;
        
        fs::write(path, content)
            .context("Failed to write keys file")?;
        
        Ok(())
    }
}

