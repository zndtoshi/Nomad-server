use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use nostr_sdk::{
    Alphabet, Client, Event, EventBuilder, Filter, Keys, Kind, PublicKey, RelayPoolNotification,
    SingleLetterTag, Tag,
};
use serde_json::Value;
use tokio::time::timeout;
use tokio::sync::broadcast;

use crate::electrs::ElectrsClient;

#[derive(Clone)]
pub struct NostrState {
    pub client: Arc<Client>,
}

impl NostrState {
    pub async fn new(keys: Keys, relays: Vec<String>) -> Result<Self> {
        // IMPORTANT: pass OWNED Keys, not &Keys
        let client = Client::new(keys);

        // nostr-sdk v0.44.1 API
        for relay in relays {
            client.add_relay(relay).await?;
        }

        // connect() returns ()
        client.connect().await;

        Ok(Self {
            client: Arc::new(client),
        })
    }
}

pub async fn run_nomadserver_nostr_loop(
    client: Arc<Client>,
    electrs: Arc<ElectrsClient>,
) -> Result<()> {
    client.wait_for_connection(Duration::from_secs(10)).await;

    // Our server pubkey (hex)
    let server_pk: PublicKey = client.public_key().await?;
    let server_pk_hex = server_pk.to_string();

    // Filter: only kind 30078 that p-tags THIS server pubkey
    let filter = Filter::new()
        .kind(Kind::Custom(30078))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::P), server_pk_hex.clone());

    client.subscribe(filter, None).await?;
    log::info!("NS_NOSTR: subscribed to kind=30078 p={}", server_pk_hex);

    // IMPORTANT: keep a receiver and do NOT crash on lag
    let mut notifications = client.notifications();

    loop {
        let notif = match notifications.recv().await {
            Ok(n) => n,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                log::warn!("NS_NOSTR: notifications lagged by {}; continuing", n);
                continue;
            }
            Err(e) => {
                return Err(anyhow::anyhow!("notifications recv failed: {e:?}"));
            }
        };

        if let RelayPoolNotification::Event { event, .. } = notif {
            if let Err(e) =
                handle_nomadserver_event(client.clone(), electrs.clone(), *event).await
            {
                log::error!("NS_NOSTR: handler error: {e:?}");
            }
        }
    }
}

async fn handle_nomadserver_event(
    client: Arc<Client>,
    electrs: Arc<ElectrsClient>,
    event: Event,
) -> Result<()> {
    // Parse JSON payload
    let payload: Value = serde_json::from_str(&event.content)
        .map_err(|e| anyhow!("invalid JSON content: {e}"))?;

    let typ = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if typ != "bitcoin_lookup" {
        return Ok(()); // ignore unrelated messages
    }

    let query = payload
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing query"))?;

    // Extract req id from tags: ["req", "<uuid>"]
    let mut req_id: Option<String> = None;
    for t in event.tags.iter() {
        let v = t.clone().to_vec();
        if v.len() >= 2 && v[0] == "req" {
            req_id = Some(v[1].clone());
            break;
        }
    }
    let req_id = req_id.ok_or_else(|| anyhow!("missing req tag"))?;

    log::info!(
        "NS_NOSTR: received request req={} from={} query={}",
        req_id,
        event.pubkey.to_string(),
        query
    );

    let response_json = build_response_json(electrs, query, &req_id).await?;

    // Publish response event kind 30079
    let mut tags: Vec<Tag> = Vec::new();
    tags.push(Tag::parse(vec!["req".to_string(), req_id.clone()])?);
    // Optional but recommended: p-tag back to requester
    tags.push(Tag::parse(vec!["p".to_string(), event.pubkey.to_string()])?);

    // Build EventBuilder (no explicit pubkey; client injects and signs)
    let builder = EventBuilder::new(Kind::Custom(30079), response_json)
        .tags(tags);

    // Sign using client-held keys
    let signed: Event = client.sign_event_builder(builder).await?;

    // Publish
    client.send_event(&signed).await?;

    log::info!("NS_NOSTR: published response req={} kind=30079", req_id);

    Ok(())
}

async fn build_response_json(
    electrs: Arc<ElectrsClient>,
    query: &str,
    req_id: &str,
) -> Result<String> {
    let (confirmed, unconfirmed) = timeout(
        Duration::from_secs(30),
        electrs.get_address_balance(query),
    )
    .await
    .map_err(|_| anyhow!("Electrs balance timeout"))??;

    let txids = match timeout(
        Duration::from_secs(20),
        electrs.get_address_txs(query),
    )
    .await
    {
        Ok(Ok(v)) => v,
        _ => Vec::new(),
    };

    let transactions: Vec<Value> = txids
        .iter()
        .map(|txid| {
            serde_json::json!({
                "txid": txid,
                "confirmations": 0,
                "amount": 0
            })
        })
        .collect();

    let resp = serde_json::json!({
        "req": req_id,
        "confirmedBalance": confirmed,
        "unconfirmedBalance": unconfirmed,
        "transactions": transactions
    });

    Ok(resp.to_string())
}

