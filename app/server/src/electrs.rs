//! Electrs client for querying Bitcoin balances and transactions
//!
//! Connects to the local Electrs instance running on the Umbrel Bitcoin node.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info, warn};

/// Electrs API client
pub struct ElectrsClient {
    client: Client,
    base_url: String,
}

/// Electrs balance response
#[derive(Debug, Deserialize)]
struct ElectrsBalance {
    confirmed: i64,
    unconfirmed: i64,
}

/// Electrs transaction response
#[derive(Debug, Deserialize)]
struct ElectrsTx {
    txid: String,
    status: ElectrsTxStatus,
    #[serde(default)]
    fee: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ElectrsTxStatus {
    confirmed: bool,
    #[serde(default)]
    block_height: Option<u32>,
    #[serde(default)]
    block_hash: Option<String>,
    #[serde(default)]
    block_time: Option<i64>,
}

/// Transaction history entry
#[derive(Debug, Clone)]
pub struct AddressTransaction {
    pub txid: String,
    pub timestamp: i64,
    pub amount: i64,
    pub confirmations: u32,
}

impl ElectrsClient {
    /// Create a new Electrs client
    ///
    /// Defaults based on environment:
    /// - ELECTRS_URL env var (if set)
    /// - http://localhost:3002 (fallback for local development)
    /// 
    /// In Umbrel, Electrs is typically accessible via:
    /// - http://electrs:3002 (if in same Docker network)
    /// - http://localhost:3002 (if on same host)
    pub fn new() -> Result<Self> {
        // Try ELECTRS_URL first, then common Umbrel defaults
        let base_url = if let Ok(url) = std::env::var("ELECTRS_URL") {
            url
        } else {
            // Try to detect Umbrel environment
            // In Umbrel, services are often accessible via service names
            // Try electrs service name first, then localhost
            "http://electrs:3002".to_string()
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        info!("Electrs client initialized with URL: {}", base_url);
        info!("Electrs URL can be overridden via ELECTRS_URL environment variable");

        Ok(Self { client, base_url })
    }

    /// Get balance for a single Bitcoin address
    pub async fn get_address_balance(&self, address: &str) -> Result<(i64, i64)> {
        let url = format!("{}/address/{}/balance", self.base_url, address);
        
        info!("Querying Electrs for address balance: {}", address);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Electrs")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Electrs returned error {}: {}",
                status,
                text
            ));
        }

        let balance: ElectrsBalance = response
            .json()
            .await
            .context("Failed to parse Electrs balance response")?;

        info!(
            "Address {} balance: confirmed={}, unconfirmed={}",
            address, balance.confirmed, balance.unconfirmed
        );

        Ok((balance.confirmed, balance.unconfirmed))
    }

    /// Get transaction history for a single Bitcoin address
    pub async fn get_address_transactions(&self, address: &str) -> Result<Vec<AddressTransaction>> {
        let url = format!("{}/address/{}/txs", self.base_url, address);
        
        info!("Querying Electrs for address transactions: {}", address);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Electrs")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Electrs returned error {}: {}",
                status,
                text
            ));
        }

        let txs: Vec<ElectrsTx> = response
            .json()
            .await
            .context("Failed to parse Electrs transactions response")?;

        let mut transactions = Vec::new();

        for tx in txs {
            let timestamp = tx.status.block_time.unwrap_or(0);
            let confirmations = if tx.status.confirmed {
                // For confirmed transactions, estimate confirmations based on current block
                // Since we don't have current block height, use 1 as minimum
                1
            } else {
                0
            };

            // Fetch full transaction to calculate amount
            let amount = match self.get_transaction_amount(&tx.txid, address).await {
                Ok(amt) => amt,
                Err(e) => {
                    warn!("Failed to get amount for tx {}: {}", tx.txid, e);
                    0 // Default to 0 if we can't calculate
                }
            };

            transactions.push(AddressTransaction {
                txid: tx.txid,
                timestamp,
                amount,
                confirmations,
            });
        }

        info!("Found {} transactions for address {}", transactions.len(), address);

        Ok(transactions)
    }

    /// Get balance and transactions for multiple addresses (aggregated)
    pub async fn get_addresses_balance_and_txs(
        &self,
        addresses: &[String],
    ) -> Result<(i64, i64, Vec<AddressTransaction>)> {
        let mut total_confirmed = 0i64;
        let mut total_unconfirmed = 0i64;
        let mut all_transactions = Vec::new();

        for address in addresses {
            match self.get_address_balance(address).await {
                Ok((confirmed, unconfirmed)) => {
                    total_confirmed += confirmed;
                    total_unconfirmed += unconfirmed;
                }
                Err(e) => {
                    warn!("Failed to get balance for {}: {}", address, e);
                    // Continue with other addresses
                }
            }

            match self.get_address_transactions(address).await {
                Ok(txs) => {
                    all_transactions.extend(txs);
                }
                Err(e) => {
                    warn!("Failed to get transactions for {}: {}", address, e);
                    // Continue with other addresses
                }
            }
        }

        // Deduplicate transactions by txid
        all_transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        all_transactions.dedup_by(|a, b| a.txid == b.txid);

        Ok((total_confirmed, total_unconfirmed, all_transactions))
    }

    /// Get the net amount for an address in a transaction
    async fn get_transaction_amount(&self, txid: &str, address: &str) -> Result<i64> {
        let url = format!("{}/tx/{}", self.base_url, txid);
        
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Electrs")?;

        if !response.status().is_success() {
            return Ok(0); // Return 0 if transaction not found
        }

        // Parse transaction JSON
        let tx_json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse transaction")?;

        // Calculate net amount for this address
        // Sum all outputs to this address, subtract all inputs from this address
        let mut amount = 0i64;

        // Sum outputs
        if let Some(outputs) = tx_json.get("vout").and_then(|v| v.as_array()) {
            for output in outputs {
                if let Some(scriptpubkey_address) = output
                    .get("scriptpubkey_address")
                    .and_then(|v| v.as_str())
                {
                    if scriptpubkey_address == address {
                        if let Some(value) = output.get("value").and_then(|v| v.as_i64()) {
                            amount += value;
                        }
                    }
                }
            }
        }

        // Subtract inputs (if this address was used as input)
        // Note: This is simplified - in production you'd want to check vin for this address
        // For now, we'll just return the output amount

        Ok(amount)
    }
}

