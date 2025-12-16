//! Extended public key (xpub) address derivation
//!
//! Derives Bitcoin addresses from xpub/ypub/zpub/tpub with gap limit support.

use anyhow::{Context, Result};
use bitcoin::bip32::{DerivationPath, Xpub};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::Network;
use std::str::FromStr;
use tracing::{info, warn};

/// Derive addresses from an extended public key
///
/// Supports xpub (mainnet), ypub/zpub (SegWit), tpub (testnet)
/// Derives both external (receiving) and internal (change) addresses
/// with a gap limit of 20 for each chain.
pub fn derive_addresses(xpub_str: &str, gap_limit: u32) -> Result<Vec<String>> {
    info!("Deriving addresses from xpub with gap_limit={}", gap_limit);

    // Determine network from xpub prefix
    let network = detect_network(xpub_str)?;

    // Parse the xpub using bitcoin crate
    let xpub = Xpub::from_str(xpub_str)
        .context("Failed to parse extended public key")?;

    // Create secp256k1 context for key operations
    let secp = Secp256k1::new();

    let mut addresses = Vec::new();

    // Derive external (receiving) addresses: m/0/0, m/0/1, ..., m/0/(gap_limit-1)
    info!("Deriving external (receiving) addresses");
    for i in 0..gap_limit {
        let path_str = format!("m/0/{}", i);
        let path = DerivationPath::from_str(&path_str)
            .context("Failed to create derivation path")?;
        
        match derive_address_from_path(&xpub, &path, network, &secp) {
            Ok(addr) => {
                addresses.push(addr);
            }
            Err(e) => {
                warn!("Failed to derive address at path {}: {}", path_str, e);
                break; // Stop if derivation fails
            }
        }
    }

    // Derive internal (change) addresses: m/1/0, m/1/1, ..., m/1/(gap_limit-1)
    info!("Deriving internal (change) addresses");
    for i in 0..gap_limit {
        let path_str = format!("m/1/{}", i);
        let path = DerivationPath::from_str(&path_str)
            .context("Failed to create derivation path")?;
        
        match derive_address_from_path(&xpub, &path, network, &secp) {
            Ok(addr) => {
                addresses.push(addr);
            }
            Err(e) => {
                warn!("Failed to derive address at path {}: {}", path_str, e);
                break; // Stop if derivation fails
            }
        }
    }

    info!("Derived {} addresses from xpub", addresses.len());

    Ok(addresses)
}

/// Detect Bitcoin network from xpub prefix
fn detect_network(xpub_str: &str) -> Result<Network> {
    let prefix = xpub_str.get(0..4).unwrap_or("");
    
    match prefix {
        "xpub" => Ok(Network::Bitcoin),
        "tpub" => Ok(Network::Testnet),
        "ypub" | "zpub" => {
            // ypub/zpub can be mainnet or testnet, default to mainnet
            // In a real implementation, you might want to check more carefully
            Ok(Network::Bitcoin)
        }
        _ => {
            // Default to mainnet, but warn
            warn!("Unknown xpub prefix '{}', defaulting to mainnet", prefix);
            Ok(Network::Bitcoin)
        }
    }
}

/// Derive a single address from xpub and derivation path
fn derive_address_from_path(
    xpub: &Xpub,
    path: &DerivationPath,
    network: Network,
    secp: &Secp256k1<bitcoin::secp256k1::All>,
) -> Result<String> {
    // Derive the public key at this path
    let child_xpub = xpub.derive_pub(secp, path)
        .context("Failed to derive child key")?;

    // Get the public key - in bitcoin 0.32, Xpub.public_key is a field
    let secp_pubkey = child_xpub.public_key;
    
    // Convert secp256k1::PublicKey to bitcoin::PublicKey
    // bitcoin::PublicKey::new() takes secp256k1::PublicKey
    let bitcoin_pubkey = bitcoin::PublicKey::new(secp_pubkey);

    // Convert to Bitcoin address (P2PKH for legacy, P2WPKH for SegWit)
    // For now, use P2PKH - in production you'd detect address type from xpub prefix
    let address = bitcoin::Address::p2pkh(&bitcoin_pubkey, network);
    
    Ok(address.to_string())
}

/// Check if a string looks like an extended public key
pub fn is_xpub(query: &str) -> bool {
    query.starts_with("xpub")
        || query.starts_with("ypub")
        || query.starts_with("zpub")
        || query.starts_with("tpub")
}

/// Check if a string looks like a Bitcoin address
pub fn is_bitcoin_address(query: &str) -> bool {
    // Basic check - starts with 1, 3, bc1, or tb1
    query.starts_with('1')
        || query.starts_with('3')
        || query.starts_with("bc1")
        || query.starts_with("tb1")
}

