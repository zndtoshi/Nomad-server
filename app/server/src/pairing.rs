//! Pairing management for Android app
//!
//! Stores and retrieves the paired Android app's public key and relay list.

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const PAIRING_FILENAME: &str = "android_pairing.json";

/// Pairing information for Android app
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AndroidPairing {
    pub android_pubkey: String,
    pub relays: Vec<String>,
}

/// Manages Android app pairing
#[derive(Clone)]
pub struct PairingManager {
    pairing_path: PathBuf,
}

impl PairingManager {
    /// Initialize pairing manager
    pub fn new(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        let pairing_path = data_dir.join(PAIRING_FILENAME);

        // Ensure data directory exists
        fs::create_dir_all(data_dir)
            .context("Failed to create data directory")?;

        Ok(Self { pairing_path })
    }

    /// Check if an Android app is paired
    pub fn has_pairing(&self) -> bool {
        self.pairing_path.exists()
    }

    /// Get the paired Android pubkey
    pub fn get_android_pubkey(&self) -> Result<Option<PublicKey>> {
        if !self.has_pairing() {
            return Ok(None);
        }

        let pairing = self.load_pairing()?;
        let pubkey = PublicKey::from_hex(&pairing.android_pubkey)
            .context("Invalid Android pubkey in pairing file")?;

        Ok(Some(pubkey))
    }

    /// Get the relay list from pairing
    pub fn get_relays(&self) -> Result<Vec<String>> {
        if !self.has_pairing() {
            return Ok(Vec::new());
        }

        let pairing = self.load_pairing()?;
        Ok(pairing.relays)
    }

    /// Store pairing information (called when "hello / paired" is received)
    pub fn store_pairing(&self, android_pubkey: PublicKey, relays: Vec<String>) -> Result<()> {
        let pairing = AndroidPairing {
            android_pubkey: android_pubkey.to_hex(),
            relays,
        };

        let json = serde_json::to_string_pretty(&pairing)
            .context("Failed to serialize pairing")?;

        fs::write(&self.pairing_path, json)
            .context("Failed to write pairing file")?;

        info!("Stored Android pairing: {}", android_pubkey.to_hex());

        Ok(())
    }

    fn load_pairing(&self) -> Result<AndroidPairing> {
        let content = fs::read_to_string(&self.pairing_path)
            .context("Failed to read pairing file")?;

        let pairing: AndroidPairing = serde_json::from_str(&content)
            .context("Invalid pairing file format")?;

        Ok(pairing)
    }
}

