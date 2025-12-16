use anyhow::{Context, Result};
use tracing::info;

use axum::{
    routing::get,
    Router,
    response::{IntoResponse, Response},
    http::{StatusCode, header},
};
use tokio::net::TcpListener;
use std::net::SocketAddr;

mod config;
mod error;
mod identity;
mod relays;
mod qr;

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

    let pubkey = identity.public_key_hex();
    let relays = relays::get_relays();

    let payload = qr::PairingPayload::new(pubkey, relays);
    let pairing_json = payload.to_json()?;
    let qr_svg = payload.generate_qr_svg()?;

    let pairing_json_clone = pairing_json.clone();
    let qr_svg_clone = qr_svg.clone();

    let app = Router::new()
        .route("/", get(|| async { "BalanceBridge is running" }))
        .route("/pairing", get(move || async move { pairing_json_clone.clone() }))
        .route("/qr", get(move || async move { serve_svg(qr_svg_clone.clone()) }));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3829));
    info!("Listening on http://{}", addr);

    let listener = TcpListener::bind(addr)
        .await
        .context("Failed to bind")?;

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
