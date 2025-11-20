# CoinShift 1.0 Architecture

This document describes the organization and architecture changes made to support CoinShift 1.0, a trustless swap system for the sidechain.

## Overview

CoinShift 1.0 extends the existing sidechain codebase to support:
- Multiple parent chain support (BTC, BCH, LTC, XMR, ETH, Tron)
- Trustless swapping of L2 coins for parent chain assets (L2 → L1)
- Conditional payments based on L1 transaction confirmations
- Configurable confirmation requirements (default: ~45 minutes of PoW per chain)

**Note**: L1 → L2 deposits are already handled by the BIP300 sidechain implementation itself. When users deposit BTC, Litecoin, etc., the sidechain automatically mints corresponding L2 coins. The CoinShift swap system is specifically for **L2 → L1 swaps** (exchanging L2 coins for parent chain assets with other users).

## Architecture

### Parent Chain Abstraction

The new `lib/parent_chain/` module provides a unified interface for interacting with multiple parent chains:

- **`config.rs`**: Configuration management for parent chain node addresses and settings
- **`client.rs`**: Abstract client interface for parent chain interactions
- **`swap.rs`**: Trustless swap implementation with conditional payments

### Key Components

#### 1. Parent Chain Configuration

Parent chains are configured via:
- Command-line arguments: `--parent-chain-node CHAIN:URL` (can be repeated)
- Configuration file: `--parent-chain-config /path/to/config.json`

Example command-line usage:
```bash
./coinshift \
  --parent-chain-node BTC:http://localhost:8332 \
  --parent-chain-node BCH:http://localhost:8334 \
  --parent-chain-node ETH:http://localhost:8545
```

Example JSON configuration file:
```json
{
  "chains": {
    "BTC": {
      "node_url": "http://localhost:8332",
      "auth": {
        "Basic": {
          "username": "rpcuser",
          "password": "rpcpass"
        }
      },
      "confirmation_count": 3
    },
    "ETH": {
      "node_url": "http://localhost:8545",
      "confirmation_count": 225
    }
  }
}
```

#### 2. Swap System

The swap system implements **L2 → L1 swaps** (exchanging L2 coins for parent chain assets). It implements conditional payments: "will pay L2 coins, iff a specific L1 transaction exists and gets X confirmations."

**Example Use Case**: Alice has L2 coins and wants BTC. She creates a swap offering her L2 coins. Bob sends BTC to Alice's address. After confirmations, Bob claims Alice's L2 coins. This enables trustless peer-to-peer exchange between L2 and L1 assets.

**Swap States:**
- `Pending`: Swap created, waiting for L1 transaction to appear
- `WaitingConfirmations`: L1 transaction detected, waiting for required confirmations
- `ReadyToClaim`: Required confirmations reached, L2 payment can be claimed
- `Completed`: Swap completed (L2 coins claimed)
- `Cancelled`: Swap expired or cancelled

**Default Confirmation Requirements:**
- Calculated to provide ~45 minutes of PoW security per chain:
  - BTC: 3 blocks (10 min/block)
  - BCH: 3 blocks (10 min/block)
  - LTC: 18 blocks (2.5 min/block)
  - XMR: 23 blocks (2 min/block)
  - ETH: 225 blocks (12 sec/block)
  - Tron: 900 blocks (3 sec/block)

Users can override these defaults per swap or in configuration.

#### 3. Multiple First Claimers Problem

**Status**: To be determined

This is a known issue that needs to be addressed. Potential solutions include:
- Time-locked claims with first-come-first-served
- Atomic swap patterns
- Multi-signature requirements
- Economic incentives/disincentives

## Implementation Status

### Completed
- ✅ Parent chain abstraction module structure
- ✅ Configuration system for multiple chains
- ✅ Swap state machine and management
- ✅ CLI integration for parent chain configuration
- ✅ Default confirmation calculation based on block times

### TODO
- [ ] Implement actual RPC clients for each parent chain (BTC, BCH, LTC, XMR, ETH, Tron)
- [ ] Integrate swap manager into node state
- [ ] Add swap transaction types to sidechain
- [ ] Implement swap claim mechanism
- [ ] Add GUI for swap management
- [ ] Resolve "multiple first claimers" problem
- [ ] Add swap persistence to database
- [ ] Add monitoring and alerting for swaps

## Swap User Flow & Implementation Plan

### Sequence Diagram: Swap Flow (L1 → L2)

```
User          GUI/CLI        Node          SwapManager    ParentChainClient    L1 Chain
  |              |             |                |                |                |
  |-- Create Swap Request -->  |                |                |                |
  |              |             |                |                |                |
  |              |             |-- create_swap() -->             |                |
  |              |             |                |                |                |
  |              |             |<-- SwapId -----|                |                |
  |              |<-- SwapId --|                |                |                |
  |<-- SwapId ---|             |                |                |                |
  |              |             |                |                |                |
  |-- Send L1 TX -->          |                |                |                |
  |              |             |                |                |                |
  |              |             |                |                |-- L1 TX -->    |
  |              |             |                |                |                |
  |              |             |-- Background Task (periodic) -->|                |
  |              |             |                |                |                |
  |              |             |                |-- get_transaction() -->          |
  |              |             |                |                |                |
  |              |             |                |<-- TX Info (0 conf) --|          |
  |              |             |                |                |                |
  |              |             |-- update_state() -->            |                |
  |              |             |                |                |                |
  |              |             |<-- State: WaitingConfirmations -|                |
  |              |             |                |                |                |
  |              |             |-- Background Task (periodic) -->|                |
  |              |             |                |                |                |
  |              |             |                |-- get_transaction() -->          |
  |              |             |                |                |                |
  |              |             |                |<-- TX Info (N conf) --|         |
  |              |             |                |                |                |
  |              |             |-- update_state() -->            |                |
  |              |             |                |                |                |
  |              |             |<-- State: ReadyToClaim ---------|                |
  |              |             |                |                |                |
  |-- Claim Request -->        |                |                |                |
  |              |             |                |                |                |
  |              |             |-- validate_claim() -->          |                |
  |              |             |                |                |                |
  |              |             |-- create_claim_tx() -->         |                |
  |              |             |                |                |                |
  |              |             |-- submit_transaction() -->      |                |
  |              |             |                |                |                |
  |              |             |-- mark_completed() -->          |                |
  |              |             |                |                |                |
  |              |             |<-- State: Completed ------------|                |
  |              |             |                |                |                |
  |<-- Claim Success ---------|                |                |                |
```

### Detailed Task Breakdown

#### Phase 1: Core Infrastructure

**1.1 Swap Manager Integration into Node State**
- [ ] Add `SwapManager` field to `Node` struct
- [ ] Initialize `SwapManager` in `Node::new()` with parent chain client
- [ ] Add background task to periodically update swap states
- [ ] Add swap persistence database to `State` struct
- [ ] Implement swap loading from database on node startup
- [ ] Implement swap saving to database on state changes

**1.2 Swap Transaction Types**
- [ ] Add `SwapCreate` variant to `TransactionData` enum
  - Fields: `swap_id`, `parent_chain`, `l1_txid`, `required_confirmations`, `l2_recipient`, `l2_amount`
- [ ] Add `SwapClaim` variant to `TransactionData` enum
  - Fields: `swap_id`, `proof_data` (optional, for future verification)
- [ ] Update transaction validation to handle swap transactions
- [ ] Update transaction serialization/deserialization
- [ ] Add swap transaction handling in `state::block::apply_transaction()`

**1.3 Swap Persistence**
- [ ] Create swap database schema in `State`
  - Database: `swaps: DatabaseUnique<SwapId, Swap>`
  - Database: `swaps_by_l1_txid: DatabaseUnique<(ParentChainType, TxId), SwapId>` (for lookup)
  - Database: `swaps_by_recipient: DatabaseUnique<Address, Vec<SwapId>>` (for user queries)
- [ ] Implement swap save/load operations
- [ ] Add swap rollback support (if swap is in pending state when block is rolled back)
- [ ] Handle swap expiration cleanup

#### Phase 2: Claim Mechanism

**2.1 Swap Claim Validation**
- [ ] Implement `validate_swap_claim()` function
  - Verify swap exists and is in `ReadyToClaim` state
  - Verify claimer is the swap recipient (or authorized)
  - Verify swap hasn't expired
  - Verify L1 transaction still has required confirmations
  - Prevent double-claiming (check if swap already completed)
- [ ] Add claim validation to transaction validation pipeline
- [ ] Handle "multiple first claimers" problem
  - Option A: First-come-first-served (simplest)
  - Option B: Time-locked claims with priority
  - Option C: Atomic claim with proof requirement

**2.2 Swap Claim Transaction Creation**
- [ ] Implement `create_swap_claim_transaction()` function
  - Create transaction with `SwapClaim` data
  - Set recipient as transaction output
  - Calculate fees appropriately
  - Sign transaction with recipient's key
- [ ] Add claim transaction builder to wallet module
- [ ] Integrate with existing transaction creation flow

**2.3 Swap State Updates**
- [ ] Implement background task for swap monitoring
  - Poll parent chain clients for transaction status
  - Update swap states based on confirmation counts
  - Handle transaction disappearance (reorgs)
  - Mark expired swaps as cancelled
- [ ] Add swap state change notifications
- [ ] Integrate with node's block height updates

#### Phase 3: User Interface

**3.1 CLI Commands**
- [ ] Add `swap create` command
  - Parameters: `--parent-chain`, `--l1-txid`, `--recipient`, `--amount`, `--confirmations`
- [ ] Add `swap list` command
  - Show all swaps with their states
  - Filter by state, parent chain, recipient
- [ ] Add `swap status <swap-id>` command
  - Show detailed swap information
- [ ] Add `swap claim <swap-id>` command
  - Create and submit claim transaction

**3.2 GUI Components**
- [ ] Create swap management screen
  - List of active swaps with status indicators
  - Swap creation form
  - Swap details view
  - Claim button (enabled when ready)
- [ ] Add swap notifications
  - Alert when swap reaches `ReadyToClaim` state
  - Alert on swap expiration
  - Alert on swap completion
- [ ] Add swap history view
  - Show completed and cancelled swaps
- [ ] Integrate with existing wallet UI

#### Phase 4: Advanced Features

**4.1 Multiple First Claimers Solution**
- [ ] Implement chosen solution (see 2.1)
- [ ] Add claim locking mechanism
- [ ] Add claim timeout/retry logic
- [ ] Add monitoring for claim conflicts

**4.2 Monitoring & Alerting**
- [ ] Add swap metrics
  - Active swaps count by state
  - Average time to confirmation
  - Claim success rate
  - Expired swaps count
- [ ] Add swap event logging
- [ ] Add alert system for:
  - Stuck swaps (pending too long)
  - Failed claims
  - L1 transaction reversals
  - Parent chain client errors

**4.3 Additional Parent Chain Clients**
- [ ] Implement BCH client (Bitcoin Cash Node RPC)
- [ ] Implement LTC client (Litecoin Core RPC)
- [ ] Implement XMR client (Monero daemon RPC)
- [ ] Implement ETH client (Ethereum JSON-RPC)
- [ ] Implement Tron client (Tron node RPC)
- [ ] Add tests for each client
- [ ] Add error handling for chain-specific issues

#### Phase 5: Testing & Documentation

**5.1 Unit Tests**
- [ ] Test swap creation and state transitions
- [ ] Test swap claim validation
- [ ] Test swap persistence
- [ ] Test swap expiration
- [ ] Test multiple claimers handling

**5.2 Integration Tests**
- [ ] Test full swap flow with mock parent chain
- [ ] Test swap with real parent chain node (BTC)
- [ ] Test swap claim transaction submission
- [ ] Test swap state updates during block processing
- [ ] Test swap rollback scenarios

**5.3 Documentation**
- [ ] Update API documentation
- [ ] Add swap usage examples
- [ ] Document swap security considerations
- [ ] Add troubleshooting guide

### Implementation Order Recommendation

1. **Start with Phase 1.1 & 1.3** (Swap Manager Integration & Persistence)
   - Foundation for everything else
   - Allows swaps to be tracked and persisted

2. **Then Phase 1.2** (Swap Transaction Types)
   - Enables swaps to be part of the blockchain
   - Required for claims to work

3. **Then Phase 2.1 & 2.2** (Claim Validation & Creation)
   - Core functionality for users to claim swaps
   - Can test with CLI before GUI

4. **Then Phase 2.3** (State Updates)
   - Automated monitoring and updates
   - Critical for user experience

5. **Then Phase 3** (User Interface)
   - Makes swaps accessible to end users
   - Can start with CLI, then add GUI

6. **Finally Phase 4 & 5** (Advanced Features & Testing)
   - Polish and production readiness
   - Comprehensive testing and monitoring

## File Structure

```
lib/
  parent_chain/
    mod.rs          # Module exports and utilities
    config.rs       # Configuration management
    client.rs       # Parent chain client abstraction
    swap.rs         # Swap implementation

app/
  cli.rs            # Updated with parent chain config options
```

## Usage Example

```rust
use plain_bitassets::parent_chain::{
    ParentChainConfig, ParentChainType, SwapManager, default_confirmations
};
use plain_bitassets::parent_chain::client::TxId;

// Create swap manager
let mut swap_manager = SwapManager::new();

// Create a swap for BTC deposit
let l1_txid = TxId::Hash32([...]); // BTC transaction ID
let swap_id = swap_manager.create_swap(
    ParentChainType::Btc,
    l1_txid,
    None, // Use default confirmations
    recipient_address,
    amount,
    current_height,
);

// Update swap state (typically called in a background task)
swap_manager.update_all_swaps(&parent_chain_client, current_height)?;

// Check if swap is ready
if let Some(swap) = swap_manager.get_swap(&swap_id) {
    if matches!(swap.state, SwapState::ReadyToClaim) {
        // Claim the L2 coins
    }
}
```

## Node Requirements

To run CoinShift with full functionality, you need to run full nodes for:
- BTC (Bitcoin Core)
- BCH (Bitcoin Cash Node)
- LTC (Litecoin Core)
- XMR (Monero daemon)
- ETH (Ethereum node, e.g., Geth)
- Tron (Tron node)

**Note**: This requires significant resources and storage. Consider using:
- Pruned nodes where possible
- Light clients for some chains (if supported)
- Remote RPC endpoints (with appropriate security)

## Security Considerations

1. **Node Security**: Ensure parent chain nodes are properly secured and authenticated
2. **Confirmation Requirements**: Default to conservative confirmation counts
3. **Swap Expiration**: Implement appropriate expiration mechanisms
4. **Double-Spending**: Monitor for transaction reversals (especially on chains with reorgs)
5. **Multiple Claimers**: Address the first-claimer problem before production use

## Future Enhancements

- Support for additional chains
- Atomic swap patterns
- Cross-chain liquidity pools
- Automated market making
- Integration with DEX protocols

