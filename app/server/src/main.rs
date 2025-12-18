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
    let _ = default_provider().install_default();
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== BALANCEBRIDGE MAIN STARTED ===");

    install_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    println!("=== BALANCEBRIDGE BUILD MARKER: trace-timeout-v2 ===");

    info!("BalanceBridge Umbrel Server starting...");

    let data_dir = config::get_data_dir();
    info!("Using data dir: {}", data_dir.display());

    let keys = identity::load_or_create_keys();
    let pubkey = keys.public_key().to_hex();
    let relay_list = relays::get_relays();

    // ✅ Electrs MUST be initialized before Nostr handler
    info!("Initializing Electrs client...");
    let electrs_client = Arc::new(
        electrs::ElectrsClient::new()
            .context("Failed to initialize Electrs client")?
    );
    info!("Electrs client initialized successfully");
    info!("Warming up Electrs...");
    match electrs_client.warm_up() {
        Ok(_) => info!("Electrs warm-up successful"),
        Err(e) => warn!("Electrs warm-up failed: {}", e),
    }

    // Initialize pairing manager
    let pairing_manager = pairing::PairingManager::new(&data_dir)
        .context("Failed to init pairing manager")?;

    // Generate QR code for pairing
    let payload = qr::PairingPayload::new(pubkey.clone(), relay_list.clone());
    let pairing_json = payload.to_json()?;
    let qr_svg = payload.generate_qr_svg()?;

    let pairing_json_clone = pairing_json.clone();
    let qr_svg_clone = qr_svg.clone();

    // Start Nostr handler
    info!("Server pubkey: {}", pubkey);
    info!("BalanceBridge request kind: {}", crate::nostr_handler::BALANCEBRIDGE_REQUEST_KIND);
    info!("BalanceBridge response kind: {}", crate::nostr_handler::BALANCEBRIDGE_RESPONSE_KIND);
    info!("Nostr relays: {}", relay_list.join(", "));

    let nostr_task = tokio::spawn({
        let keys_clone = keys.clone();
        let pairing_manager_clone = pairing_manager.clone();
        let relay_list_clone = relay_list.clone();
        let electrs_client_clone = Arc::clone(&electrs_client);

        async move {
            match nostr_handler::NostrHandler::new(
                keys_clone,
                pairing_manager_clone,
                relay_list_clone,
                electrs_client_clone,
            )
            .await
            {
                Ok(handler) => {
                    if let Err(e) = handler.start_listening().await {
                        eprintln!("Nostr handler exited with error: {}", e);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to start Nostr handler: {}", e);
                }
            }
        }
    });

    tokio::spawn(async move {
        if let Err(e) = nostr_task.await {
            eprintln!("Nostr task panicked: {:?}", e);
        }
    });

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
                match tokio::task::spawn_blocking(move || electrs_client.test_connectivity()).await {
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
