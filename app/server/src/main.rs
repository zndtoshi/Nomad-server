use anyhow::{Context, Result};
use rustls::crypto::ring::default_provider;
use tracing::{error, info, warn};

use axum::{
    routing::get,
    Router,
    response::{IntoResponse, Response},
    http::{StatusCode, header},
    Json,
};
use tokio::net::TcpListener;
use std::net::SocketAddr;
use std::sync::Arc;

mod config;
mod error;
mod identity;
mod relays;
mod qr;
mod protocol;
mod pairing;
mod nostr_handler;
mod electrs;
mod xpub;

fn install_crypto_provider() {
    // Safe to call once; ignore error if already installed
    let _ = default_provider().install_default();
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== BALANCEBRIDGE MAIN STARTED ===");

    install_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("BalanceBridge Umbrel Server starting...");

    let data_dir = config::get_data_dir();
    info!("Using data dir: {}", data_dir.display());

    let identity = identity::IdentityManager::new(&data_dir)
        .context("Failed to init identity")?;

    let keys = identity.keys().clone();
    let pubkey = identity.public_key_hex();
    let relay_list = relays::get_relays();

    // Initialize pairing manager
    let pairing_manager = pairing::PairingManager::new(&data_dir)
        .context("Failed to init pairing manager")?;

    // Generate QR code for pairing
    let payload = qr::PairingPayload::new(pubkey.clone(), relay_list.clone());
    let pairing_json = payload.to_json()?;
    let qr_svg = payload.generate_qr_svg()?;

    let pairing_json_clone = pairing_json.clone();
    let qr_svg_clone = qr_svg.clone();

    // Start Nostr handler in background task
    info!("Server pubkey: {}", pubkey);
    info!("BalanceBridge request kind: {}", crate::nostr_handler::BALANCEBRIDGE_REQUEST_KIND);
    info!("BalanceBridge response kind: {}", crate::nostr_handler::BALANCEBRIDGE_RESPONSE_KIND);
    info!("Nostr relays: {}", relay_list.join(", "));

    let pairing_manager_clone = pairing_manager.clone();
    let keys_clone = keys.clone();
    let relay_list_clone = relay_list.clone();
    info!("Spawning Nostr handler task");

    let nostr_task = tokio::spawn(async move {
        println!("=== NOSTR TASK STARTED ===");

        match nostr_handler::NostrHandler::new(
            keys_clone,
            pairing_manager_clone,
            relay_list_clone,
        )
        .await
        {
            Ok(handler) => {
                println!("=== NOSTR HANDLER INITIALIZED ===");
                if let Err(e) = handler.start_listening().await {
                    eprintln!("Nostr handler exited with error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to start Nostr handler: {}", e);
            }
        }
    });

    tokio::spawn(async move {
        if let Err(e) = nostr_task.await {
            eprintln!("Nostr task panicked: {:?}", e);
        }
    });

    // Initialize Electrs client
    info!("Initializing Electrs client...");
    let electrs_client = Arc::new(electrs::ElectrsClient::new().await
        .context("Failed to initialize Electrs client")?);
    info!("Electrs client initialized successfully");

    let electrs_client_health = Arc::clone(&electrs_client);

    let app = Router::new()
        .route("/", get(|| async { "BalanceBridge is running" }))
        .route("/pairing", get(move || async move { pairing_json_clone.clone() }))
        .route("/qr", get(move || async move { serve_svg(qr_svg_clone.clone()) }))
        .route("/health", get(|| async {
            info!("HTTP GET /health request received");
            (StatusCode::OK, "OK").into_response()
        }))
        .route("/health/electrs", get(move || {
            let electrs_client = Arc::clone(&electrs_client_health);
            async move {
                info!("HTTP GET /health/electrs request received");
                match electrs_client.test_connectivity().await {
                    Ok(_) => (StatusCode::OK, "OK"),
                    Err(e) => {
                        error!("Electrs health check failed: {}", e);
                        (StatusCode::SERVICE_UNAVAILABLE, "Electrs unavailable")
                    }
                }
            }
        }));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3829));
    info!("Listening on http://{}", addr);

    let listener = TcpListener::bind(addr)
        .await
        .context("Failed to bind")?;

    info!("Server ready. Waiting for Android app pairing...");
    axum::serve(listener, app).await?;
    Ok(())
}

fn serve_svg(svg: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        svg,
    )
        .into_response()
}


/// Handle Bitcoin lookup HTTP request with shared Electrs client
async fn handle_bitcoin_lookup_with_client(
    request: protocol::BitcoinLookupRequest,
    electrs_client: &electrs::ElectrsClient,
) -> Response {
    info!("Processing Bitcoin lookup request: query={}", request.query);

    // Validate request
    if !request.is_valid() {
        warn!("Invalid Bitcoin lookup request: type={}, query={}",
            request.request_type, request.query);
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid request: type must be 'bitcoin_lookup' and query must not be empty"
            })),
        )
            .into_response();
    }

    if request.query.is_empty() {
        warn!("Empty query in Bitcoin lookup request");
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Query cannot be empty"
            })),
        )
            .into_response();
    }

    // Process the query with the shared client
    let response = match process_bitcoin_lookup(&request.query, electrs_client).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Bitcoin lookup failed for query '{}': {}", request.query, e);
            error!("Electrs connection failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "electrs_unreachable",
                    "details": e.to_string()
                })),
            )
                .into_response();
        }
    };

    info!(
        "Bitcoin lookup completed: query={}, confirmed={} sats, unconfirmed={} sats, tx_count={}",
        request.query,
        response.confirmed_balance,
        response.unconfirmed_balance,
        response.transactions.len()
    );

    let response_json = serde_json::to_string(&response)
        .unwrap_or_else(|_| "{}".to_string());
    info!("Sending HTTP response: size={} bytes", response_json.len());

    (StatusCode::OK, Json(response)).into_response()
}

/// Process a Bitcoin lookup request (using Electrum TCP protocol)
async fn process_bitcoin_lookup(
    query: &str,
    electrs_client: &electrs::ElectrsClient,
) -> Result<protocol::BitcoinLookupResponse> {
    let mut response = protocol::BitcoinLookupResponse::new(query.to_string());

    if xpub::is_xpub(query) {
        // Handle xpub/ypub/zpub/tpub - derive addresses and aggregate
        info!("Processing xpub query: {}", query);
        let addresses = xpub::derive_addresses(query, 20)
            .context("Failed to derive addresses from xpub")?;
        info!("Derived {} addresses from xpub (gap_limit=20)", addresses.len());

        let mut total_balance = 0u64;
        let mut all_txids = Vec::new();

        for address in addresses {
            match electrs_client.get_address_balance(&address).await {
                Ok((confirmed, unconfirmed)) => {
                    total_balance += confirmed + unconfirmed;
                }
                Err(e) => {
                    warn!("Failed to get balance for derived address {}: {}", address, e);
                }
            }

            match electrs_client.get_address_txs(&address).await {
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

        info!(
            "Electrs query completed for xpub: total_balance={} sats, tx_count={}",
            total_balance, all_txids.len()
        );

        response.confirmed_balance = total_balance as i64;
        response.unconfirmed_balance = 0; // Electrum doesn't distinguish confirmed/unconfirmed
        response.transactions = all_txids
            .into_iter()
            .map(|txid| protocol::TransactionInfo {
                txid,
                timestamp: 0, // Electrum doesn't provide timestamps
                amount: 0,    // Electrum doesn't provide amounts in history
                confirmations: 1, // Assume confirmed if in history
            })
            .collect();
    } else if xpub::is_bitcoin_address(query) {
        // Handle single Bitcoin address
        info!("Processing single address query: {}", query);

        let (confirmed, unconfirmed) = electrs_client
            .get_address_balance(query)
            .await
            .context("Failed to query Electrs for address balance")?;

        let txids = electrs_client
            .get_address_txs(query)
            .await
            .context("Failed to query Electrs for address transactions")?;

        info!(
            "Electrs query completed: confirmed={} sats, unconfirmed={} sats, tx_count={}",
            confirmed, unconfirmed, txids.len()
        );

        response.confirmed_balance = confirmed as i64;
        response.unconfirmed_balance = unconfirmed as i64;
        response.transactions = txids
            .into_iter()
            .map(|txid| protocol::TransactionInfo {
                txid,
                timestamp: 0, // Electrum doesn't provide timestamps
                amount: 0,    // Electrum doesn't provide amounts in history
                confirmations: 1, // Assume confirmed if in history
            })
            .collect();
    } else {
        return Err(anyhow::anyhow!("Invalid query: not an address or xpub"));
    }

    Ok(response)
}
