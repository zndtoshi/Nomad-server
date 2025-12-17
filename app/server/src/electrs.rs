use anyhow::{Result, anyhow};
use electrum_client::{Client, ElectrumApi};
use electrum_client::bitcoin::{Address, ScriptBuf, Network};
use std::str::FromStr;
use tracing::info;

pub struct ElectrsClient {
    client: Client,
}

impl ElectrsClient {
    pub async fn new() -> Result<Self> {
        // Umbrel-safe default: localhost
        let addr = std::env::var("ELECTRS_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:50001".to_string());

        info!("Connecting to Electrs at {}", addr);

        let client = Client::new(&format!("tcp://{}", addr))
            .map_err(|e| anyhow!("Failed to connect to Electrs: {}", e))?;

        Ok(Self { client })
    }

    pub async fn test_connectivity(&self) -> Result<()> {
        self.client
            .ping()
            .map_err(|e| anyhow!("Electrs ping failed: {}", e))?;
        Ok(())
    }

    pub async fn get_address_balance(&self, address: &str) -> Result<(u64, u64)> {
        let addr = Address::from_str(address)?
            .require_network(Network::Bitcoin)?;
        let script: ScriptBuf = addr.script_pubkey();

        let balance = self.client.script_get_balance(&script)?;
        let confirmed = balance.confirmed.max(0) as u64;
        let unconfirmed = balance.unconfirmed.max(0) as u64;

        Ok((confirmed, unconfirmed))
    }

    pub async fn get_address_txs(&self, address: &str) -> Result<Vec<String>> {
        let addr = Address::from_str(address)?
            .require_network(Network::Bitcoin)?;
        let script: ScriptBuf = addr.script_pubkey();

        let history = self.client.script_get_history(&script)?;
        Ok(history
            .into_iter()
            .map(|h| h.tx_hash.to_string())
            .collect())
    }
}

