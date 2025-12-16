use anyhow::{Context, Result};
use tracing::{error, info, warn};

use axum::{
    routing::get,
    Router,
    response::{IntoResponse, Response},
    http::{StatusCode, header},
};
use tokio::net::TcpListener;
use std::net::SocketAddr;
use reqwest::Client as HttpClient;

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

#[tokio::main]
async fn main() -> Result<()> {
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
    let payload = qr::PairingPayload::new(pubkey, relay_list.clone());
    let pairing_json = payload.to_json()?;
    let qr_svg = payload.generate_qr_svg()?;

    let pairing_json_clone = pairing_json.clone();
    let qr_svg_clone = qr_svg.clone();

    // Start Nostr handler in background task
    let pairing_manager_clone = pairing_manager.clone();
    let keys_clone = keys.clone();
    let relay_list_clone = relay_list.clone();
    tokio::spawn(async move {
        info!("Starting Nostr handler...");
        match nostr_handler::NostrHandler::new(
            keys_clone,
            pairing_manager_clone,
            relay_list_clone,
        )
        .await
        {
            Ok(handler) => {
                if let Err(e) = handler.start_listening().await {
                    error!("Nostr handler error: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to start Nostr handler: {}", e);
            }
        }
    });

    // HTTP server for pairing QR code
    let electrs_url = std::env::var("ELECTRS_URL")
        .unwrap_or_else(|_| "http://electrs:3002".to_string());
    
    let app = Router::new()
        .route("/", get(|| async { "BalanceBridge is running" }))
        .route("/pairing", get(move || async move { pairing_json_clone.clone() }))
        .route("/qr", get(move || async move { serve_svg(qr_svg_clone.clone()) }))
        .route("/health", get(move || async move {
            // Health check endpoint that also tests Electrs connectivity
            match test_electrs_connectivity(&electrs_url).await {
                Ok(_) => (StatusCode::OK, "OK - Electrs reachable").into_response(),
                Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("Electrs unreachable: {}", e)).into_response(),
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

/// Test Electrs connectivity
async fn test_electrs_connectivity(electrs_url: &str) -> Result<()> {
    let client = HttpClient::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to create HTTP client")?;

    // Try to reach Electrs root endpoint
    let test_url = format!("{}/", electrs_url);
    let response = client
        .get(&test_url)
        .send()
        .await
        .context("Failed to connect to Electrs")?;

    if response.status().is_success() || response.status().as_u16() == 404 {
        // 404 is OK - means Electrs is reachable but endpoint doesn't exist
        Ok(())
    } else {
        Err(anyhow::anyhow!("Electrs returned status: {}", response.status()))
    }
}
