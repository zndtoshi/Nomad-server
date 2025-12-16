//! Nostr event handler for Bitcoin lookup requests
//!
//! Listens for encrypted events from the paired Android app,
//! processes Bitcoin lookup requests, and sends responses.

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use tracing::{error, info, warn};

use crate::electrs::ElectrsClient;
use crate::pairing::PairingManager;
use crate::protocol::{BitcoinLookupRequest, BitcoinLookupResponse};
use crate::xpub::{derive_addresses, is_bitcoin_address, is_xpub};

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

        // Add all relays
        for relay_url in &relay_urls {
            let url = Url::parse(relay_url)
                .with_context(|| format!("Invalid relay URL: {}", relay_url))?;
            client.add_relay(url).await?;
            info!("Added relay: {}", relay_url);
        }

        // Connect to all relays
        client.connect().await;
        info!("Connected to {} relay(s)", relay_urls.len());

        let electrs_client = ElectrsClient::new()?;

        Ok(Self {
            client,
            keys,
            pairing_manager,
            electrs_client,
        })
    }

    /// Start listening for events from the paired Android app
    pub async fn start_listening(&self) -> Result<()> {
        let android_pubkey = match self.pairing_manager.get_android_pubkey()? {
            Some(pk) => pk,
            None => {
                warn!("No Android app paired yet, waiting for pairing...");
                // Still listen for pairing events
                self.listen_for_pairing().await?;
                return Ok(());
            }
        };

        info!("Listening for events from Android pubkey: {}", android_pubkey.to_hex());

        // Create filter for events from Android app (kind 30078, encrypted)
        let filter = Filter::new()
            .kinds(vec![Kind::Custom(30078)])
            .authors(vec![android_pubkey]);

        // Subscribe to events
        self.client.subscribe(filter, None).await?;

        // Get notification stream
        let mut notifications = self.client.notifications();

        // Process incoming events
        loop {
            match notifications.recv().await {
                Ok(notification) => {
                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            if event.pubkey == android_pubkey {
                                if let Err(e) = self.handle_event(*event).await {
                                    error!("Error handling event: {}", e);
                                }
                            } else {
                                warn!(
                                    "Received event from unexpected pubkey: {}",
                                    event.pubkey.to_hex()
                                );
                            }
                        }
                        RelayPoolNotification::Message { .. } => {
                            // Ignore other message types
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    error!("Error receiving notification: {}", e);
                    // Continue listening
                }
            }
        }
    }

    /// Listen for pairing events (before Android app is paired)
    async fn listen_for_pairing(&self) -> Result<()> {
        info!("Listening for pairing events...");

        // Listen for any kind 30078 events
        let filter = Filter::new().kinds(vec![Kind::Custom(30078)]);

        self.client.subscribe(filter, None).await?;

        let mut notifications = self.client.notifications();

        loop {
            match notifications.recv().await {
                Ok(notification) => {
                    match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            if let Err(e) = self.handle_pairing_event(*event).await {
                                error!("Error handling pairing event: {}", e);
                            }
                            // After pairing, we can break and start normal listening
                            if self.pairing_manager.has_pairing() {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    error!("Error receiving notification: {}", e);
                }
            }
        }
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

    /// Handle an incoming event from the paired Android app
    async fn handle_event(&self, event: Event) -> Result<()> {
        let pubkey_hex = event.pubkey.to_hex();
        let sender_pubkey_short = format!("{}...{}", 
            &pubkey_hex[0..8], 
            &pubkey_hex[pubkey_hex.len()-8..]);
        
        info!("Received event from Android app: {} (sender: {})", 
            event.id.to_hex(), sender_pubkey_short);

        // Decrypt the event content
        let decrypted = self
            .keys
            .nip44_decrypt(&event.pubkey, &event.content)
            .await
            .context("Failed to decrypt event")?;

        info!("Decrypted message: {}", decrypted);

        // Try to parse as Bitcoin lookup request
        let request: BitcoinLookupRequest = match serde_json::from_str(&decrypted) {
            Ok(req) => req,
            Err(e) => {
                warn!("Failed to parse request JSON from {}: {}", sender_pubkey_short, e);
                return Ok(());
            }
        };

        // Validate request
        if !request.is_valid() {
            warn!("Invalid Bitcoin lookup request from {}: {:?}", sender_pubkey_short, request);
            return Ok(());
        }

        // Determine query type
        let query_type = if crate::xpub::is_xpub(&request.query) {
            "xpub"
        } else if crate::xpub::is_bitcoin_address(&request.query) {
            "address"
        } else {
            "unknown"
        };

        info!("Processing Bitcoin lookup request from {}: type={}, query={}", 
            sender_pubkey_short, query_type, request.query);

        // Process the request
        let response = self.process_bitcoin_lookup(&request.query).await?;

        // Send response back
        self.send_response(event.pubkey, &response).await?;

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

            let (confirmed, unconfirmed, transactions) = self
                .electrs_client
                .get_addresses_balance_and_txs(&addresses)
                .await?;

            info!("Electrs query completed for xpub: confirmed={} sats, unconfirmed={} sats, tx_count={}", 
                confirmed, unconfirmed, transactions.len());

            response.confirmed_balance = confirmed;
            response.unconfirmed_balance = unconfirmed;
            response.transactions = transactions
                .into_iter()
                .map(|tx| crate::protocol::TransactionInfo {
                    txid: tx.txid,
                    timestamp: tx.timestamp,
                    amount: tx.amount,
                    confirmations: tx.confirmations,
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

            let transactions = self
                .electrs_client
                .get_address_transactions(query)
                .await?;

            info!("Electrs transaction query completed: tx_count={}", transactions.len());

            response.confirmed_balance = confirmed;
            response.unconfirmed_balance = unconfirmed;
            response.transactions = transactions
                .into_iter()
                .map(|tx| crate::protocol::TransactionInfo {
                    txid: tx.txid,
                    timestamp: tx.timestamp,
                    amount: tx.amount,
                    confirmations: tx.confirmations,
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
    async fn send_response(&self, recipient: PublicKey, response: &BitcoinLookupResponse) -> Result<()> {
        let recipient_short = format!("{}...{}", 
            &recipient.to_hex()[0..8], 
            &recipient.to_hex()[recipient.to_hex().len()-8..]);

        let response_json = serde_json::to_string(response)
            .context("Failed to serialize response")?;

        info!("Sending response to Android app ({}): confirmed={} sats, unconfirmed={} sats, tx_count={}", 
            recipient_short, response.confirmed_balance, response.unconfirmed_balance, response.transactions.len());

        // Encrypt the response
        let encrypted = match self.keys.nip44_encrypt(&recipient, &response_json).await {
            Ok(enc) => enc,
            Err(e) => {
                error!("Failed to encrypt response to {}: {}", recipient_short, e);
                return Err(anyhow::anyhow!("Encryption failed: {}", e));
            }
        };

        // Create unsigned event
        let unsigned = EventBuilder::new(Kind::Custom(30078), encrypted)
            .build(self.keys.public_key());

        // Sign the event
        let event = self.keys.sign_event(unsigned).await
            .context("Failed to sign event")?;

        match self.client.send_event(&event).await {
            Ok(_) => {
                info!("Response sent successfully to {}", recipient_short);
                Ok(())
            }
            Err(e) => {
                error!("Failed to send response to {}: {}", recipient_short, e);
                Err(anyhow::anyhow!("Failed to send event: {}", e))
            }
        }
    }
}

