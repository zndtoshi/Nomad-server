# Server

Rust backend server for BalanceBridge Umbrel app.

## Responsibilities

- Nostr identity management (keypair generation and persistence)
- Relay configuration (default public relays)
- QR code generation for pairing
- Nostr relay connection management (future)
- Event processing and routing (future)
- NIP-44 encryption/decryption for Android communication (future)
- Integration with Fulcrum/Electrs (future)

## Dependencies

- `nostr-sdk` - Core Nostr functionality and relay connections
- `nostr` - Nostr protocol primitives
- `tokio` - Async runtime
- `qrcode` - QR code generation
- `image` - Image encoding for QR codes
- `serde` / `serde_json` - JSON serialization
- `tracing` - Structured logging
- `anyhow` / `thiserror` - Error handling

## Modules

- `identity.rs` - Nostr keypair generation and persistence
- `relays.rs` - Relay configuration management
- `qr.rs` - QR payload and image generation
- `error.rs` - Error types

## Usage

### In Umbrel Container

The server runs inside an Umbrel container. Umbrel sets:
- `UMBREL_APP_DATA_DIR` - Persistent data directory (e.g., `/umbrel/app-data/balancebridge/data`)
- `UMBREL_APP_ID` - App identifier

All persistent data (keys, QR codes) is stored in the Umbrel app data directory.

### Local Development

```bash
# For local testing, defaults to ./data
cargo run --bin balancebridge-server
```

## Output

On first run, the server will:
1. Generate a Nostr keypair
2. Save keys to `{UMBREL_APP_DATA_DIR}/nostr_keys.json`
3. Generate pairing QR code to `{UMBREL_APP_DATA_DIR}/pairing_qr.png`
4. Log the pairing payload JSON

The QR code contains:
- `version`: 1
- `app`: "umbrel-balancebridge"
- `nodePubkey`: The node's Nostr public key (hex)
- `relays`: List of public relay URLs

## Communication Architecture

### Android Communication
- **Nostr-only**: All Android communication happens via Nostr events
- **NIP-44 encrypted**: All events are encrypted using NIP-44
- **Public relays**: Events flow through public Nostr relays
- **No HTTP/WebSocket**: Never expose REST, GraphQL, or WebSocket APIs for mobile

### Web UI Communication
- **Local only**: Web UI is accessible only within Umbrel network
- **Direct server access**: UI can use HTTP/WebSocket for local interface
