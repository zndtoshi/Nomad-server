//! Nostr event handler for Bitcoin lookup requests
//!
//! Listens for encrypted events from the paired Android app,
//! processes Bitcoin lookup requests, and sends responses.

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

use crate::electrs::ElectrsClient;
use crate::pairing::PairingManager;
use crate::protocol::BitcoinLookupResponse;
use crate::xpub::{derive_addresses, is_bitcoin_address, is_xpub};

// BalanceBridge event kind (must match Android app)
// We use the SAME kind in both directions and distinguish by author pubkey.
pub const BALANCEBRIDGE_REQUEST_KIND: u16 = 30078;
pub const BALANCEBRIDGE_RESPONSE_KIND: u16 = 30079;

impl NostrHandler {
    /// Extract request ID from event tags, or generate one deterministically
    fn extract_request_id(event: &Event) -> String {
        // Look for ["req", "<request_id>"] tag
        for tag in event.tags.iter() {
            if tag.kind() == TagKind::Custom("req".into()) {
                if let Some(value) = tag.as_slice().get(1) {
                    return value.to_string();
                }
            }
        }

        // Fallback: generate deterministic ID from event ID
        format!("req_{}", &event.id.to_hex()[0..16])
    }

    /// Check if event is a valid BalanceBridge request for this server
    fn is_balancebridge_request(event: &Event, server_pubkey: &PublicKey) -> Option<String> {
        let mut req_id: Option<String> = None;
        let mut p_match = false;

        for tag in event.tags.iter() {
            let tag_vec = tag.clone().to_vec();

            if tag_vec.len() < 2 {
                continue;
            }

            match tag_vec[0].as_str() {
                "req" => {
                    req_id = Some(tag_vec[1].clone());
                }
                "p" => {
                    if tag_vec[1] == server_pubkey.to_hex() {
                        p_match = true;
                    }
                }
                _ => {}
            }
        }

        if p_match {
            req_id
        } else {
            None
        }
    }
}

/// Handles Nostr communication with Android app
pub struct NostrHandler {
    client: Client,
    keys: Keys,
    pairing_manager: PairingManager,
    electrs_client: ElectrsClient,
}

impl NostrHandler {
    /// Create a new Nostr handler
    pub async fn new(
        keys: Keys,
        pairing_manager: PairingManager,
        relay_urls: Vec<String>,
    ) -> Result<Self> {
        let client = Client::new(keys.clone());

        // Add all relays (continue on failure for redundancy)
        let mut added_count = 0;
        for relay_url in &relay_urls {
            match Url::parse(relay_url) {
                Ok(url) => {
                    match client.add_relay(url).await {
                        Ok(_) => {
                            info!("Relay connected: {}", relay_url);
                            added_count += 1;
                        }
                        Err(e) => {
                            warn!("Relay failed: {} - {}", relay_url, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Invalid relay URL {}: {}", relay_url, e);
                }
            }
        }

        if added_count == 0 {
            return Err(anyhow::anyhow!("Failed to add any relays"));
        }

        // Connect to all relays and wait for completion
        client.connect().await;
        println!("=== CLIENT CONNECT() CALLED ===");
        info!("Nostr relays: {}", relay_urls.join(", "));
        info!("Connected to {} relay(s)", added_count);

        let electrs_client = match ElectrsClient::new().await {
            Ok(c) => {
                info!("Electrs client connected");
                c
            }
            Err(e) => {
                error!("Electrs unavailable at startup: {}", e);
                return Err(e);
            }
        };

        let handler = Self {
            client,
            keys,
            pairing_manager,
            electrs_client,
        };

        handler.subscribe_requests().await?;

        Ok(handler)
    }

    async fn subscribe_requests(&self) -> Result<()> {
        let ten_minutes_ago = Timestamp::now().as_secs().saturating_sub(600);

        // Broad debug subscription: any kind=30078 from last 10 minutes
        let filter = Filter::new()
            .kinds(vec![Kind::Custom(BALANCEBRIDGE_REQUEST_KIND)])
            .since(Timestamp::from(ten_minutes_ago));

        println!("=== SUBSCRIBE_REQUESTS CALLED ===");
        println!("=== SUBSCRIBING kind={} since={} ===", BALANCEBRIDGE_REQUEST_KIND, ten_minutes_ago);

        self.client.subscribe(filter, None).await?;

        println!("=== SUBSCRIBE_REQUESTS DONE ===");
        Ok(())
    }

    /// Start listening for events from the paired Android app
    pub async fn start_listening(&self) -> Result<()> {
        println!("=== START_LISTENING ENTERED ===");

        // Get notification stream (subscription already active from startup)
        let mut notifications = self.client.notifications();

        // Get paired Android pubkey (if any)
        let android_pubkey = self.pairing_manager.get_android_pubkey()?;

        if let Some(ref pk) = android_pubkey {
            info!("Listening for events from Android pubkey: {}", pk.to_hex());
        } else {
            warn!("No Android app paired yet, accepting all kind 30078 events for pairing...");
        }

        // Process incoming events with reconnection handling
        loop {
            match notifications.recv().await {
                Ok(notification) => match notification {
                    RelayPoolNotification::Event { event, .. } => {
                        println!(
                            "=== NOTIF EVENT RECEIVED === kind={:?} pubkey={} id={} created_at={}",
                            event.kind,
                            event.pubkey.to_hex(),
                            event.id.to_hex(),
                            event.created_at.as_secs()
                        );
                        println!("=== TAGS === {:?}", event.tags);

                        if event.kind == Kind::Custom(BALANCEBRIDGE_REQUEST_KIND) {
                            if let Some(req_id) = Self::is_balancebridge_request(&event, &self.keys.public_key()) {
                                println!("=== VALID BALANCEBRIDGE REQUEST === req_id={}", req_id);
                                if let Err(e) = self.handle_event(*event).await {
                                    eprintln!("=== HANDLE_EVENT ERROR === {}", e);
                                }
                            } else {
                                // Ignore spam
                                println!("--- ignored kind=30078 (not a BalanceBridge request)");
                            }
                        }
                    }
                    RelayPoolNotification::Message { .. } => {
                        // Ignore other message types
                    }
                    RelayPoolNotification::Shutdown => {
                        warn!("Relay pool shutdown (will reconnect automatically)");
                    }
                },
                Err(e) => {
                    warn!("Error receiving notification: {}", e);
                    // Continue listening - do not exit loop
                }
            }
        }
    }

    /// Check if event is within expiration tolerance (60 seconds)
    /// Returns Ok(()) if valid, logs warning if expired but doesn't reject
    fn check_event_expiration(&self, event: &Event) -> Result<()> {
        let event_time = event.created_at.as_secs();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time error")?
            .as_secs();
        
        let delta = if now > event_time {
            now - event_time
        } else {
            0
        };
        
        if delta > 60 {
            warn!(
                "Event is {} seconds old (max 60s), but processing anyway",
                delta
            );
        } else {
            info!(
                "Event timestamp check: created_at={}, now={}, delta={}s",
                event_time, now, delta
            );
        }

        Ok(())
    }


    /// Handle a pairing event ("hello / paired")
    async fn handle_pairing_event(&self, event: Event) -> Result<()> {
        // Try to decrypt the event
        let decrypted = match self.keys.nip44_decrypt(&event.pubkey, &event.content).await {
            Ok(msg) => msg,
            Err(e) => {
                warn!("Failed to decrypt pairing event: {}", e);
                return Ok(());
            }
        };

        if decrypted == "hello / paired" {
            info!("Received pairing from: {}", event.pubkey.to_hex());

            // Get relays from the event tags (if available)
            // For now, use default relays
            let relays = self.pairing_manager.get_relays()
                .unwrap_or_else(|_| vec!["wss://relay.damus.io".to_string()]);

            // Store the pairing
            self.pairing_manager.store_pairing(event.pubkey, relays)?;

            info!("Android app paired successfully");
        }

        Ok(())
    }

    /// Handle an incoming BalanceBridge request event
    async fn handle_event(&self, event: Event) -> Result<()> {
        info!("=== HANDLE_EVENT ENTERED ===");
        info!(
            "Incoming event id={} pubkey={} kind={:?}",
            event.id,
            event.pubkey,
            event.kind
        );

        // --- Extract req tag ---
        let mut req_id: Option<String> = None;

        for tag in event.tags.iter() {
            let parts = tag.clone().to_vec();
            if parts.len() >= 2 && parts[0] == "req" {
                req_id = Some(parts[1].clone());
            }
        }

        let req_id = match req_id {
            Some(r) => r,
            None => {
                warn!("No req tag found — aborting");
                return Ok(());
            }
        };

        info!("=== BALANCEBRIDGE REQUEST ACCEPTED === req_id={}", req_id);
        info!(
            "=== STEP 1: WILL SEND RESPONSE === req_id={} client_pubkey={}",
            req_id,
            event.pubkey
        );
        info!("Event content: {}", event.content);

        // --- Parse request JSON ---
        let parsed: serde_json::Value = match serde_json::from_str(&event.content) {
            Ok(v) => v,
            Err(e) => {
                error!("JSON parse failed: {}", e);
                return Ok(());
            }
        };

        let request_type = parsed
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        info!("Request type = {}", request_type);

        // Extract query from request
        let query = parsed
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        println!(
            "=== STEP 1: LOOKUP START === req_id={} query={}",
            req_id,
            query
        );

        // --- Perform Bitcoin lookup ---
        let lookup_result = self.process_bitcoin_lookup(query).await;

        // --- Build response payload ---
        let response_json = match lookup_result {
            Ok(result) => {
                println!(
                    "=== STEP 1: LOOKUP DONE === req_id={} tx_count={}",
                    req_id,
                    result.transactions.len()
                );

                serde_json::json!({
                    "type": "bitcoin_lookup_response",
                    "req": req_id,
                    "status": "ok",
                    "result": {
                        "query": result.query,
                        "confirmed_balance": result.confirmed_balance,
                        "unconfirmed_balance": result.unconfirmed_balance,
                        "transactions": result.transactions
                    }
                })
            }
            Err(e) => {
                println!(
                    "=== STEP 1: LOOKUP ERROR === req_id={} error={:?}",
                    req_id,
                    e
                );

                serde_json::json!({
                    "type": "bitcoin_lookup_response",
                    "req": req_id,
                    "status": "error",
                    "error": e.to_string()
                })
            }
        };

        println!(
            "=== STEP 2: RESPONSE BUILT === req_id={} payload_size={}",
            req_id,
            response_json.to_string().len()
        );

        info!("Publishing response payload: {}", response_json);

        // --- Build response event ---
        let mut builder = EventBuilder::new(
            Kind::Custom(BALANCEBRIDGE_RESPONSE_KIND),
            response_json.to_string(),
        );

        // add tags explicitly (nostr-sdk ≥0.44 style)
        builder = builder
            .tag(Tag::parse(vec![
                "p".to_string(),
                event.pubkey.to_string(),
            ])?)
            .tag(Tag::parse(vec![
                "req".to_string(),
                req_id.clone(),
            ])?);

        // sign event
        let response_event = builder
            .sign_with_keys(&self.keys)?;

        info!(
            "Response event built: id={} author={}",
            response_event.id,
            response_event.pubkey
        );

        println!(
            "=== RESPONSE BUILT === req_id={} kind={:?} pubkey={} tags={:?}",
            req_id,
            response_event.kind,
            response_event.pubkey,
            response_event.tags
        );

        // --- Publish response ---
        println!("=== STEP 2: SENDING RESPONSE === req_id={}", req_id);
        match self.client.send_event(&response_event).await {
            Ok(_) => {
                info!("=== RESPONSE EVENT PUBLISHED SUCCESSFULLY ===");
                println!(
                    "=== RESPONSE SENT === req_id={} event_id={}",
                    req_id,
                    response_event.id
                );
            }
            Err(e) => {
                error!("FAILED TO PUBLISH RESPONSE EVENT: {}", e);
                println!(
                    "=== RESPONSE SEND ERROR === req_id={} err={:?}",
                    req_id,
                    e
                );
            }
        }

        Ok(())
    }

    /// Process a Bitcoin lookup request
    async fn process_bitcoin_lookup(&self, query: &str) -> Result<BitcoinLookupResponse> {
        let mut response = BitcoinLookupResponse::new(query.to_string());

        if is_xpub(query) {
            // Handle xpub/ypub/zpub/tpub
            info!("Processing xpub query: {}", query);
            let addresses = derive_addresses(query, 20)?;
            info!("Derived {} addresses from xpub (gap_limit=20)", addresses.len());

            let mut total_confirmed = 0u64;
            let mut total_unconfirmed = 0u64;
            let mut all_txids = Vec::new();

            for address in addresses {
                match self.electrs_client.get_address_balance(&address).await {
                    Ok((confirmed, unconfirmed)) => {
                        total_confirmed += confirmed;
                        total_unconfirmed += unconfirmed;
                    }
                    Err(e) => {
                        warn!("Failed to get balance for derived address {}: {}", address, e);
                    }
                }

                match self.electrs_client.get_address_txs(&address).await {
                    Ok(txids) => {
                        all_txids.extend(txids);
                    }
                    Err(e) => {
                        warn!("Failed to get transactions for derived address {}: {}", address, e);
                    }
                }
            }

            // Deduplicate txids
            all_txids.sort();
            all_txids.dedup();

            info!("Electrs query completed for xpub: confirmed={} sats, unconfirmed={} sats, tx_count={}",
                total_confirmed, total_unconfirmed, all_txids.len());

            response.confirmed_balance = total_confirmed as i64;
            response.unconfirmed_balance = total_unconfirmed as i64;
            response.transactions = all_txids
                .into_iter()
                .map(|txid| crate::protocol::TransactionInfo {
                    txid,
                    timestamp: 0, // Electrum doesn't provide timestamps
                    amount: 0,    // Electrum doesn't provide amounts in history
                    confirmations: 1, // Assume confirmed if in history
                })
                .collect();
        } else if is_bitcoin_address(query) {
            // Handle single Bitcoin address
            info!("Processing single address query: {}", query);
            let (confirmed, unconfirmed) = self
                .electrs_client
                .get_address_balance(query)
                .await?;

            info!("Electrs balance query completed: confirmed={} sats, unconfirmed={} sats", 
                confirmed, unconfirmed);

            let txids: Vec<String> = self
                .electrs_client
                .get_address_txs(query)
                .await?;

            info!("Electrs transaction query completed: tx_count={}", txids.len());

            response.confirmed_balance = confirmed as i64;
            response.unconfirmed_balance = unconfirmed as i64;
            response.transactions = txids
                .into_iter()
                .map(|txid| crate::protocol::TransactionInfo {
                    txid,
                    timestamp: 0, // Electrum doesn't provide timestamps
                    amount: 0,    // Electrum doesn't provide amounts in history
                    confirmations: 1, // Assume confirmed if in history
                })
                .collect();
        } else {
            return Err(anyhow::anyhow!("Invalid query: not an address or xpub"));
        }

        info!(
            "Bitcoin lookup result: confirmed={}, unconfirmed={}, tx_count={}",
            response.confirmed_balance,
            response.unconfirmed_balance,
            response.transactions.len()
        );

        Ok(response)
    }

    /// Send a response back to the Android app
    async fn send_response(&self, recipient: PublicKey, response: &BitcoinLookupResponse, request_id: &str) -> Result<()> {
        let recipient_short = format!("{}...{}",
            &recipient.to_hex()[0..8],
            &recipient.to_hex()[recipient.to_hex().len()-8..]);

        let response_json = serde_json::to_string(response)
            .context("Failed to serialize response")?;

        info!("Sending BalanceBridge response to {} (req_id={}): confirmed={} sats, unconfirmed={} sats, tx_count={}",
            recipient_short, request_id, response.confirmed_balance, response.unconfirmed_balance, response.transactions.len());

        // Encrypt the response
        let encrypted = match self.keys.nip44_encrypt(&recipient, &response_json).await {
            Ok(enc) => enc,
            Err(e) => {
                error!("Failed to encrypt response to {}: {}", recipient_short, e);
                return Err(anyhow::anyhow!("Encryption failed: {}", e));
            }
        };

        // Create unsigned event with proper tags
        let unsigned = EventBuilder::new(Kind::Custom(BALANCEBRIDGE_RESPONSE_KIND), encrypted)
            .tags(vec![
                Tag::public_key(recipient),  // "p" tag with recipient pubkey
                Tag::parse(["req", request_id])?, // "req" tag
            ])
            .build(self.keys.public_key());

        // Sign the event
        let event = self.keys.sign_event(unsigned).await
            .context("Failed to sign event")?;

        match self.client.send_event(&event).await {
            Ok(_) => {
                info!("Published BalanceBridge response: req_id={}, client_pubkey={}", request_id, recipient_short);
                Ok(())
            }
            Err(e) => {
                error!("Failed to publish BalanceBridge response to {}: {}", recipient_short, e);
                Err(anyhow::anyhow!("Failed to send event: {}", e))
            }
        }
    }
}

