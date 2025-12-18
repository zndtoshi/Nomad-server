use anyhow::{anyhow, Result};
use electrum_client::bitcoin::{Address, Network, ScriptBuf};
use electrum_client::{Client, ElectrumApi};
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::{info, warn};

#[derive(Clone)]
pub struct ElectrsClient {
    client: Arc<Client>,
    addr: String,

    // Soft rate limit between individual RPC calls
    last_call: Arc<Mutex<Instant>>,

    // Hard global gate: only one in-flight Electrs request at a time
    gate: Arc<Semaphore>,

    // Cooldown until this time (set when a timeout happens)
    cooldown_until: Arc<Mutex<Option<Instant>>>,
}

impl ElectrsClient {
    pub fn new() -> Result<Self> {
        let addr = std::env::var("ELECTRS_ADDR").unwrap_or_else(|_| "electrs:50001".to_string());
        info!("ElectrsClient using ELECTRS_ADDR={}", addr);

        preflight_tcp(&addr)?;

        let client = Client::new(&addr)
            .map_err(|e| anyhow!("Failed to create electrum client for {}: {}", addr, e))?;

        Ok(Self {
            client: Arc::new(client),
            addr,
            last_call: Arc::new(Mutex::new(Instant::now())),
            gate: Arc::new(Semaphore::new(1)),
            cooldown_until: Arc::new(Mutex::new(None)),
        })
    }

    pub fn test_connectivity(&self) -> Result<()> {
        self.client
            .ping()
            .map_err(|e| anyhow!("Electrs ping failed ({}) : {}", self.addr, e))?;
        Ok(())
    }

    /// Warm-up call at startup. This is intentionally blocking and should be called once in main()
    /// before the Nostr listener starts handling requests.
    pub fn warm_up(&self) -> Result<()> {
        info!("Electrs warm-up: ping()");
        self.test_connectivity()?;
        info!("Electrs warm-up OK");
        Ok(())
    }

    fn rate_limit(&self) {
        let mut last = self.last_call.lock().unwrap();
        let elapsed = last.elapsed();

        // Minimum spacing between Electrum RPC calls (soft limit)
        if elapsed < Duration::from_millis(100) {
            std::thread::sleep(Duration::from_millis(100) - elapsed);
        }

        *last = Instant::now();
    }

    fn check_cooldown(&self) -> Result<()> {
        let mut cd = self.cooldown_until.lock().unwrap();
        if let Some(until) = *cd {
            let now = Instant::now();
            if now < until {
                let remaining = until.duration_since(now);
                return Err(anyhow!(
                    "Electrs cooling down ({}ms remaining)",
                    remaining.as_millis()
                ));
            } else {
                *cd = None;
            }
        }
        Ok(())
    }

    fn set_cooldown(&self, seconds: u64) {
        let mut cd = self.cooldown_until.lock().unwrap();
        *cd = Some(Instant::now() + Duration::from_secs(seconds));
    }

    /// BLOCKING tx history lookup
    fn get_address_txs_blocking(&self, address: &str) -> Result<Vec<String>> {
        self.rate_limit();

        let addr = Address::from_str(address)?.require_network(Network::Bitcoin)?;
        let script: ScriptBuf = addr.script_pubkey();

        let history = self.client.script_get_history(&script)?;
        Ok(history.into_iter().map(|h| h.tx_hash.to_string()).collect())
    }

    /// BLOCKING balance lookup with history fast-path:
    /// 1) Call script_get_history first
    ///    - if empty => immediately return (0,0) (avoids listunspent cost/blocking)
    /// 2) If non-empty => call script_list_unspent and sum values
    ///
    /// This keeps the service stateless while avoiding listunspent calls for unused addresses.
    fn get_address_balance_blocking(&self, address: &str) -> Result<(u64, u64)> {
        let addr = Address::from_str(address)?.require_network(Network::Bitcoin)?;
        let script: ScriptBuf = addr.script_pubkey();

        // ---- Fast-path: check history first ----
        self.rate_limit();
        let history = self.client.script_get_history(&script)?;
        if history.is_empty() {
            return Ok((0, 0));
        }

        // ---- Only if there is history, compute balance from UTXOs ----
        self.rate_limit();
        let utxos = self.client.script_list_unspent(&script)?;

        let mut confirmed: u64 = 0;
        let mut unconfirmed: u64 = 0;

        for u in utxos {
            // Convention: height == 0 => mempool/unconfirmed
            if u.height > 0 {
                confirmed = confirmed.saturating_add(u.value);
            } else {
                unconfirmed = unconfirmed.saturating_add(u.value);
            }
        }

        Ok((confirmed, unconfirmed))
    }

    /// Balance lookup:
    /// - single-flight gate (global)
    /// - cooldown after timeout
    /// - 90s timeout + 1 retry
    pub async fn get_address_balance(&self, address: &str) -> Result<(u64, u64)> {
        use tokio::task::spawn_blocking;
        use tokio::time::{timeout, Duration};

        // Respect cooldown (fast-fail instead of wedging Electrs)
        self.check_cooldown()?;

        // Global single-flight gate
        let _permit = self.gate.acquire().await.unwrap();

        // Re-check cooldown after acquiring (someone else might have set it)
        self.check_cooldown()?;

        // ---- First attempt (90s) ----
        let addr1 = address.to_string();
        let this1 = self.clone();

        let first = timeout(
            Duration::from_secs(90),
            spawn_blocking(move || this1.get_address_balance_blocking(&addr1)),
        )
        .await;

        match first {
            Ok(Ok(Ok(v))) => return Ok(v),
            Ok(Ok(Err(e))) => return Err(anyhow!("Electrs balance error: {}", e)),
            Ok(Err(e)) => return Err(anyhow!("Electrs join error: {}", e)),
            Err(_) => {
                warn!("Electrs balance timed out, setting cooldown + retrying once...");
                // cooldown helps the whole system recover (wallet + UI)
                self.set_cooldown(10);
            }
        }

        // ---- Second attempt (retry, 90s) ----
        let addr2 = address.to_string();
        let this2 = self.clone();

        let second = timeout(
            Duration::from_secs(90),
            spawn_blocking(move || this2.get_address_balance_blocking(&addr2)),
        )
        .await;

        match second {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("Electrs balance error (retry): {}", e)),
            Ok(Err(e)) => Err(anyhow!("Electrs join error (retry): {}", e)),
            Err(_) => {
                warn!("Electrs balance timed out after retry; setting longer cooldown");
                self.set_cooldown(20);
                Err(anyhow!("Electrs balance timeout (after retry)"))
            }
        }
    }

    /// History lookup (used only for xpub path):
    /// - single-flight gate (global)
    /// - cooldown after timeout
    /// - 45s timeout (no retries here by default)
    pub async fn get_address_txs(&self, address: &str) -> Result<Vec<String>> {
        use tokio::task::spawn_blocking;
        use tokio::time::{timeout, Duration};

        self.check_cooldown()?;
        let _permit = self.gate.acquire().await.unwrap();
        self.check_cooldown()?;

        let addr = address.to_string();
        let this = self.clone();

        let res = timeout(
            Duration::from_secs(45),
            spawn_blocking(move || this.get_address_txs_blocking(&addr)),
        )
        .await;

        match res {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("Electrs tx error: {}", e)),
            Ok(Err(e)) => Err(anyhow!("Electrs join error: {}", e)),
            Err(_) => {
                warn!("Electrs history timed out; setting cooldown");
                self.set_cooldown(10);
                Err(anyhow!("Electrs history timeout"))
            }
        }
    }
}

fn preflight_tcp(addr: &str) -> Result<()> {
    let mut addrs = addr
        .to_socket_addrs()
        .map_err(|e| anyhow!("Invalid ELECTRS_ADDR '{}': {}", addr, e))?;

    let sock = addrs
        .next()
        .ok_or_else(|| anyhow!("ELECTRS_ADDR '{}' did not resolve", addr))?;

    let stream = std::net::TcpStream::connect_timeout(&sock, Duration::from_secs(3))
        .map_err(|e| anyhow!("Electrs TCP preflight failed to {}: {}", addr, e))?;

    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    Ok(())
}
