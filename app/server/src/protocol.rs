//! Protocol definitions for Bitcoin lookup requests and responses
//!
//! Defines the JSON structures used for communication between Android app and Umbrel server.

use serde::{Deserialize, Serialize};

/// Bitcoin lookup request from Android app
#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinLookupRequest {
    #[serde(rename = "type")]
    pub request_type: String,
    pub query: String,
}

impl BitcoinLookupRequest {
    /// Validate that this is a valid Bitcoin lookup request
    pub fn is_valid(&self) -> bool {
        self.request_type == "bitcoin_lookup" && !self.query.is_empty()
    }
}

/// Transaction information in response
#[derive(Debug, Clone, Serialize)]
pub struct TransactionInfo {
    pub txid: String,
    pub timestamp: i64,
    pub amount: i64,
    pub confirmations: u32,
}

/// Bitcoin lookup response to Android app
#[derive(Debug, Clone, Serialize)]
pub struct BitcoinLookupResponse {
    #[serde(rename = "type")]
    pub response_type: String,
    pub query: String,
    pub confirmed_balance: i64,
    pub unconfirmed_balance: i64,
    pub transactions: Vec<TransactionInfo>,
}

impl BitcoinLookupResponse {
    /// Create a new response
    pub fn new(query: String) -> Self {
        Self {
            response_type: "bitcoin_lookup_result".to_string(),
            query,
            confirmed_balance: 0,
            unconfirmed_balance: 0,
            transactions: Vec::new(),
        }
    }
}
