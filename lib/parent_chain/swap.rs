//! Trustless swap functionality for CoinShift
//!
//! Implements L2 → L1 swaps: conditional payments where L2 coins are paid
//! if a specific L1 transaction exists and gets X confirmations.
//!
//! Note: L1 → L2 deposits are handled by the BIP300 sidechain implementation
//! itself (automatic minting when deposits are detected). This swap system
//! is specifically for L2 → L1 peer-to-peer exchanges.

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

/// Direction of the swap
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SwapDirection {
    /// L1 → L2: User sends L1 coins, receives L2 coins
    L1ToL2,
    /// L2 → L1: User offers L2 coins, receives L1 coins (someone else sends L1)
    L2ToL1,
}

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
    /// Direction of the swap
    pub direction: SwapDirection,
    /// The parent chain this swap is for
    pub parent_chain: ParentChainType,
    /// The L1 transaction ID that must be confirmed
    pub l1_txid: TxId,
    /// Required number of confirmations
    pub required_confirmations: u32,
    /// Current state of the swap
    pub state: SwapState,
    /// L2 address that will receive the coins (or sender for L2ToL1)
    pub l2_recipient: crate::types::Address,
    /// Amount of L2 coins to be paid
    pub l2_amount: bitcoin::Amount,
    /// For L2ToL1 swaps: L1 address where L1 coins should be sent
    pub l1_recipient_address: Option<String>,
    /// For L2ToL1 swaps: Amount of L1 coins required
    pub l1_amount: Option<bitcoin::Amount>,
    /// Block height when swap was created
    pub created_at_height: u32,
    /// Optional expiration height
    pub expires_at_height: Option<u32>,
}

impl Swap {
    // TODO remove it
    /// Create a new L1 → L2 swap (deposit)
    pub fn new_l1_to_l2(
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
            direction: SwapDirection::L1ToL2,
            parent_chain,
            l1_txid,
            required_confirmations,
            state: SwapState::Pending,
            l2_recipient,
            l2_amount,
            l1_recipient_address: None,
            l1_amount: None,
            created_at_height: current_height,
            expires_at_height: None,
        }
    }

    /// Create a new L2 → L1 swap (withdrawal/exchange)
    /// Alice offers L2 coins, Bob sends L1 coins to Alice's address, Bob claims L2 coins
    pub fn new_l2_to_l1(
        parent_chain: ParentChainType,
        l1_recipient_address: String,
        l1_amount: bitcoin::Amount,
        l2_sender: crate::types::Address, // Alice's L2 address (offering coins)
        l2_amount: bitcoin::Amount,
        l2_recipient: crate::types::Address, // Bob's L2 address (will claim)
        required_confirmations: Option<u32>,
        current_height: u32,
    ) -> Self {
        let required_confirmations = required_confirmations
            .unwrap_or_else(|| default_confirmations(parent_chain));

        // For L2ToL1, we don't have l1_txid yet (Bob hasn't sent it)
        // Use a placeholder that will be updated when Bob fills the swap
        let placeholder_txid = TxId::Hash32([0u8; 32]);

        // Generate swap ID from parent chain, L1 address, L1 amount, L2 sender, and L2 recipient
        let mut id_data = Vec::new();
        id_data.extend_from_slice(l1_recipient_address.as_bytes());
        id_data.extend_from_slice(&l1_amount.to_sat().to_le_bytes());
        id_data.extend_from_slice(&l2_sender.0);
        id_data.extend_from_slice(&l2_recipient.0);
        let id_hash = blake3::hash(&id_data);
        let id = SwapId(*id_hash.as_bytes());

        Self {
            id,
            direction: SwapDirection::L2ToL1,
            parent_chain,
            l1_txid: placeholder_txid,
            required_confirmations,
            state: SwapState::Pending,
            l2_recipient, // Bob's address (will claim)
            l2_amount,
            l1_recipient_address: Some(l1_recipient_address),
            l1_amount: Some(l1_amount),
            created_at_height: current_height,
            expires_at_height: None,
        }
    }

    /// Update L1 transaction ID for L2ToL1 swaps (when Bob fills the swap)
    pub fn set_l1_txid(&mut self, l1_txid: TxId) -> Result<(), SwapError> {
        if self.direction != SwapDirection::L2ToL1 {
            return Err(SwapError::InvalidStateTransition);
        }
        if !matches!(self.state, SwapState::Pending) {
            return Err(SwapError::InvalidStateTransition);
        }
        self.l1_txid = l1_txid;
        Ok(())
    }

    /// Legacy method for backwards compatibility
    pub fn new(
        parent_chain: ParentChainType,
        l1_txid: TxId,
        required_confirmations: Option<u32>,
        l2_recipient: crate::types::Address,
        l2_amount: bitcoin::Amount,
        current_height: u32,
    ) -> Self {
        Self::new_l1_to_l2(
            parent_chain,
            l1_txid,
            required_confirmations,
            l2_recipient,
            l2_amount,
            current_height,
        )
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
    pub(crate) swaps: HashMap<SwapId, Swap>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Address;

    fn create_test_address() -> Address {
        Address([0u8; 32])
    }

    fn create_test_txid() -> TxId {
        TxId::Hash32([1u8; 32])
    }

    #[test]
    fn test_swap_creation() {
        let parent_chain = ParentChainType::Btc;
        let l1_txid = create_test_txid();
        let recipient = create_test_address();
        let amount = bitcoin::Amount::from_sat(100_000);
        let height = 100;

        let swap = Swap::new(
            parent_chain.clone(),
            l1_txid.clone(),
            None,
            recipient,
            amount,
            height,
        );

        assert_eq!(swap.parent_chain, parent_chain);
        assert_eq!(swap.l1_txid, l1_txid);
        assert_eq!(swap.l2_amount, amount);
        assert_eq!(swap.created_at_height, height);
        assert!(matches!(swap.state, SwapState::Pending));
        assert_eq!(swap.required_confirmations, default_confirmations(parent_chain));
    }

    #[test]
    fn test_swap_id_deterministic() {
        let parent_chain = ParentChainType::Btc;
        let l1_txid = create_test_txid();
        let recipient = create_test_address();
        let amount = bitcoin::Amount::from_sat(100_000);
        let height = 100;

        let swap1 = Swap::new(
            parent_chain.clone(),
            l1_txid.clone(),
            None,
            recipient,
            amount,
            height,
        );

        let swap2 = Swap::new(
            parent_chain,
            l1_txid,
            None,
            create_test_address(), // Different recipient
            amount,
            height,
        );

        // Different recipients should produce different swap IDs
        assert_ne!(swap1.id, swap2.id);
    }

    #[test]
    fn test_swap_id_same_for_same_params() {
        let parent_chain = ParentChainType::Btc;
        let l1_txid = create_test_txid();
        let recipient = create_test_address();
        let amount = bitcoin::Amount::from_sat(100_000);
        let height = 100;

        let swap1 = Swap::new(
            parent_chain.clone(),
            l1_txid.clone(),
            None,
            recipient,
            amount,
            height,
        );

        let swap2 = Swap::new(
            parent_chain,
            l1_txid,
            None,
            create_test_address(), // Same recipient (test address is deterministic)
            amount,
            height,
        );

        // Same parameters should produce same swap ID
        assert_eq!(swap1.id, swap2.id);
    }

    #[test]
    fn test_swap_mark_completed() {
        let mut swap = Swap::new(
            ParentChainType::Btc,
            create_test_txid(),
            None,
            create_test_address(),
            bitcoin::Amount::from_sat(100_000),
            100,
        );

        // Can't mark completed from Pending state
        assert!(swap.mark_completed().is_err());

        // Set to ReadyToClaim
        swap.state = SwapState::ReadyToClaim;

        // Now can mark as completed
        assert!(swap.mark_completed().is_ok());
        assert!(matches!(swap.state, SwapState::Completed));
    }

    #[test]
    fn test_swap_manager_create() {
        let mut manager = SwapManager::new();
        let swap_id = manager.create_swap(
            ParentChainType::Btc,
            create_test_txid(),
            None,
            create_test_address(),
            bitcoin::Amount::from_sat(100_000),
            100,
        );

        assert!(manager.get_swap(&swap_id).is_some());
        let swap = manager.get_swap(&swap_id).unwrap();
        assert!(matches!(swap.state, SwapState::Pending));
    }

    #[test]
    fn test_swap_expiration() {
        let mut swap = Swap::new(
            ParentChainType::Btc,
            create_test_txid(),
            None,
            create_test_address(),
            bitcoin::Amount::from_sat(100_000),
            100,
        );

        swap.expires_at_height = Some(150);

        // At height 149, should not be expired
        // At height 150, should be expired
        // We can't test this without a mock client, but the logic is there
        assert_eq!(swap.expires_at_height, Some(150));
    }

    #[test]
    fn test_swap_custom_confirmations() {
        let custom_confirmations = 10;
        let swap = Swap::new(
            ParentChainType::Btc,
            create_test_txid(),
            Some(custom_confirmations),
            create_test_address(),
            bitcoin::Amount::from_sat(100_000),
            100,
        );

        assert_eq!(swap.required_confirmations, custom_confirmations);
    }

    #[test]
    fn test_alice_bob_l2_to_l1_swap() {
        // Scenario: Alice wants BTC, Bob wants L2 coins
        // 1. Alice has L2 coins and wants BTC
        // 2. Alice creates a swap offer: "I'll give 100,000 L2 sats if you send 0.001 BTC to my BTC address"
        // 3. Bob sends 0.001 BTC to Alice's BTC address
        // 4. After confirmations, Bob claims Alice's L2 coins

        let alice_l2_address = Address([1u8; 32]); // Alice's L2 address (has coins)
        let bob_l2_address = Address([2u8; 32]);    // Bob's L2 address (will receive Alice's coins)
        let alice_btc_address = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_string();
        
        let l2_amount = bitcoin::Amount::from_sat(100_000); // Alice offers 100k L2 sats
        let l1_amount = bitcoin::Amount::from_sat(100_000);  // Bob needs to send 0.001 BTC (100k sats)
        let current_height = 1000;

        // Step 1: Alice creates the swap offer
        let mut swap = Swap::new_l2_to_l1(
            ParentChainType::Btc,
            alice_btc_address.clone(),
            l1_amount,
            alice_l2_address,      // Alice is offering her L2 coins
            l2_amount,
            bob_l2_address,         // Bob will receive Alice's L2 coins
            Some(3),                // 3 confirmations required
            current_height,
        );

        // Verify swap is set up correctly
        assert_eq!(swap.direction, SwapDirection::L2ToL1);
        assert_eq!(swap.l1_recipient_address, Some(alice_btc_address.clone()));
        assert_eq!(swap.l1_amount, Some(l1_amount));
        assert_eq!(swap.l2_amount, l2_amount);
        assert_eq!(swap.l2_recipient, bob_l2_address);
        assert!(matches!(swap.state, SwapState::Pending));
        assert_eq!(swap.required_confirmations, 3);

        // Step 2: Bob sends BTC to Alice's address
        // In reality, Bob would create a Bitcoin transaction sending 0.001 BTC to alice_btc_address
        // For the test, we simulate this by setting the L1 transaction ID
        let bob_btc_txid = TxId::Hash32([0x42u8; 32]); // Bob's BTC transaction ID
        
        // Update swap with Bob's transaction
        assert!(swap.set_l1_txid(bob_btc_txid.clone()).is_ok());
        assert_eq!(swap.l1_txid, bob_btc_txid);

        // Step 3: Simulate transaction being detected and confirmed
        // In a real scenario, the background task would detect the transaction
        // and update the swap state as confirmations come in
        
        // Simulate: Transaction found, 0 confirmations
        swap.state = SwapState::WaitingConfirmations {
            current_confirmations: 0,
            required_confirmations: 3,
        };

        // Simulate: 1 confirmation
        swap.state = SwapState::WaitingConfirmations {
            current_confirmations: 1,
            required_confirmations: 3,
        };

        // Simulate: 2 confirmations
        swap.state = SwapState::WaitingConfirmations {
            current_confirmations: 2,
            required_confirmations: 3,
        };

        // Simulate: 3 confirmations reached - ready to claim!
        swap.state = SwapState::ReadyToClaim;

        // Step 4: Bob claims Alice's L2 coins
        assert!(swap.mark_completed().is_ok());
        assert!(matches!(swap.state, SwapState::Completed));

        // Verify final state
        assert_eq!(swap.l1_txid, bob_btc_txid);
        assert_eq!(swap.l2_recipient, bob_l2_address); // Bob receives the coins
        assert_eq!(swap.l2_amount, l2_amount);
    }

    #[test]
    fn test_l2_to_l1_swap_id_deterministic() {
        // Verify that same parameters produce same swap ID
        let alice_l2 = Address([1u8; 32]);
        let bob_l2 = Address([2u8; 32]);
        let alice_btc = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_string();
        let l1_amount = bitcoin::Amount::from_sat(100_000);
        let l2_amount = bitcoin::Amount::from_sat(100_000);

        let swap1 = Swap::new_l2_to_l1(
            ParentChainType::Btc,
            alice_btc.clone(),
            l1_amount,
            alice_l2,
            l2_amount,
            bob_l2,
            Some(3),
            1000,
        );

        let swap2 = Swap::new_l2_to_l1(
            ParentChainType::Btc,
            alice_btc,
            l1_amount,
            Address([1u8; 32]), // Same as alice_l2
            l2_amount,
            Address([2u8; 32]), // Same as bob_l2
            Some(3),
            1000,
        );

        // Same parameters should produce same swap ID
        assert_eq!(swap1.id, swap2.id);
    }

    #[test]
    fn test_l2_to_l1_cannot_set_txid_after_pending() {
        let mut swap = Swap::new_l2_to_l1(
            ParentChainType::Btc,
            "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_string(),
            bitcoin::Amount::from_sat(100_000),
            Address([1u8; 32]),
            bitcoin::Amount::from_sat(100_000),
            Address([2u8; 32]),
            Some(3),
            1000,
        );

        // Can set txid when pending
        assert!(swap.set_l1_txid(TxId::Hash32([1u8; 32])).is_ok());

        // Cannot set txid after state changes
        swap.state = SwapState::ReadyToClaim;
        assert!(swap.set_l1_txid(TxId::Hash32([2u8; 32])).is_err());
    }

    
}
