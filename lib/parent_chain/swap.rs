//! Trustless swap functionality for CoinShift
//!
//! Implements conditional payments: "will pay L2 coins, iff a specific L1
//! transaction exists and gets X confirmations"

use std::collections::HashMap;
use thiserror::Error;

use crate::parent_chain::{
    config::ParentChainType,
    client::{ParentChainClient, TxId},
    default_confirmations,
};

/// Unique identifier for a swap
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SwapId(pub [u8; 32]);

/// State of a swap
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SwapState {
    /// Swap has been created but L1 transaction not yet seen
    Pending,
    /// L1 transaction detected, waiting for confirmations
    WaitingConfirmations {
        current_confirmations: u32,
        required_confirmations: u32,
    },
    /// Required confirmations reached, L2 payment can be claimed
    ReadyToClaim,
    /// Swap has been completed (L2 coins claimed)
    Completed,
    /// Swap expired or was cancelled
    Cancelled,
}

/// A trustless swap between L2 coins and a parent chain asset
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Swap {
    pub id: SwapId,
    /// The parent chain this swap is for
    pub parent_chain: ParentChainType,
    /// The L1 transaction ID that must be confirmed
    pub l1_txid: TxId,
    /// Required number of confirmations
    pub required_confirmations: u32,
    /// Current state of the swap
    pub state: SwapState,
    /// L2 address that will receive the coins
    pub l2_recipient: crate::types::Address,
    /// Amount of L2 coins to be paid
    pub l2_amount: bitcoin::Amount,
    /// Block height when swap was created
    pub created_at_height: u32,
    /// Optional expiration height
    pub expires_at_height: Option<u32>,
}

impl Swap {
    /// Create a new swap
    pub fn new(
        parent_chain: ParentChainType,
        l1_txid: TxId,
        required_confirmations: Option<u32>,
        l2_recipient: crate::types::Address,
        l2_amount: bitcoin::Amount,
        current_height: u32,
    ) -> Self {
        let required_confirmations = required_confirmations
            .unwrap_or_else(|| default_confirmations(parent_chain));

        // Generate swap ID from parent chain, txid, and recipient
        let mut id_data = Vec::new();
        id_data.extend_from_slice(&l1_txid.hash_bytes());
        id_data.extend_from_slice(&l2_recipient.0);
        let id_hash = blake3::hash(&id_data);
        let id = SwapId(*id_hash.as_bytes());

        Self {
            id,
            parent_chain,
            l1_txid,
            required_confirmations,
            state: SwapState::Pending,
            l2_recipient,
            l2_amount,
            created_at_height: current_height,
            expires_at_height: None,
        }
    }

    /// Update swap state based on current L1 transaction status
    pub async fn update_state(
        &mut self,
        client: &ParentChainClient,
        current_height: u32,
    ) -> Result<(), SwapError> {
        // Check if expired
        if let Some(expires_at) = self.expires_at_height {
            if current_height >= expires_at {
                self.state = SwapState::Cancelled;
                return Ok(());
            }
        }

        // Check L1 transaction status
        let tx_info = client
            .get_client(&self.parent_chain)
            .ok_or(SwapError::ChainNotConfigured(self.parent_chain.clone()))?
            .get_transaction(&self.l1_txid)
            .await
            .map_err(|e| SwapError::ClientError(e.to_string()))?;

        match tx_info {
            None => {
                // Transaction not found yet
                if matches!(self.state, SwapState::Pending) {
                    // Stay in pending
                } else {
                    // If we were waiting but tx disappeared, something is wrong
                    return Err(SwapError::TransactionDisappeared);
                }
            }
            Some(tx) => {
                match self.state {
                    SwapState::Pending => {
                        // Transaction found, now waiting for confirmations
                        self.state = SwapState::WaitingConfirmations {
                            current_confirmations: tx.confirmations,
                            required_confirmations: self.required_confirmations,
                        };
                    }
                    SwapState::WaitingConfirmations {
                        current_confirmations: _,
                        required_confirmations: _,
                    } => {
                        // Update confirmation count
                        if tx.confirmations >= self.required_confirmations {
                            self.state = SwapState::ReadyToClaim;
                        } else {
                            self.state = SwapState::WaitingConfirmations {
                                current_confirmations: tx.confirmations,
                                required_confirmations: self.required_confirmations,
                            };
                        }
                    }
                    SwapState::ReadyToClaim | SwapState::Completed | SwapState::Cancelled => {
                        // Already in final state
                    }
                }
            }
        }

        Ok(())
    }

    /// Mark swap as completed after L2 payment is claimed
    pub fn mark_completed(&mut self) -> Result<(), SwapError> {
        match self.state {
            SwapState::ReadyToClaim => {
                self.state = SwapState::Completed;
                Ok(())
            }
            _ => Err(SwapError::InvalidStateTransition),
        }
    }
}

/// Manager for active swaps
pub struct SwapManager {
    swaps: HashMap<SwapId, Swap>,
}

impl SwapManager {
    pub fn new() -> Self {
        Self {
            swaps: HashMap::new(),
        }
    }

    pub fn create_swap(
        &mut self,
        parent_chain: ParentChainType,
        l1_txid: TxId,
        required_confirmations: Option<u32>,
        l2_recipient: crate::types::Address,
        l2_amount: bitcoin::Amount,
        current_height: u32,
    ) -> SwapId {
        let swap = Swap::new(
            parent_chain,
            l1_txid,
            required_confirmations,
            l2_recipient,
            l2_amount,
            current_height,
        );
        let id = swap.id.clone();
        self.swaps.insert(id.clone(), swap);
        id
    }

    pub fn get_swap(&self, id: &SwapId) -> Option<&Swap> {
        self.swaps.get(id)
    }

    pub fn get_swap_mut(&mut self, id: &SwapId) -> Option<&mut Swap> {
        self.swaps.get_mut(id)
    }

    pub fn update_all_swaps(
        &mut self,
        client: &ParentChainClient,
        current_height: u32,
    ) -> Result<(), SwapError> {
        for swap in self.swaps.values_mut() {
            swap.update_state(client, current_height).ok();
        }
        Ok(())
    }
}

impl Default for SwapManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Error)]
pub enum SwapError {
    #[error("Chain not configured: {0}")]
    ChainNotConfigured(ParentChainType),
    #[error("Client error: {0}")]
    ClientError(String),
    #[error("Transaction disappeared from chain")]
    TransactionDisappeared,
    #[error("Invalid state transition")]
    InvalidStateTransition,
    #[error("Swap not found")]
    SwapNotFound,
    #[error("Swap expired")]
    SwapExpired,
}

// Helper trait for TxId to get hash bytes
trait TxIdHash {
    fn hash_bytes(&self) -> &[u8];
}

impl TxIdHash for TxId {
    fn hash_bytes(&self) -> &[u8] {
        match self {
            TxId::Hash32(hash) => hash.as_slice(),
            TxId::Hash(hash) => hash.as_slice(),
        }
    }
}

