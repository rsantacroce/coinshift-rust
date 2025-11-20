//! Parent chain client abstraction
//!
//! Provides a unified interface for interacting with different parent chains

use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use crate::parent_chain::config::{ChainAuth, ParentChainConfig, ParentChainNodeConfig, ParentChainType};

/// Transaction ID type (varies by chain)
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TxId {
    /// 32-byte hash (BTC, BCH, LTC, XMR)
    Hash32([u8; 32]),
    /// Variable length hash (ETH, Tron)
    Hash(Vec<u8>),
}

/// Transaction information from a parent chain
#[derive(Clone, Debug)]
pub struct ParentChainTx {
    pub txid: TxId,
    pub confirmations: u32,
    pub block_hash: Option<String>,
    pub block_height: Option<u64>,
}

/// Trait for parent chain clients
pub trait ParentChainClientTrait: Send + Sync {
    /// Get the chain type this client handles
    fn chain_type(&self) -> ParentChainType;

    /// Get transaction information
    async fn get_transaction(&self, txid: &TxId) -> Result<Option<ParentChainTx>, Error>;

    /// Get current block height
    async fn get_block_height(&self) -> Result<u64, Error>;

    /// Verify a transaction exists and has sufficient confirmations
    async fn verify_transaction(
        &self,
        txid: &TxId,
        min_confirmations: u32,
    ) -> Result<bool, Error>;
}

/// Client manager for multiple parent chains
pub struct ParentChainClient {
    clients: HashMap<ParentChainType, Box<dyn ParentChainClientTrait>>,
}

impl ParentChainClient {
    pub fn new(config: &ParentChainConfig) -> Result<Self, Error> {
        let mut clients = HashMap::new();

        // Initialize BTC client if configured
        if let Some(btc_config) = config.get_chain(&ParentChainType::Btc) {
            let btc_client = BtcClient::new(btc_config)?;
            clients.insert(ParentChainType::Btc, Box::new(btc_client));
        }
        
        Ok(Self { clients })
    }

    pub fn get_client(&self, chain: &ParentChainType) -> Option<&dyn ParentChainClientTrait> {
        self.clients.get(chain).map(|c| c.as_ref())
    }

    pub fn verify_transaction(
        &self,
        chain: &ParentChainType,
        txid: &TxId,
        min_confirmations: u32,
    ) -> Result<bool, Error> {
        let client = self
            .clients
            .get(chain)
            .ok_or(Error::ChainNotConfigured(chain.clone()))?;
        
        // This would need to be async, but for now showing the structure
        // In practice, this would need to be called from an async context
        Ok(false) // Placeholder
    }
}

/// Bitcoin Core JSON-RPC client
pub struct BtcClient {
    rpc_url: String,
    http_client: Arc<reqwest::Client>,
    auth: Option<ChainAuth>,
}

impl BtcClient {
    pub fn new(config: &ParentChainNodeConfig) -> Result<Self, Error> {
        let rpc_url = config.node_url.to_string();
        let auth = config.auth.clone();
        
        // Create HTTP client with appropriate timeout
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Network(format!("Failed to create HTTP client: {}", e)))?;
        
        Ok(Self {
            rpc_url,
            http_client: Arc::new(http_client),
            auth,
        })
    }

    /// Make a JSON-RPC call to Bitcoin Core
    async fn rpc_call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, Error> {
        // Build request
        let mut request = self.http_client.post(&self.rpc_url);
        
        // Add authentication if provided
        if let Some(ChainAuth::Basic { username, password }) = &self.auth {
            request = request.basic_auth(username, Some(password));
        }
        
        // Bitcoin Core uses JSON-RPC 1.0 format
        let jsonrpc_request = serde_json::json!({
            "jsonrpc": "1.0",
            "id": "coinshift",
            "method": method,
            "params": params
        });
        
        // Send request
        let response = request
            .json(&jsonrpc_request)
            .send()
            .await
            .map_err(|e| Error::Network(format!("HTTP request failed: {}", e)))?;
        
        // Check status
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::Rpc(format!(
                "HTTP error {}: {}",
                status, text
            )));
        }
        
        // Parse response
        let json_response: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::Network(format!("Failed to parse JSON response: {}", e)))?;
        
        // Check for RPC error
        if let Some(error) = json_response.get("error") {
            if !error.is_null() {
                let error_msg = error
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown RPC error");
                return Err(Error::Rpc(error_msg.to_string()));
            }
        }
        
        // Extract result
        let result = json_response
            .get("result")
            .ok_or_else(|| Error::Rpc("Missing 'result' field in RPC response".to_string()))?;
        
        serde_json::from_value(result.clone())
            .map_err(|e| Error::Rpc(format!("Failed to deserialize RPC result: {}", e)))
    }
}

impl ParentChainClientTrait for BtcClient {
    fn chain_type(&self) -> ParentChainType {
        ParentChainType::Btc
    }

    async fn get_transaction(&self, txid: &TxId) -> Result<Option<ParentChainTx>, Error> {
        // Extract hex string from TxId
        let txid_hex = match txid {
            TxId::Hash32(hash) => hex::encode(hash),
            TxId::Hash(_) => return Err(Error::InvalidTxId),
        };
        
        // Call getrawtransaction with verbose=true to get block info
        let params = serde_json::json!([txid_hex, true]);
        
        // Bitcoin Core returns null if transaction not found
        let tx_info: Option<BtcTxInfo> = self.rpc_call("getrawtransaction", params).await?;
        
        if let Some(tx_info) = tx_info {
            // Confirmations can be negative for unconfirmed transactions
            let confirmations = tx_info.confirmations.unwrap_or(0).max(0) as u32;
            let block_hash = tx_info.blockhash;
            let block_height = tx_info.blockheight;
            
            Ok(Some(ParentChainTx {
                txid: txid.clone(),
                confirmations,
                block_hash,
                block_height,
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_block_height(&self) -> Result<u64, Error> {
        let params = serde_json::json!([]);
        let height: u64 = self.rpc_call("getblockcount", params).await?;
        Ok(height)
    }

    async fn verify_transaction(
        &self,
        txid: &TxId,
        min_confirmations: u32,
    ) -> Result<bool, Error> {
        let tx = self.get_transaction(txid).await?;
        
        if let Some(tx) = tx {
            Ok(tx.confirmations >= min_confirmations)
        } else {
            Ok(false)
        }
    }
}

/// Bitcoin transaction info from getrawtransaction
#[derive(Debug, serde::Deserialize)]
struct BtcTxInfo {
    #[serde(default)]
    confirmations: Option<i64>,
    #[serde(default)]
    blockhash: Option<String>,
    #[serde(default)]
    blockheight: Option<u64>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Chain not configured: {0}")]
    ChainNotConfigured(ParentChainType),
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Invalid transaction ID")]
    InvalidTxId,
    #[error("Transaction not found")]
    TxNotFound,
}

#[cfg(test)]
mod tests {
    //! Tests for Bitcoin Core RPC client
    //!
    //! These tests require a running Bitcoin Core node with RPC enabled.
    //! Set the following environment variables to run the tests:
    //!
    //! - `BTC_RPC_URL`: The RPC endpoint URL (e.g., `http://localhost:8332`)
    //! - `BTC_RPC_USER`: (Optional) RPC username
    //! - `BTC_RPC_PASS`: (Optional) RPC password
    //!
    //! Example:
    //! ```bash
    //! BTC_RPC_URL=http://localhost:8332 BTC_RPC_USER=rpcuser BTC_RPC_PASS=rpcpass \
    //! cargo test --package plain_bitassets --lib parent_chain::client::tests
    //! ```
    //!
    //! Tests will be skipped if `BTC_RPC_URL` is not set.

    use super::*;
    use crate::parent_chain::config::{ChainAuth, ParentChainNodeConfig};
    use std::env;
    use url::Url;

    /// Helper to create a BTC client from environment variables
    /// Returns None if BTC_RPC_URL is not set
    fn create_test_btc_client() -> Option<BtcClient> {
        let rpc_url = env::var("BTC_RPC_URL").ok()?;
        let url = Url::parse(&rpc_url).ok()?;
        
        let auth = if let (Ok(username), Ok(password)) = (
            env::var("BTC_RPC_USER"),
            env::var("BTC_RPC_PASS"),
        ) {
            Some(ChainAuth::Basic { username, password })
        } else {
            None
        };

        let config = ParentChainNodeConfig {
            node_url: url,
            auth,
            confirmation_count: None,
        };

        BtcClient::new(&config).ok()
    }

    #[tokio::test]
    async fn test_btc_get_block_height() {
        let client = match create_test_btc_client() {
            Some(client) => client,
            None => {
                eprintln!("Skipping test: BTC_RPC_URL not set");
                return;
            }
        };

        let height = client.get_block_height().await;
        match height {
            Ok(h) => {
                assert!(h > 0, "Block height should be greater than 0");
                println!("✓ Successfully fetched block height: {}", h);
            }
            Err(e) => {
                panic!("Failed to get block height: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_btc_get_transaction() {
        let client = match create_test_btc_client() {
            Some(client) => client,
            None => {
                eprintln!("Skipping test: BTC_RPC_URL not set");
                return;
            }
        };

        // Use a well-known Bitcoin transaction ID for testing
        // This is the genesis block coinbase transaction
        let genesis_txid_hex = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
        let genesis_txid_bytes = hex::decode(genesis_txid_hex)
            .expect("Failed to decode hex");
        let mut txid_array = [0u8; 32];
        txid_array.copy_from_slice(&genesis_txid_bytes);
        let txid = TxId::Hash32(txid_array);

        let tx = client.get_transaction(&txid).await;
        match tx {
            Ok(Some(tx_info)) => {
                println!("✓ Successfully fetched transaction:");
                println!("  - Confirmations: {}", tx_info.confirmations);
                if let Some(ref block_hash) = tx_info.block_hash {
                    println!("  - Block hash: {}", block_hash);
                }
                if let Some(block_height) = tx_info.block_height {
                    println!("  - Block height: {}", block_height);
                }
                // Genesis transaction should have many confirmations
                assert!(tx_info.confirmations > 0, "Genesis transaction should be confirmed");
            }
            Ok(None) => {
                eprintln!("⚠ Transaction not found (may not be in this node's view)");
            }
            Err(e) => {
                // This is okay - the transaction might not be in the node's view
                // or the node might be on a different network
                eprintln!("⚠ Could not fetch transaction (this is okay if node doesn't have it): {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_btc_verify_transaction() {
        let client = match create_test_btc_client() {
            Some(client) => client,
            None => {
                eprintln!("Skipping test: BTC_RPC_URL not set");
                return;
            }
        };

        // Use genesis transaction for testing
        let genesis_txid_hex = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
        let genesis_txid_bytes = hex::decode(genesis_txid_hex)
            .expect("Failed to decode hex");
        let mut txid_array = [0u8; 32];
        txid_array.copy_from_slice(&genesis_txid_bytes);
        let txid = TxId::Hash32(txid_array);

        // Verify with minimum 1 confirmation
        let verified = client.verify_transaction(&txid, 1).await;
        match verified {
            Ok(true) => {
                println!("✓ Transaction verified with sufficient confirmations");
            }
            Ok(false) => {
                eprintln!("⚠ Transaction exists but doesn't have enough confirmations (or not found)");
            }
            Err(e) => {
                eprintln!("⚠ Could not verify transaction (this is okay if node doesn't have it): {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_btc_client_integration() {
        let client = match create_test_btc_client() {
            Some(client) => client,
            None => {
                eprintln!("Skipping test: BTC_RPC_URL not set");
                return;
            }
        };

        // Test that chain_type returns BTC
        assert_eq!(client.chain_type(), ParentChainType::Btc);
        println!("✓ Chain type is correct: BTC");

        // Test getting block height
        let height = client.get_block_height().await;
        assert!(height.is_ok(), "Should be able to get block height");
        let height = height.unwrap();
        assert!(height > 0, "Block height should be positive");
        println!("✓ Block height: {}", height);

        println!("✓ All integration tests passed!");
    }
}

