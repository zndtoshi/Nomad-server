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
mod nostr;
mod electrs;
mod xpub;

fn install_crypto_provider() {
    let _ = default_provider().install_default();
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("=== NOMAD_SERVER MAIN STARTED ===");

    install_crypto_provider();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    println!("=== NOMAD_SERVER BUILD MARKER: trace-timeout-v2 ===");

    info!("NomadServer starting...");

    let data_dir = config::get_data_dir();
    info!("Using data dir: {}", data_dir.display());

    let keys = identity::load_or_create_keys();
    let pubkey = keys.public_key().to_hex();
    let relay_list = relays::get_relays();
    let nostr_state = nostr::NostrState::new(keys.clone(), relay_list.clone()).await?;

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
    let pubkey_clone = pubkey.clone();
    let relay_list_clone = relay_list.clone();

    // Spawn lightweight NomadServer Nostr loop (request/response)
    {
        let client = nostr_state.client.clone();
        let electrs_for_nostr = Arc::clone(&electrs_client);
        tokio::spawn(async move {
            loop {
                if let Err(e) =
                    crate::nostr::run_nomadserver_nostr_loop(client.clone(), electrs_for_nostr.clone()).await
                {
                    error!("NS_NOSTR: loop crashed: {e:?} — restarting in 2s");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        });
    }

    // Start Nostr handler
    info!("Server pubkey: {}", pubkey);
    info!("NomadServer request kind: {}", crate::nostr_handler::NOMAD_SERVER_REQUEST_KIND);
    info!("NomadServer response kind: {}", crate::nostr_handler::NOMAD_SERVER_RESPONSE_KIND);
    info!("Nostr relays: {}", relay_list.join(", "));

    let nostr_task = tokio::spawn({
        let keys_clone = keys.clone();
        let pairing_manager_clone = pairing_manager.clone();
        let electrs_client_clone = Arc::clone(&electrs_client);
        let nostr_state_clone = nostr_state.clone();

        async move {
            match nostr_handler::NostrHandler::new(
                nostr_state_clone,
                keys_clone,
                pairing_manager_clone,
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

    let app_state = nostr_state.clone();
    let pubkey_for_root = pubkey_clone.clone();
    let relay_list_for_root = relay_list_clone.clone();
    let pubkey_for_pubkey = pubkey_clone.clone();
    let pubkey_for_info = pubkey_clone.clone();
    let relay_list_for_info = relay_list_clone.clone();

    let app = Router::new()
        .route("/", get(move || async move {
            serve_html_index(pubkey_for_root.clone(), relay_list_for_root.clone())
        }))
        .route("/pubkey", get(move || async move {
            serve_pubkey_plain(pubkey_for_pubkey.clone())
        }))
        .route("/info", get(move || async move {
            serve_info_text(pubkey_for_info.clone(), relay_list_for_info.clone())
        }))
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
        }))
        .with_state(app_state);

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

fn serve_html_index(pubkey: String, relay_list: Vec<String>) -> Response {
    let relay_list_html: String = relay_list
        .iter()
        .map(|relay| format!("<li>{}</li>", html_escape(relay)))
        .collect::<Vec<_>>()
        .join("\n    ");


    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>NomadServer</title>
    <style>
        body {{
            font-family: system-ui, -apple-system, sans-serif;
            max-width: 800px;
            margin: 40px auto;
            padding: 20px;
            line-height: 1.6;
        }}
        h1 {{ color: #2563eb; }}
        h2 {{ color: #1e40af; margin-top: 30px; }}
        code {{
            background: #f3f4f6;
            padding: 2px 6px;
            border-radius: 4px;
            font-family: 'Courier New', monospace;
            word-break: break-all;
        }}
        button {{
            background: #2563eb;
            color: white;
            border: none;
            padding: 8px 16px;
            border-radius: 4px;
            cursor: pointer;
            margin-left: 10px;
        }}
        button:hover {{ background: #1e40af; }}
        ul {{ list-style-type: none; padding-left: 0; }}
        li {{ margin: 8px 0; }}
        a {{ color: #2563eb; text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
        .status {{ color: #059669; font-weight: bold; }}
        .copy-info {{
            font-size: 0.9em;
            color: #6b7280;
            margin-top: 4px;
        }}
    </style>
</head>
<body>
    <h1>🔐 NomadServer</h1>
    <p>Status: <span class="status">✅ Running</span></p>
    
    <h2>Pairing Configuration</h2>
    <p>
        <code id="pubkey">{}</code>
        <button onclick="copyPairingJSON()">Copy Pairing JSON</button>
    </p>
    <p class="copy-info">
        💡 The "Copy Pairing JSON" button copies the complete pairing configuration. 
        Paste it into the NomadWallet app's manual entry field.
    </p>
    
    <h2>Pairing Options</h2>
    <ul>
        <li><a href="/qr">📱 QR Code</a> - Scan with your phone to pair</li>
        <li><a href="/pairing">🔗 Pairing JSON</a> - View raw JSON (open in new tab to copy)</li>
    </ul>
    
    <h2>API Endpoints</h2>
    <ul>
        <li><a href="/pubkey">/pubkey</a> - Plain text public key</li>
        <li><a href="/info">/info</a> - Human-readable server info</li>
        <li><a href="/health">/health</a> - Health check</li>
        <li><a href="/health/electrs">/health/electrs</a> - Electrs connectivity check</li>
    </ul>
    
    <h2>Connected Relays</h2>
    <ul>
        {}
    </ul>

    <script>
        async function copyPairingJSON() {{
            try {{
                const response = await fetch('/pairing');
                const pairingJSON = await response.text();
                
                // Try modern Clipboard API first
                if (navigator.clipboard && navigator.clipboard.writeText) {{
                    try {{
                        await navigator.clipboard.writeText(pairingJSON);
                        alert('Pairing JSON copied to clipboard! Paste it into the NomadWallet app.');
                        return;
                    }} catch (clipboardErr) {{
                        console.log('Clipboard API failed, trying fallback:', clipboardErr);
                    }}
                }}
                
                // Fallback: Use execCommand (works in more browsers)
                const textArea = document.createElement('textarea');
                textArea.value = pairingJSON;
                textArea.style.position = 'fixed';
                textArea.style.left = '-999999px';
                textArea.style.top = '-999999px';
                document.body.appendChild(textArea);
                textArea.focus();
                textArea.select();
                
                try {{
                    const successful = document.execCommand('copy');
                    document.body.removeChild(textArea);
                    if (successful) {{
                        alert('Pairing JSON copied to clipboard! Paste it into the NomadWallet app.');
                    }} else {{
                        throw new Error('execCommand copy failed');
                    }}
                }} catch (execErr) {{
                    document.body.removeChild(textArea);
                    throw execErr;
                }}
            }} catch (err) {{
                alert('Failed to copy. Please manually copy from /pairing endpoint or use the QR code. Error: ' + err.message);
            }}
        }}
    </script>
</body>
</html>"#,
        html_escape(&pubkey),
        relay_list_html
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

fn serve_pubkey_plain(pubkey: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        pubkey,
    )
        .into_response()
}

fn serve_info_text(pubkey: String, relay_list: Vec<String>) -> Response {
    let relay_list_text = relay_list.join("\n  - ");

    let info = format!(
        r#"NomadServer
====================

Status: ✅ Running

Nostr Public Key:
  {}

Connected Relays:
  - {}

Pairing Options:
  - QR Code: http://localhost:3829/qr
  - JSON: http://localhost:3829/pairing

API Endpoints:
  - GET /          - This info page (HTML)
  - GET /pubkey    - Plain text public key
  - GET /info      - This info (text format)
  - GET /pairing   - Pairing JSON
  - GET /qr        - QR code (SVG)
  - GET /health    - Health check
  - GET /health/electrs - Electrs connectivity

To pair your wallet:
  1. Scan the QR code at /qr with your phone
  2. Or copy the pubkey above and enter manually
  3. Or use the pairing JSON at /pairing
"#,
        pubkey, relay_list_text
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        info,
    )
        .into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
