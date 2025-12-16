//! Error types for the BalanceBridge server

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("Nostr error: {0}")]
    Nostr(#[from] nostr_sdk::client::Error),
    
    #[error("Relay connection failed: {0}")]
    RelayConnection(String),
    
    #[error("Encryption error: {0}")]
    Encryption(String),
    
    #[error("Invalid event: {0}")]
    InvalidEvent(String),
}

pub type ServerResult<T> = Result<T, ServerError>;

