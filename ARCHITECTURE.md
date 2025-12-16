# Architecture

## Communication Model

### Android ↔ Server Communication

**Protocol**: Nostr (NIP-44 encrypted events)  
**Transport**: Public Nostr relays  
**No**: REST APIs, GraphQL, WebSockets, or direct HTTP connections

The Android app and Umbrel server communicate exclusively through:
1. **Nostr events** - Standard Nostr event format
2. **NIP-44 encryption** - All events encrypted end-to-end
3. **Public relays** - Events published/subscribed via public Nostr relays

The server never exposes HTTP endpoints for mobile clients. All communication is asynchronous and relay-based.

### Web UI ↔ Server Communication

**Protocol**: HTTP/WebSocket (local only)  
**Scope**: Umbrel network only  
**Purpose**: Local administration and monitoring

The web UI is a separate interface for Umbrel users to:
- Monitor server status
- View logs and metrics
- Configure server settings
- Generate pairing QR codes

This UI is **not** accessible to mobile clients and does not handle mobile communication.

## Data Flow

```
Android App → Public Nostr Relay → Server (listens)
Server → Public Nostr Relay → Android App (listens)
```

Both sides:
1. Connect to public relays
2. Subscribe to relevant event kinds
3. Encrypt/decrypt using NIP-44
4. Process events asynchronously

## Security

- **NIP-44 encryption**: All mobile communication is encrypted
- **Public key authentication**: Uses Nostr keypairs for identity
- **No direct connections**: No server IPs or ports exposed to mobile
- **Relay-based**: Leverages Nostr's decentralized relay network

