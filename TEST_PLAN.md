# BalanceBridge End-to-End Test Plan

## Overview
This document provides step-by-step instructions for testing the BalanceBridge Umbrel app end-to-end, including Bitcoin lookup requests from the Android app to the Rust server.

---

## STEP 1: Verify Runtime Configuration

### Check Electrs URL Configuration

The server will log the Electrs URL at startup. Verify it's correct for your Umbrel setup:

**Expected log output:**
```
Electrs client initialized with URL: http://electrs:3002
Electrs URL can be overridden via ELECTRS_URL environment variable
```

**If Electrs is not accessible via `http://electrs:3002`, update `umbrel/app.yml`:**
```yaml
environment:
  ELECTRS_URL: http://localhost:3002  # or your custom URL
```

---

## STEP 2: Umbrel Commands

### Rebuild and Restart the App

```bash
# Navigate to your Umbrel app directory
cd /path/to/umbrel-balancebridge

# Rebuild the Docker image (if code changed)
docker build -t balancebridge:latest .

# Restart the app via Umbrel UI, or if you have direct access:
# Stop the container
docker stop balancebridge

# Start the container
docker start balancebridge
```

### View Logs

```bash
# Tail logs from the BalanceBridge container
docker logs -f balancebridge

# Or if using Umbrel's app management:
# Logs are typically available in Umbrel UI under the app's "Logs" tab
```

### Test Electrs Connectivity

**From inside the BalanceBridge container:**
```bash
# Enter the container
docker exec -it balancebridge /bin/sh

# Test Electrs connectivity
curl http://electrs:3002/
# or
curl http://localhost:3002/

# Test a specific address endpoint (replace with a real address)
curl http://electrs:3002/address/bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh/balance
```

**From the host (using the health check endpoint):**
```bash
# Test the health endpoint (tests Electrs connectivity)
curl http://localhost:3829/health

# Expected response: "OK - Electrs reachable"
# If Electrs is unreachable: "Electrs unreachable: <error>"
```

---

## STEP 3: Test Data

### Testnet Address (Recommended for Testing)

**Address with known transactions:**
```
tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx
```

This is a testnet address that should have some transaction history.

### Mainnet Address (Use with Caution)

**Address with known transactions:**
```
bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh
```

**Note:** Only use mainnet addresses if your Umbrel node is synced to mainnet.

### Testnet Extended Public Key (xpub)

**Testnet xpub for testing:**
```
tpubD6NzVbkrYhZ4XHndKkuB8FifXm8r5FQHwrN6oZuWCz13qb93rtgKTCdQ5wLZQDiU6Kj2n6TdkV6Vb7SzL5jXb5P8Z7Y3vN8K9mN2pQ3rT4u
```

**Note:** This is a placeholder. Use a real testnet xpub that has derived addresses with transactions.

---

## STEP 4: Expected Log Output

### On Server Startup

```
BalanceBridge Umbrel Server starting...
Using data dir: /data
Electrs client initialized with URL: http://electrs:3002
Electrs URL can be overridden via ELECTRS_URL environment variable
Starting Nostr handler...
Added relay: wss://relay.damus.io
Connected to 3 relay(s)
Listening for events from Android pubkey: <pubkey>...
Server ready. Waiting for Android app pairing...
```

### On Receiving Bitcoin Lookup Request

```
Received event from Android app: <event_id> (sender: <pubkey_short>)
Decrypted message: {"type":"bitcoin_lookup","query":"<address>"}
Processing Bitcoin lookup request from <pubkey_short>: type=address, query=<address>
Processing single address query: <address>
Querying Electrs for address balance: <address>
Address <address> balance: confirmed=<satoshis>, unconfirmed=<satoshis>
Querying Electrs for address transactions: <address>
Found <count> transactions for address <address>
Electrs balance query completed: confirmed=<satoshis>, unconfirmed=<satoshis>
Electrs transaction query completed: tx_count=<count>
Bitcoin lookup result: confirmed=<satoshis>, unconfirmed=<satoshis>, tx_count=<count>
Sending response to Android app (<pubkey_short>): confirmed=<satoshis>, unconfirmed=<satoshis>, tx_count=<count>
Response sent successfully to <pubkey_short>
```

### For xpub Queries

```
Processing Bitcoin lookup request from <pubkey_short>: type=xpub, query=<xpub>
Processing xpub query: <xpub>
Deriving addresses from xpub with gap_limit=20
Deriving external (receiving) addresses
Deriving internal (change) addresses
Derived 40 addresses from xpub (gap_limit=20)
Querying Electrs for address balance: <address1>
...
Electrs query completed for xpub: confirmed=<satoshis>, unconfirmed=<satoshis>, tx_count=<count>
```

---

## STEP 5: Success Criteria

### ✅ Successful Test Indicators

1. **Server starts without errors**
   - All modules initialize
   - Electrs client connects
   - Nostr handler connects to relays

2. **Health check passes**
   ```bash
   curl http://localhost:3829/health
   # Returns: "OK - Electrs reachable"
   ```

3. **Request received and processed**
   - Logs show "Received event from Android app"
   - Logs show "Processing Bitcoin lookup request"
   - Logs show Electrs queries completing

4. **Response sent successfully**
   - Logs show "Response sent successfully"
   - Android app receives JSON response with:
     - `type: "bitcoin_lookup_result"`
     - `confirmed_balance: <number>`
     - `unconfirmed_balance: <number>`
     - `transactions: [<array>]`

### ❌ Common Failures and Debugging

#### 1. Electrs Connection Failed

**Symptoms:**
```
Electrs returned error 500: <error>
Failed to connect to Electrs
```

**Debug:**
```bash
# Check if Electrs is running
docker ps | grep electrs

# Test connectivity from container
docker exec -it balancebridge curl http://electrs:3002/

# Try alternative URL
# Update ELECTRS_URL in umbrel/app.yml to http://localhost:3002
```

#### 2. Nostr Relay Connection Failed

**Symptoms:**
```
Failed to add relay: <url>
Nostr handler error: <error>
```

**Debug:**
- Check internet connectivity from Umbrel
- Verify relay URLs are correct
- Try different public relays

#### 3. No Events Received

**Symptoms:**
- Server starts but no "Received event" logs

**Debug:**
- Verify Android app is paired (check pairing file exists)
- Check Android app logs for "Sent Bitcoin lookup request"
- Verify both devices are connected to same relays
- Check event filters match (kind 30078, correct pubkeys)

#### 4. Decryption Failed

**Symptoms:**
```
Failed to decrypt event
Failed to decrypt pairing event
```

**Debug:**
- Verify pairing was successful
- Check that Android app is using correct Umbrel node pubkey
- Ensure NIP-44 encryption is working on both sides

#### 5. Invalid Request Format

**Symptoms:**
```
Failed to parse request JSON
Invalid Bitcoin lookup request
```

**Debug:**
- Check Android app is sending correct JSON format:
  ```json
  {"type": "bitcoin_lookup", "query": "<address_or_xpub>"}
  ```
- Verify query is valid (address or xpub format)

---

## STEP 6: End-to-End Test Procedure

### Prerequisites
1. Umbrel node running with Bitcoin node synced
2. Electrs service running and accessible
3. BalanceBridge app installed and running on Umbrel
4. Android app installed and paired with Umbrel node

### Test Steps

1. **Start the server and verify logs:**
   ```bash
   docker logs -f balancebridge
   ```
   Wait for: "Server ready. Waiting for Android app pairing..."

2. **Pair Android app:**
   - Open Android app
   - Scan QR code from Umbrel app
   - Verify pairing successful

3. **Test single address lookup:**
   - In Android app, enter testnet address: `tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx`
   - Tap "Lookup"
   - **Expected:** Response with balance and transactions

4. **Verify server logs:**
   - Check for "Received event from Android app"
   - Check for "Processing single address query"
   - Check for "Electrs query completed"
   - Check for "Response sent successfully"

5. **Test xpub lookup (if xpub available):**
   - Enter a testnet xpub
   - Tap "Lookup"
   - **Expected:** Response with aggregated balance and transactions from derived addresses

6. **Verify response format:**
   - Android app should display:
     - Confirmed balance
     - Unconfirmed balance
     - Transaction list

---

## STEP 7: Verification Checklist

- [ ] Server starts without errors
- [ ] Electrs URL is correct and reachable
- [ ] Health check endpoint returns "OK"
- [ ] Nostr handler connects to relays
- [ ] Android app pairs successfully
- [ ] Bitcoin lookup request received (check logs)
- [ ] Electrs queries complete successfully
- [ ] Response sent back to Android app
- [ ] Android app displays results correctly

---

## Additional Notes

- **Network Configuration:** If Electrs is not accessible via `http://electrs:3002`, you may need to:
  - Check Umbrel's Docker network configuration
  - Use `http://localhost:3002` if Electrs runs on the same host
  - Configure custom URL via `ELECTRS_URL` environment variable

- **Logging Level:** To see more detailed logs, set:
  ```bash
  RUST_LOG=debug
  ```

- **Testnet vs Mainnet:** Use testnet addresses for initial testing to avoid using real funds.

