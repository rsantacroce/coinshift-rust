# CoinShift 1.0 Architecture

This document describes the organization and architecture changes made to support CoinShift 1.0, a trustless swap system for the sidechain.

## Overview

CoinShift 1.0 extends the existing sidechain codebase to support:
- Multiple parent chain support (BTC, BCH, LTC, XMR, ETH, Tron)
- Trustless swapping of L2 coins with parent chain assets
- Conditional payments based on L1 transaction confirmations
- Configurable confirmation requirements (default: ~45 minutes of PoW per chain)

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

The swap system implements conditional payments: "will pay L2 coins, iff a specific L1 transaction exists and gets X confirmations."

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

