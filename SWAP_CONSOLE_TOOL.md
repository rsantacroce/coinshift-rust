# Swap Console Tool Implementation Plan

This document outlines the implementation of swap functionality as a production-ready console tool, not just a test.

## Overview

Users will be able to use the sidechain swap functionality via command-line tools, enabling:
- Creating swap offers (L2 → L1)
- Filling swaps by sending L1 payments
- Claiming swaps after confirmations
- Monitoring swap status

## Implementation Components

### 1. RPC API Endpoints (rpc-api/lib.rs)

Added methods:
- `create_swap` - Create a new L2 → L1 swap
- `update_swap_l1_txid` - Update swap with L1 transaction ID
- `get_swap_status` - Get current swap status
- `claim_swap` - Claim a swap (create SwapClaim transaction)
- `list_swaps` - List all swaps

### 2. RPC Server Implementation (app/rpc_server.rs)

Each endpoint will:
- Validate inputs
- Create/update transactions via wallet
- Submit to mempool
- Return appropriate responses

### 3. Wallet Methods (lib/wallet.rs)

New methods needed:
- `create_swap_create_tx` - Create SwapCreate transaction
- `create_swap_claim_tx` - Create SwapClaim transaction
- `find_locked_outputs_for_swap` - Find outputs locked to a swap

### 4. CLI Commands (cli/lib.rs)

User-friendly commands:
- `create-swap` - Create a swap offer
- `update-swap` - Update swap with L1 txid
- `swap-status` - Check swap status
- `claim-swap` - Claim a swap
- `list-swaps` - List all swaps

## Usage Examples

### Alice Creates a Swap Offer

```bash
# Alice wants to exchange 100k L2 sats for 0.001 BTC
coinshift-cli create-swap \
    --parent-chain BTC \
    --l1-recipient-address bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh \
    --l1-amount-sats 100000 \
    --l2-recipient <BOB_L2_ADDRESS> \
    --l2-amount-sats 100000 \
    --required-confirmations 3

# Output: Swap ID (hex)
# Swap created! Your coins are locked. Share this swap ID with the filler.
```

### Bob Fills the Swap

```bash
# 1. Bob sends BTC to Alice's address (via Bitcoin wallet)
# Gets transaction ID: abc123...

# 2. Bob updates the swap with the transaction ID
coinshift-cli update-swap \
    --swap-id <SWAP_ID> \
    --l1-txid abc123...

# Output: Swap updated! Waiting for confirmations...
```

### Monitor Swap Status

```bash
# Check swap status
coinshift-cli swap-status --swap-id <SWAP_ID>

# Output:
# Swap ID: <SWAP_ID>
# State: WaitingConfirmations (2/3)
# L1 Transaction: abc123...
# L1 Amount: 100000 sats
# L2 Amount: 100000 sats
# Created: Block 1234
```

### Bob Claims the Swap

```bash
# After confirmations are met
coinshift-cli claim-swap --swap-id <SWAP_ID>

# Output: Transaction ID: xyz789...
# Swap claimed! Your L2 coins will be available after the next block.
```

### List All Swaps

```bash
coinshift-cli list-swaps

# Output:
# Swap 1: <ID> - Pending - 100k sats for 0.001 BTC
# Swap 2: <ID> - ReadyToClaim - 50k sats for 0.0005 BTC
# Swap 3: <ID> - Completed - 200k sats for 0.002 BTC
```

## Implementation Status

- [x] RPC API definitions
- [ ] RPC server implementations
- [ ] Wallet methods for swap transactions
- [ ] CLI commands
- [ ] Error handling and user feedback
- [ ] Documentation

## Next Steps

1. Implement RPC server methods
2. Add wallet transaction creation methods
3. Add CLI commands
4. Test end-to-end flow
5. Add help text and examples

