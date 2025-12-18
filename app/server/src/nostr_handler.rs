use anyhow::{anyhow, Context, Result};
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::electrs::ElectrsClient;
use crate::pairing::PairingManager;
use crate::xpub;

pub const BALANCEBRIDGE_REQUEST_KIND: u16 = 30078;
pub const BALANCEBRIDGE_RESPONSE_KIND: u16 = 30079;

pub struct NostrHandler {
    keys: Keys,
    #[allow(dead_code)]
    pairing: PairingManager,
    relays: Vec<String>,
    electrs: Arc<ElectrsClient>,
}

#[derive(Debug, Deserialize)]
struct LookupRequest {
    #[serde(rename = "type")]
    typ: String,
    query: String,
}

#[derive(Debug, Serialize)]
struct LookupResult {
    confirmed_balance: u64,
    unconfirmed_balance: u64,
    transactions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LookupResponse {
    #[serde(rename = "type")]
    typ: String,
    status: String,
    req: String,
    result: Option<LookupResult>,
    error: Option<String>,
}

impl NostrHandler {
    pub async fn new(
        keys: Keys,
        pairing: PairingManager,
        relays: Vec<String>,
        electrs: Arc<ElectrsClient>,
    ) -> Result<Self> {
        Ok(Self {
            keys,
            pairing,
            relays,
            electrs,
        })
    }

    pub async fn start_listening(&self) -> Result<()> {
        info!("=== NOSTR HANDLER start_listening ===");

        let client = Client::new(self.keys.clone());

        for r in &self.relays {
            let url = RelayUrl::parse(r).with_context(|| format!("Invalid relay URL: {}", r))?;
            client.add_relay(url).await?;
        }

        client.connect().await;

        let filter = Filter::new().kind(Kind::Custom(BALANCEBRIDGE_REQUEST_KIND));
        client.subscribe(filter, None).await;

        info!(
            "Subscribed to kind={} on {} relay(s)",
            BALANCEBRIDGE_REQUEST_KIND,
            self.relays.len()
        );

        let mut notifications = client.notifications();

        loop {
            match notifications.recv().await {
                Ok(RelayPoolNotification::Event { event, .. }) => {
                    if let Err(e) = self.handle_event(&client, &event).await {
                        warn!("handle_event error: {}", e);
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("Nostr notifications error: {}", e),
            }
        }
    }

    async fn handle_event(&self, client: &Client, event: &Event) -> Result<()> {
        if event.kind != Kind::Custom(BALANCEBRIDGE_REQUEST_KIND) {
            return Ok(());
        }

        let server_pk_hex = self.keys.public_key().to_hex();
        if !has_tag(event, "p", &server_pk_hex) {
            return Ok(());
        }

        let req_id = match get_tag(event, "req") {
            Some(v) => v,
            None => return Ok(()),
        };

        let android_pubkey = event.pubkey;

        let req: LookupRequest = match serde_json::from_str(&event.content) {
            Ok(v) => v,
            Err(_) => {
                self.send_error(client, android_pubkey, req_id, "invalid_json".into())
                    .await?;
                return Ok(());
            }
        };

        if req.typ != "bitcoin_lookup" {
            self.send_error(client, android_pubkey, req_id, "invalid_type".into())
                .await?;
            return Ok(());
        }

        let query = req.query.trim().to_string();
        if query.is_empty() {
            self.send_error(client, android_pubkey, req_id, "empty_query".into())
                .await?;
            return Ok(());
        }

        info!(
            "Nostr lookup request: from={} req={} query={}",
            android_pubkey.to_hex(),
            req_id,
            query
        );

        // IMPORTANT: this confirms Electrs is finished before we publish.
        match self.perform_lookup(&query).await {
            Ok(result) => {
                info!(
                    "Lookup OK: req={} from={} confirmed={} unconfirmed={} txs={}",
                    req_id,
                    android_pubkey.to_hex(),
                    result.confirmed_balance,
                    result.unconfirmed_balance,
                    result.transactions.len()
                );
                self.send_ok(client, android_pubkey, req_id, result).await?
            }
            Err(e) => {
                error!("Lookup failed: req={} err={}", req_id, e);
                self.send_error(client, android_pubkey, req_id, e.to_string())
                    .await?;
            }
        }

        Ok(())
    }

    async fn perform_lookup(&self, query: &str) -> Result<LookupResult> {
        if xpub::is_xpub(query) {
            let addresses =
                xpub::derive_addresses(query, 20).context("Failed to derive addresses from xpub")?;

            let mut confirmed = 0u64;
            let mut unconfirmed = 0u64;
            let mut txids: HashSet<String> = HashSet::new();

            for addr in addresses {
                if let Ok((c, u)) = self.electrs.get_address_balance(&addr).await {
                    confirmed = confirmed.saturating_add(c);
                    unconfirmed = unconfirmed.saturating_add(u);
                }

                if let Ok(txs) = self.electrs.get_address_txs(&addr).await {
                    for t in txs {
                        txids.insert(t);
                    }
                }
            }

            let mut tx_list: Vec<String> = txids.into_iter().collect();
            tx_list.sort();

            Ok(LookupResult {
                confirmed_balance: confirmed,
                unconfirmed_balance: unconfirmed,
                transactions: tx_list,
            })
        } else if xpub::is_bitcoin_address(query) {
            let (confirmed, unconfirmed) = self.electrs.get_address_balance(query).await?;

            Ok(LookupResult {
                confirmed_balance: confirmed,
                unconfirmed_balance: unconfirmed,
                transactions: Vec::new(),
            })
        } else {
            Err(anyhow!("invalid_query"))
        }
    }

    async fn send_ok(
        &self,
        client: &Client,
        android_pubkey: PublicKey,
        req_id: String,
        result: LookupResult,
    ) -> Result<()> {
        let resp = LookupResponse {
            typ: "bitcoin_lookup_response".into(),
            status: "ok".into(),
            req: req_id.clone(),
            result: Some(result),
            error: None,
        };

        self.publish_response(client, android_pubkey, req_id, resp).await
    }

    async fn send_error(
        &self,
        client: &Client,
        android_pubkey: PublicKey,
        req_id: String,
        msg: String,
    ) -> Result<()> {
        let resp = LookupResponse {
            typ: "bitcoin_lookup_response".into(),
            status: "error".into(),
            req: req_id.clone(),
            result: None,
            error: Some(msg),
        };

        self.publish_response(client, android_pubkey, req_id, resp).await
    }

    async fn publish_response(
        &self,
        client: &Client,
        android_pubkey: PublicKey,
        req_id: String,
        resp: LookupResponse,
    ) -> Result<()> {
        let content = serde_json::to_string(&resp).context("Failed to serialize response")?;

        let server_pk_hex = self.keys.public_key().to_hex();

        // We include BOTH "p" tags to satisfy either client filtering style:
        // - Some clients filter responses by their own pubkey (recipient-style)
        // - Some mirror the request pattern and filter by server pubkey
        let p_tag_android = Tag::parse(vec!["p".to_string(), android_pubkey.to_hex()])?;
        let p_tag_server = Tag::parse(vec!["p".to_string(), server_pk_hex.clone()])?;
        let req_tag = Tag::parse(vec!["req".to_string(), req_id.clone()])?;

        let builder = EventBuilder::new(Kind::Custom(BALANCEBRIDGE_RESPONSE_KIND), content)
            .tags(vec![p_tag_android, p_tag_server, req_tag]);

        info!(
            "Publishing response: kind={} to={} req={} (server_pk={})",
            BALANCEBRIDGE_RESPONSE_KIND,
            android_pubkey.to_hex(),
            req_id,
            server_pk_hex
        );

        // Log the returned event id so we know publish actually happened.
        let event_id = client.send_event_builder(builder).await?;

        info!(
            "Published response OK: event_id={} req={} to={}",
            event_id.to_hex(),
            req_id,
            android_pubkey.to_hex()
        );

        Ok(())
    }
}

fn has_tag(event: &Event, key: &str, value: &str) -> bool {
    for t in event.tags.iter() {
        let v = t.clone().to_vec();
        if v.len() >= 2 && v[0] == key && v[1] == value {
            return true;
        }
    }
    false
}

fn get_tag(event: &Event, key: &str) -> Option<String> {
    for t in event.tags.iter() {
        let v = t.clone().to_vec();
        if v.len() >= 2 && v[0] == key {
            return Some(v[1].clone());
        }
    }
    None
}
