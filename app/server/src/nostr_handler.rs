use anyhow::{anyhow, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use tracing::{error, info, warn};

use crate::electrs::ElectrsClient;
use crate::nostr::NostrState;
use crate::pairing::PairingManager;

pub const NOMAD_SERVER_REQUEST_KIND: u16 = 30078;
pub const NOMAD_SERVER_RESPONSE_KIND: u16 = 30079;

/* -------------------- Request / Response -------------------- */

#[derive(Debug, Serialize, Deserialize)]
struct BitcoinLookupRequest {
    #[serde(rename = "type")]
    req_type: String,
    query: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BroadcastTxRequest {
    #[serde(rename = "type")]
    req_type: String,
    #[serde(rename = "txHex")]
    tx_hex: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetFeesRequest {
    #[serde(rename = "type")]
    req_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetUtxosRequest {
    #[serde(rename = "type")]
    req_type: String,
    addresses: Vec<String>,
}

/*
 Android MVP compatibility:
 - req inside JSON
 - legacy field names
*/
#[derive(Debug, Serialize)]
struct BitcoinLookupResponse {
    // Android MVP fields
    req: String,
    confirmedBalance: u64,
    unconfirmedBalance: u64,
    confirmations: u64,
    amount: u64,

    // Modern fields
    confirmed_balance: u64,
    unconfirmed_balance: u64,
    transactions: Vec<TransactionInfo>,
}

#[derive(Debug, Serialize)]
struct BroadcastTxResponse {
    req: String,
    success: bool,
    txid: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct GetFeesResponse {
    req: String,
    fast: u64,   // sat/vB
    medium: u64, // sat/vB
    slow: u64,   // sat/vB
}

#[derive(Debug, Serialize)]
struct GetUtxosResponse {
    req: String,
    utxos: Vec<UtxoInfo>,
}

#[derive(Debug, Serialize)]
struct TransactionInfo {
    txid: String,
}

#[derive(Debug, Serialize)]
pub struct UtxoInfo {
    pub txid: String,
    pub vout: u32,
    pub value: u64,
    pub address: String,
    pub confirmations: u32,
}

/* -------------------- Handler -------------------- */

pub struct NostrHandler {
    client: Arc<Client>,
    keys: Keys,
    electrs_client: Arc<ElectrsClient>,
}

impl NostrHandler {
    pub async fn new(
        nostr_state: NostrState,
        keys: Keys,
        _pairing_manager: PairingManager,
        electrs_client: Arc<ElectrsClient>,
    ) -> Result<Self> {
        Ok(Self {
            client: nostr_state.client.clone(),
            keys,
            electrs_client,
        })
    }

    pub async fn start_listening(&self) -> Result<()> {
        let filter = Filter::new()
            .kinds(vec![Kind::Custom(NOMAD_SERVER_REQUEST_KIND)]);

        self.client.subscribe(filter, None).await?;

        info!(
            "Subscribed to NomadServer request kind={}",
            NOMAD_SERVER_REQUEST_KIND
        );

        let mut notifications = self.client.notifications();

        // IMPORTANT: never exit this loop on bad events
        while let Ok(notification) = notifications.recv().await {
            if let RelayPoolNotification::Event { event, .. } = notification {
                if event.kind.as_u16() != NOMAD_SERVER_REQUEST_KIND {
                    continue;
                }

                let from_pk = event.pubkey;

                // 🔑 FIX: ignore events without req tag instead of crashing
                let req_id = match extract_req_id(&event) {
                    Some(v) => v,
                    None => {
                        warn!(
                            "Ignoring NomadServer request without req tag (from={})",
                            from_pk.to_hex()
                        );
                        continue;
                    }
                };

                // Parse JSON to extract type field
                let content_value: serde_json::Value = match serde_json::from_str(&event.content) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            "Invalid request JSON (from={} req={}): {}",
                            from_pk.to_hex(),
                            req_id,
                            e
                        );
                        continue;
                    }
                };

                let req_type = content_value
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Route based on message type
                let result = match req_type.as_str() {
                    "bitcoin_lookup" => {
                        let parsed: BitcoinLookupRequest =
                            match serde_json::from_value(content_value.clone()) {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!("Invalid bitcoin_lookup request: {}", e);
                                    continue;
                                }
                            };

                        info!(
                            "Nostr lookup request: from={} req={} query={}",
                            from_pk.to_hex(),
                            req_id,
                            parsed.query
                        );

                        self.lookup_and_publish(from_pk, &req_id, parsed.query)
                            .await
                    }

                    "broadcast_tx" => {
                        let parsed: BroadcastTxRequest =
                            match serde_json::from_value(content_value.clone()) {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!("Invalid broadcast_tx request: {}", e);
                                    continue;
                                }
                            };

                        info!(
                            "Nostr broadcast_tx request: from={} req={}",
                            from_pk.to_hex(),
                            req_id
                        );

                        self.broadcast_and_publish(from_pk, &req_id, parsed.tx_hex)
                            .await
                    }

                    "get_fees" => {
                        info!(
                            "Nostr get_fees request: from={} req={}",
                            from_pk.to_hex(),
                            req_id
                        );

                        self.fees_and_publish(from_pk, &req_id).await
                    }

                    "get_utxos" => {
                        let parsed: GetUtxosRequest = match serde_json::from_value(content_value.clone()) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!("Invalid get_utxos request: {}", e);
                                continue;
                            }
                        };

                        info!(
                            "Nostr get_utxos request: from={} req={} addresses={}",
                            from_pk.to_hex(),
                            req_id,
                            parsed.addresses.len()
                        );

                        self.utxos_and_publish(from_pk, &req_id, parsed.addresses)
                            .await
                    }

                    _ => {
                        warn!(
                            "Unknown request type: {} from={} req={}",
                            req_type,
                            from_pk.to_hex(),
                            req_id
                        );
                        continue;
                    }
                };

                if let Err(e) = result {
                    error!(
                        "Request failed: type={} from={} req={} err={}",
                        req_type,
                        from_pk.to_hex(),
                        req_id,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    async fn lookup_and_publish(
        &self,
        to_pubkey: PublicKey,
        req_id: &str,
        address: String,
    ) -> Result<()> {
        let (confirmed, unconfirmed) = timeout(
            Duration::from_secs(30),
            self.electrs_client.get_address_balance(&address),
        )
        .await
        .map_err(|_| anyhow!("Electrs balance timeout"))??;

        let txids = match timeout(
            Duration::from_secs(20),
            self.electrs_client.get_address_txs(&address),
        )
        .await
        {
            Ok(Ok(v)) => v,
            _ => vec![],
        };

        info!(
            "Lookup OK: req={} confirmed={} unconfirmed={} txs={}",
            req_id,
            confirmed,
            unconfirmed,
            txids.len()
        );

        let response = BitcoinLookupResponse {
            req: req_id.to_string(),
            confirmedBalance: confirmed,
            unconfirmedBalance: unconfirmed,
            confirmations: txids.len() as u64,
            amount: confirmed + unconfirmed,

            confirmed_balance: confirmed,
            unconfirmed_balance: unconfirmed,
            transactions: txids
                .into_iter()
                .map(|txid| TransactionInfo { txid })
                .collect(),
        };

        let json = serde_json::to_string(&response)?;

        let tags = vec![
            Tag::parse(["p", to_pubkey.to_hex().as_str()])?,
            Tag::parse(["req", req_id])?,
        ];

        let event = EventBuilder::new(
            Kind::Custom(NOMAD_SERVER_RESPONSE_KIND),
            json,
        )
        .tags(tags)
        .sign_with_keys(&self.keys)?;

        info!(
            "Publishing response: kind={} to={} req={}",
            NOMAD_SERVER_RESPONSE_KIND,
            to_pubkey.to_hex(),
            req_id
        );

        self.client.send_event(&event).await?;

        Ok(())
    }

    async fn broadcast_and_publish(
        &self,
        to_pubkey: PublicKey,
        req_id: &str,
        tx_hex: String,
    ) -> Result<()> {
        info!("Broadcasting transaction: req={}", req_id);

        let electrs = self.electrs_client.clone();
        let hex = tx_hex.to_string();

        let result = timeout(
            Duration::from_secs(30),
            electrs.broadcast_transaction(&hex),
        )
        .await;

        let response = match result {
            Ok(Ok(txid)) => {
                info!("Broadcast OK: req={} txid={}", req_id, txid);
                BroadcastTxResponse {
                    req: req_id.to_string(),
                    success: true,
                    txid: Some(txid),
                    error: None,
                }
            }
            Ok(Err(e)) => {
                warn!("Broadcast failed: req={} err={}", req_id, e);
                BroadcastTxResponse {
                    req: req_id.to_string(),
                    success: false,
                    txid: None,
                    error: Some(format!("{}", e)),
                }
            }
            Err(_) => {
                warn!("Broadcast timeout: req={}", req_id);
                BroadcastTxResponse {
                    req: req_id.to_string(),
                    success: false,
                    txid: None,
                    error: Some("Timeout".to_string()),
                }
            }
        };

        let json = serde_json::to_string(&response)?;

        let tags = vec![
            Tag::parse(["p", to_pubkey.to_hex().as_str()])?,
            Tag::parse(["req", req_id])?,
        ];

        let event = EventBuilder::new(
            Kind::Custom(NOMAD_SERVER_RESPONSE_KIND),
            json,
        )
        .tags(tags)
        .sign_with_keys(&self.keys)?;

        self.client.send_event(&event).await?;

        Ok(())
    }

    async fn fees_and_publish(
        &self,
        to_pubkey: PublicKey,
        req_id: &str,
    ) -> Result<()> {
        info!("Estimating fees: req={}", req_id);

        let electrs = self.electrs_client.clone();

        let result = timeout(
            Duration::from_secs(30),
            electrs.estimate_fees(),
        )
        .await;

        let (fast, medium, slow) = match result {
            Ok(Ok((f, m, s))) => {
                info!("Fees OK: req={} fast={} medium={} slow={}", req_id, f, m, s);
                (f, m, s)
            }
            _ => {
                warn!("Fee estimation failed or timed out: req={}, using defaults", req_id);
                // Return reasonable defaults if Electrs fails
                (10, 5, 1)
            }
        };

        let response = GetFeesResponse {
            req: req_id.to_string(),
            fast,
            medium,
            slow,
        };

        let json = serde_json::to_string(&response)?;

        let tags = vec![
            Tag::parse(["p", to_pubkey.to_hex().as_str()])?,
            Tag::parse(["req", req_id])?,
        ];

        let event = EventBuilder::new(
            Kind::Custom(NOMAD_SERVER_RESPONSE_KIND),
            json,
        )
        .tags(tags)
        .sign_with_keys(&self.keys)?;

        self.client.send_event(&event).await?;

        Ok(())
    }

    async fn utxos_and_publish(
        &self,
        to_pubkey: PublicKey,
        req_id: &str,
        addresses: Vec<String>,
    ) -> Result<()> {
        info!("Fetching UTXOs: req={} addresses={}", req_id, addresses.len());

        let electrs = self.electrs_client.clone();
        let addrs = addresses.to_vec();

        // Respect the single-flight gate pattern
        let result = timeout(
            Duration::from_secs(45),
            electrs.get_utxos(&addrs),
        )
        .await;

        let utxos = match result {
            Ok(Ok(v)) => {
                info!("UTXOs OK: req={} count={}", req_id, v.len());
                v
            }
            Ok(Err(e)) => {
                warn!("UTXO fetch error: req={} err={}", req_id, e);
                vec![]
            }
            Err(_) => {
                warn!("UTXO fetch timeout: req={}", req_id);
                vec![]
            }
        };

        let response = GetUtxosResponse {
            req: req_id.to_string(),
            utxos,
        };

        let json = serde_json::to_string(&response)?;

        let tags = vec![
            Tag::parse(["p", to_pubkey.to_hex().as_str()])?,
            Tag::parse(["req", req_id])?,
        ];

        let event = EventBuilder::new(
            Kind::Custom(NOMAD_SERVER_RESPONSE_KIND),
            json,
        )
        .tags(tags)
        .sign_with_keys(&self.keys)?;

        self.client.send_event(&event).await?;

        Ok(())
    }
}

/* -------------------- Helpers -------------------- */

fn extract_req_id(event: &Event) -> Option<String> {
    for t in event.tags.iter() {
        let v = t.clone().to_vec();
        if v.len() >= 2 && v[0] == "req" {
            return Some(v[1].to_string());
        }
    }
    None
}
