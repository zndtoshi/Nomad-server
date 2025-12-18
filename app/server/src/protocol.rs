use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct BitcoinLookupRequest {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BitcoinLookupResponse {
    pub query: String,
    pub confirmed_balance: u64,
    pub unconfirmed_balance: u64,
    pub transactions: Vec<TransactionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionInfo {
    pub txid: String,
}

impl BitcoinLookupResponse {
    pub fn new(query: String) -> Self {
        Self {
            query,
            confirmed_balance: 0,
            unconfirmed_balance: 0,
            transactions: Vec::new(),
        }
    }
}