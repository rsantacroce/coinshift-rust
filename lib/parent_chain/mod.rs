//! Parent chain abstraction for CoinShift
//! 
//! This module provides an abstraction layer for interacting with multiple
//! parent chains (BTC, BCH, LTC, XMR, ETH, Tron) to support trustless swaps.

use std::time::Duration;
use thiserror::Error;

pub mod config;
pub mod client;
pub mod swap;

pub use config::{ParentChainConfig, ParentChainType, ChainAuth, ParentChainNodeConfig};
pub use client::{ParentChainClient, ParentChainClientTrait, TxId, ParentChainTx};
pub use swap::{Swap, SwapState, SwapError, SwapId, SwapManager};

/// Default confirmation time target: 45 minutes
pub const DEFAULT_CONFIRMATION_TIME: Duration = Duration::from_secs(45 * 60);

/// Block time estimates for different chains (in seconds)
pub mod block_times {
    use std::time::Duration;
    
    pub const BTC: Duration = Duration::from_secs(600);  // 10 minutes
    pub const BCH: Duration = Duration::from_secs(600);  // 10 minutes
    pub const LTC: Duration = Duration::from_secs(150); // 2.5 minutes
    pub const XMR: Duration = Duration::from_secs(120); // 2 minutes
    pub const ETH: Duration = Duration::from_secs(12);   // 12 seconds
    pub const TRON: Duration = Duration::from_secs(3);   // 3 seconds
}

/// Calculate default confirmation count for a chain based on target time
pub fn default_confirmations(chain: ParentChainType) -> u32 {
    let block_time = match chain {
        ParentChainType::Btc => block_times::BTC,
        ParentChainType::Bch => block_times::BCH,
        ParentChainType::Ltc => block_times::LTC,
        ParentChainType::Xmr => block_times::XMR,
        ParentChainType::Eth => block_times::ETH,
        ParentChainType::Tron => block_times::TRON,
    };
    
    // Calculate blocks needed for ~45 minutes
    let target_seconds = DEFAULT_CONFIRMATION_TIME.as_secs();
    let block_seconds = block_time.as_secs();
    
    // Round up to ensure at least the target time
    ((target_seconds + block_seconds - 1) / block_seconds) as u32
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Parent chain client error: {0}")]
    Client(#[from] client::Error),
    #[error("Swap error: {0}")]
    Swap(#[from] swap::SwapError),
    #[error("Configuration error: {0}")]
    Config(#[from] config::Error),
}

