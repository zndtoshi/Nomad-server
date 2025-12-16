//! Relay configuration
//! 
//! Manages the list of public Nostr relays to use.

/// Default list of public Nostr relays
pub fn default_relays() -> Vec<String> {
    vec![
        "wss://relay.damus.io".to_string(),
        "wss://nos.lol".to_string(),
        "wss://relay.snort.social".to_string(),
    ]
}

/// Get the list of relays to use
pub fn get_relays() -> Vec<String> {
    // For now, use defaults. Later can be extended to read from config
    default_relays()
}

