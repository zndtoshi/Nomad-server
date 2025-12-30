# Transaction Broadcasting, Fee Estimation, and UTXO Features Added

## Summary

Added 3 new message types to NomadServer Nostr relay server to support complete wallet functionality:
1. **Transaction Broadcasting** (`broadcast_tx`)
2. **Fee Estimation** (`get_fees`)
3. **UTXO Listing** (`get_utxos`)

## Files Modified

### 1. `app/server/src/nostr_handler.rs`

#### New Request/Response Structs

```rust
// Requests
#[derive(Debug, Serialize, Deserialize)]
struct BroadcastTxRequest {
    #[serde(rename = "type")]
    req_type: String,
    #[serde(rename = "txHex")]
    tx_hex: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetFeesRequest {
    #[serde(rename = "type")]
    req_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetUtxosRequest {
    #[serde(rename = "type")]
    req_type: String,
    addresses: Vec<String>,
}

// Responses
#[derive(Debug, Serialize)]
struct BroadcastTxResponse {
    req: String,
    success: bool,
    txid: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct GetFeesResponse {
    req: String,
    fast: u64,   // sat/vB
    medium: u64, // sat/vB
    slow: u64,   // sat/vB
}

#[derive(Debug, Serialize)]
struct GetUtxosResponse {
    req: String,
    utxos: Vec<UtxoInfo>,
}

#[derive(Debug, Serialize)]
struct UtxoInfo {
    txid: String,
    vout: u32,
    value: u64,
    address: String,
    confirmations: u32,
}
```

#### Updated start_listening() Method

- Added routing based on `type` field in request JSON
- Added match cases for all 4 message types:
  - `bitcoin_lookup` (existing)
  - `broadcast_tx` (new)
  - `get_fees` (new)
  - `get_utxos` (new)
- Proper error handling for each type
- Logging for all requests/responses

#### New Handler Methods

**1. broadcast_and_publish()**
- Accepts transaction hex and broadcasts via Electrs
- 30 second timeout
- Returns txid on success or error message on failure
- Follows existing pattern with single-flight gate

**2. fees_and_publish()**
- Estimates fees for fast (1 block), medium (6 blocks), slow (12 blocks)
- 30 second timeout
- Returns sat/vB values
- Falls back to defaults (10/5/1) if Electrs fails

**3. utxos_and_publish()**
- Fetches UTXOs for multiple addresses
- 45 second timeout
- Respects single-flight gate
- Returns empty array on error instead of failing

### 2. `app/server/src/electrs.rs`

#### New Methods

**1. broadcast_transaction_blocking() / broadcast_transaction()**
- Blocking: Decodes hex, validates transaction, broadcasts via `transaction_broadcast_raw()`
- Async wrapper: Uses spawn_blocking with single-flight gate
- Rate limiting applied
- Returns txid string

**2. estimate_fees_blocking() / estimate_fees()**
- Blocking: Calls `estimate_fee()` for 1, 6, and 12 blocks
- Converts BTC/kB to sat/vB (multiply by 100,000)
- Async wrapper: Uses spawn_blocking with single-flight gate
- Minimum 1 sat/vB enforced

**3. get_utxos_blocking() / get_utxos()**
- Blocking: Loops through addresses, calls `script_list_unspent()` for each
- Gets current blockchain height via `block_headers_subscribe()`
- Calculates confirmations: `current_height - utxo_height` (0 for mempool)
- Async wrapper: Uses spawn_blocking with single-flight gate
- Rate limiting between addresses

## Message Protocols

### 1. Transaction Broadcasting

**Request (Kind 30078):**
```json
{
  "type": "broadcast_tx",
  "txHex": "0200000001abc123..."
}
```

**Response (Kind 30079):**
```json
{
  "req": "<uuid>",
  "success": true,
  "txid": "def456..."
}
```

**Response (Error):**
```json
{
  "req": "<uuid>",
  "success": false,
  "txid": null,
  "error": "Invalid transaction"
}
```

### 2. Fee Estimation

**Request (Kind 30078):**
```json
{
  "type": "get_fees"
}
```

**Response (Kind 30079):**
```json
{
  "req": "<uuid>",
  "fast": 10,      // sat/vB for ~10 min (1 block)
  "medium": 5,     // sat/vB for ~30 min (6 blocks)
  "slow": 1        // sat/vB for ~1 hour (12 blocks)
}
```

### 3. UTXO Listing

**Request (Kind 30078):**
```json
{
  "type": "get_utxos",
  "addresses": ["bc1q...", "bc1q..."]
}
```

**Response (Kind 30079):**
```json
{
  "req": "<uuid>",
  "utxos": [
    {
      "txid": "abc123...",
      "vout": 0,
      "value": 100000,
      "address": "bc1q...",
      "confirmations": 3
    }
  ]
}
```

## Design Patterns Followed

✅ **Timeout Handling**: All operations use `tokio::time::timeout`
- Transaction broadcasting: 30 seconds
- Fee estimation: 30 seconds
- UTXO fetching: 45 seconds

✅ **Single-Flight Gate**: All Electrs operations respect the global semaphore to prevent overwhelming the Electrs server

✅ **Rate Limiting**: All blocking operations call `rate_limit()` before Electrs RPC calls (100ms minimum spacing)

✅ **Cooldown on Timeout**: Existing cooldown mechanism prevents cascading failures

✅ **Graceful Error Handling**: 
- Broadcast: Returns error message in response
- Fees: Falls back to reasonable defaults (10/5/1 sat/vB)
- UTXOs: Returns empty array instead of failing

✅ **Consistent Response Format**: All responses include `req` field for request matching

✅ **Logging**: All requests/responses logged with INFO level, errors with WARN/ERROR

## Testing Checklist

### Transaction Broadcasting
- [ ] Valid transaction hex
- [ ] Invalid hex format
- [ ] Invalid transaction structure
- [ ] Timeout handling (slow Electrs)
- [ ] Double spend (already confirmed)

### Fee Estimation
- [ ] Normal operation (Electrs responsive)
- [ ] Electrs timeout (should return defaults)
- [ ] Electrs error (should return defaults)
- [ ] Verify sat/vB conversion accuracy

### UTXO Listing
- [ ] Single address with UTXOs
- [ ] Multiple addresses
- [ ] Address with no UTXOs (empty array)
- [ ] Mix of confirmed and unconfirmed UTXOs
- [ ] Confirmation count accuracy
- [ ] Timeout handling

### Integration
- [ ] All message types work with existing `bitcoin_lookup`
- [ ] Proper request routing based on `type` field
- [ ] Response events published with correct kind (30079)
- [ ] req/p tags properly set in responses

## Client-Side Integration (NomadWallet)

The NomadWallet app will need to update `src/services/nostr/NomadServer.ts` to add 3 new methods:

```typescript
// In NomadServerService class:

async broadcastTransaction(txHex: string): Promise<string> {
  const reqId = this.generateRequestId();
  const request = { type: 'broadcast_tx', txHex };
  
  const response = await this.sendRequest(reqId, request);
  if (!response.success) {
    throw new Error(response.error || 'Broadcast failed');
  }
  return response.txid;
}

async getFeeEstimates(): Promise<{ fast: number; medium: number; slow: number }> {
  const reqId = this.generateRequestId();
  const request = { type: 'get_fees' };
  
  const response = await this.sendRequest(reqId, request);
  return {
    fast: response.fast,
    medium: response.medium,
    slow: response.slow,
  };
}

async getUTXOs(addresses: string[]): Promise<UTXO[]> {
  const reqId = this.generateRequestId();
  const request = { type: 'get_utxos', addresses };
  
  const response = await this.sendRequest(reqId, request);
  return response.utxos;
}
```

## Deployment

1. **Compile**: `cargo build --release`
2. **Test**: Run integration tests with Electrs testnet
3. **Docker**: Rebuild Docker image with new features
4. **Umbrel**: Deploy updated app to Umbrel App Store

## Dependencies

No new dependencies needed - all features use existing crates:
- `electrum-client` - Already supports broadcast, fee estimation, and UTXO queries
- `hex` - Already in dependencies for encoding/decoding
- `tokio` - Already used for async operations

## Performance Considerations

- **Single-Flight Gate**: Prevents Electrs overload - only 1 request at a time
- **Rate Limiting**: 100ms between RPC calls prevents connection saturation
- **Timeouts**: Prevent hanging on slow Electrs responses
- **Graceful Degradation**: Fees fall back to defaults instead of failing entire request

## Security Notes

⚠️ **Transaction Broadcasting**: Server validates transaction format but does NOT validate signatures or amounts - this is the wallet's responsibility (and Bitcoin's consensus rules)

✅ **No Private Keys**: Server never sees or handles private keys - only signed transactions

✅ **Read-Only Operations**: Fees and UTXOs are read-only blockchain queries

✅ **Nostr Authentication**: All requests must come from paired clients (p-tag verification)

---

**Status**: ✅ Implementation Complete
**Next**: Test with NomadWallet client integration

