# BalanceBridge Umbrel App

Umbrel app for BalanceBridge - communicates with Android via Nostr protocol.

## Structure

- `app/server/` - Rust backend server (Nostr relay connections, NIP-44 encryption)
- `app/ui/` - Web UI for the Umbrel app (future)
- `protocol/` - Nostr protocol definitions and utilities
- `docker/` - Docker configuration files
- `umbrel/` - Umbrel-specific configuration and metadata

## Technology Stack

- **Rust** - Backend server implementation
- **nostr-sdk** - Nostr protocol and relay connections
- **NIP-44** - Encryption for secure communication
- **Tokio** - Async runtime

## Building

```bash
cargo build --release
```

## Running

```bash
cargo run --bin balancebridge-server
```

## Development

```bash
cargo run
```

## Features

- ✅ Connects to public Nostr relays
- ✅ Uses NIP-44 encryption
- 🔜 Will integrate with Fulcrum/Electrs for Bitcoin data

## Communication Architecture

### Mobile (Android) Communication
- **Exclusively via Nostr protocol** - No REST, GraphQL, or WebSocket APIs
- **NIP-44 encrypted events** - All mobile communication is encrypted using NIP-44
- **Public relays** - Events are sent/received over public Nostr relays
- **No direct HTTP endpoints** - Android app never makes HTTP requests to this server

### Web UI Communication
- **Local to Umbrel only** - Web UI is only accessible within the Umbrel network
- **Direct server communication** - UI can use HTTP/WebSocket for local Umbrel interface
