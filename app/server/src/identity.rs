//! Nostr identity management
//!
//! Handles generation and persistence of Nostr keypairs for the Umbrel node.

use nostr_sdk::{Keys, SecretKey};
use std::fs;
use std::path::Path;

const DATA_DIR: &str = "/data";
const KEY_FILE: &str = "/data/nostr_secret.hex";

pub fn load_or_create_keys() -> Keys {
    fs::create_dir_all(DATA_DIR).ok();

    if Path::new(KEY_FILE).exists() {
        let hex_str = fs::read_to_string(KEY_FILE)
            .expect("Failed to read nostr secret key file")
            .trim()
            .to_string();

        let bytes = hex::decode(&hex_str)
            .expect("Invalid hex in nostr_secret.hex");

        let secret_key = SecretKey::from_slice(&bytes)
            .expect("Invalid secret key bytes");

        let keys = Keys::new(secret_key);

        log::info!(
            "Loaded persisted Nostr pubkey: {}",
            keys.public_key().to_hex()
        );

        keys
    } else {
        let keys = Keys::generate();

        let secret = keys.secret_key();
        let hex_str = hex::encode(secret.as_secret_bytes());

        fs::write(KEY_FILE, &hex_str)
            .expect("Failed to persist nostr secret key");

        log::info!(
            "Generated NEW Nostr pubkey (persisted): {}",
            keys.public_key().to_hex()
        );

        keys
    }
}

