//! Configuration for parent chain connections

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strum::{Display, EnumString};
use thiserror::Error;
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "UPPERCASE")]
pub enum ParentChainType {
    Btc,
    Bch,
    Ltc,
    Xmr,
    Eth,
    Tron,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParentChainNodeConfig {
    /// RPC/API endpoint URL for the parent chain node
    pub node_url: Url,
    /// Optional authentication credentials
    pub auth: Option<ChainAuth>,
    /// Custom confirmation count (overrides default)
    pub confirmation_count: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ChainAuth {
    /// Username and password for RPC authentication
    Basic { username: String, password: String },
    /// API key for API-based chains
    ApiKey(String),
    /// JWT token
    Token(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParentChainConfig {
    /// Configuration for each supported parent chain
    pub chains: HashMap<ParentChainType, ParentChainNodeConfig>,
}

impl ParentChainConfig {
    pub fn new() -> Self {
        Self {
            chains: HashMap::new(),
        }
    }

    /// Add or update configuration for a parent chain
    pub fn set_chain(
        &mut self,
        chain: ParentChainType,
        node_url: Url,
        auth: Option<ChainAuth>,
        confirmation_count: Option<u32>,
    ) {
        self.chains.insert(
            chain,
            ParentChainNodeConfig {
                node_url,
                auth,
                confirmation_count,
            },
        );
    }

    /// Get configuration for a specific chain
    pub fn get_chain(&self, chain: &ParentChainType) -> Option<&ParentChainNodeConfig> {
        self.chains.get(chain)
    }

    /// Check if a chain is configured
    pub fn is_configured(&self, chain: &ParentChainType) -> bool {
        self.chains.contains_key(chain)
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Chain not configured: {0}")]
    ChainNotConfigured(ParentChainType),
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
    #[error("Missing required configuration for chain: {0}")]
    MissingConfig(ParentChainType),
}

