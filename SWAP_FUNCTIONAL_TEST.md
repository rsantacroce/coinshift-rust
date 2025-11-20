# Swap Console Tool & Functional Test Design

This document describes the design and implementation plan for swap functionality as a **production-ready console tool** for users, as well as functional tests to verify the complete L2 → L1 swap flow.

## Test Scenario: Alice & Bob Swap

**Goal**: Test the complete flow of an L2 → L1 swap where:
- Alice has L2 coins and wants BTC
- Bob has BTC and wants L2 coins
- They execute a trustless swap

## Algorithm Flow

### Phase 1: Setup & Initial State

```
1. Start sidechain node with parent chain client configured
2. Alice deposits some L2 coins (via BIP300 deposit mechanism)
3. Verify Alice has L2 coins in her wallet
4. Bob also has some L2 coins (for receiving Alice's coins)
```

### Phase 2: Alice Creates Swap Offer

```
1. Alice creates a SwapCreate transaction:
   - Parent chain: BTC
   - L1 recipient address: Alice's BTC address
   - L1 amount required: 0.001 BTC (100,000 sats)
   - L2 amount offered: 100,000 L2 sats
   - L2 recipient: Bob's L2 address
   - Required confirmations: 3

2. Transaction validation:
   ✓ Verify Alice has at least 100,000 L2 sats
   ✓ Create transaction that spends Alice's coins
   ✓ Lock outputs to swap ID
   ✓ Submit to mempool

3. Mine block to include SwapCreate transaction:
   ✓ Verify swap is saved in state
   ✓ Verify Alice's coins are locked
   ✓ Verify swap state is Pending
```

### Phase 3: Bob Sends BTC to Alice

```
1. Bob creates Bitcoin transaction:
   - Send 0.001 BTC to Alice's BTC address
   - Get transaction ID

2. Update swap with L1 transaction ID:
   - Bob calls RPC to update swap with his BTC txid
   - System verifies BTC transaction exists
   - System verifies amount matches (0.001 BTC)
   - System verifies recipient address matches

3. Swap state transitions:
   - Pending → WaitingConfirmations (current: 0, required: 3)
```

### Phase 4: System Monitors Confirmations

```
1. Background task (SwapManager.update_all_swaps) runs periodically:
   - Queries parent chain client for transaction status
   - Updates confirmation count
   - State: WaitingConfirmations (current: 1, required: 3)
   - State: WaitingConfirmations (current: 2, required: 3)
   - State: WaitingConfirmations (current: 3, required: 3)
   - State: ReadyToClaim (when confirmations >= required)

2. Verify swap state progression
```

### Phase 5: Bob Claims Alice's L2 Coins

```
1. Bob creates SwapClaim transaction:
   - Swap ID: from Phase 2
   - Inputs: locked outputs from SwapCreate
   - Outputs: send coins to Bob's L2 address

2. Transaction validation:
   ✓ Verify swap exists
   ✓ Verify swap state is ReadyToClaim
   ✓ Verify inputs are locked to this swap
   ✓ Verify outputs send to swap.l2_recipient (Bob)
   ✓ Unlock outputs

3. Mine block to include SwapClaim transaction:
   ✓ Verify swap state is Completed
   ✓ Verify Bob received Alice's L2 coins
   ✓ Verify Alice's coins are unlocked (spent)
```

### Phase 6: Verification

```
1. Verify final state:
   ✓ Alice has BTC in her BTC address
   ✓ Bob has Alice's L2 coins
   ✓ Swap is marked as Completed
   ✓ No locked outputs remain
```

## Implementation Plan

### Step 1: Add RPC Endpoints

Add the following RPC endpoints to `app/rpc_server.rs`:

```rust
// Create a new L2 → L1 swap
async fn create_swap(
    parent_chain: ParentChainType,
    l1_recipient_address: String,
    l1_amount_sats: u64,
    l2_recipient: Address,
    l2_amount_sats: u64,
    required_confirmations: Option<u32>,
) -> RpcResult<SwapId>

// Update swap with L1 transaction ID (when Bob sends BTC)
async fn update_swap_l1_txid(
    swap_id: SwapId,
    l1_txid: String, // hex encoded
) -> RpcResult<()>

// Get swap status
async fn get_swap_status(swap_id: SwapId) -> RpcResult<Swap>

// Claim swap (create SwapClaim transaction)
async fn claim_swap(swap_id: SwapId) -> RpcResult<Txid>

// List all swaps
async fn list_swaps() -> RpcResult<Vec<Swap>>
```

### Step 2: Add CLI Commands

Add commands to `cli/lib.rs`:

```rust
/// Create a new L2 → L1 swap
CreateSwap {
    #[arg(long)]
    parent_chain: ParentChainType,
    #[arg(long)]
    l1_recipient_address: String,
    #[arg(long)]
    l1_amount_sats: u64,
    #[arg(long)]
    l2_recipient: Address,
    #[arg(long)]
    l2_amount_sats: u64,
    #[arg(long)]
    required_confirmations: Option<u32>,
},
/// Update swap with L1 transaction ID
UpdateSwapL1Txid {
    #[arg(long)]
    swap_id: String, // hex encoded
    #[arg(long)]
    l1_txid: String, // hex encoded
},
/// Get swap status
GetSwapStatus {
    #[arg(long)]
    swap_id: String, // hex encoded
},
/// Claim a swap
ClaimSwap {
    #[arg(long)]
    swap_id: String, // hex encoded
},
/// List all swaps
ListSwaps,
```

### Step 3: Wallet Integration

Add methods to `lib/wallet.rs`:

```rust
/// Create a SwapCreate transaction
pub fn create_swap_create_tx(
    &self,
    tx: &mut Transaction,
    swap_id: SwapId,
    parent_chain: ParentChainType,
    l1_txid_bytes: Vec<u8>,
    required_confirmations: u32,
    l2_recipient: Address,
    l2_amount: bitcoin::Amount,
    l1_recipient_address: Option<String>,
    l1_amount: Option<bitcoin::Amount>,
) -> Result<(), Error>

/// Create a SwapClaim transaction
pub fn create_swap_claim_tx(
    &self,
    tx: &mut Transaction,
    swap_id: SwapId,
    locked_outputs: Vec<OutPoint>, // Outputs locked to this swap
    recipient: Address, // Where to send the coins
) -> Result<(), Error>
```

### Step 4: Create Functional Test Script

Create `integration_tests/swap_functional_test.rs`:

```rust
// Test steps:
// 1. Setup: Start node, create wallets for Alice and Bob
// 2. Deposit: Alice deposits L2 coins
// 3. Create Swap: Alice creates swap offer
// 4. Send BTC: Bob sends BTC to Alice (simulated)
// 5. Monitor: Wait for confirmations
// 6. Claim: Bob claims Alice's coins
// 7. Verify: Check final state
```

## Test Execution Flow

### Command Sequence

```bash
# 1. Start node
./coinshift --datadir /tmp/test_node --headless

# 2. Alice gets her addresses
ALICE_L2_ADDR=$(./coinshift-cli get-new-address)
ALICE_BTC_ADDR=$(./coinshift-cli get-new-main-address)

# 3. Alice deposits L2 coins (via BIP300)
./coinshift-cli create-deposit $ALICE_L2_ADDR --value-sats 500000 --fee-sats 1000

# 4. Mine block to include deposit
./coinshift-cli mine

# 5. Verify Alice's balance
./coinshift-cli bitcoin-balance  # Should show ~500k sats

# 6. Bob gets his L2 address
BOB_L2_ADDR=$(./coinshift-cli get-new-address)

# 7. Alice creates swap
SWAP_ID=$(./coinshift-cli create-swap \
    --parent-chain BTC \
    --l1-recipient-address $ALICE_BTC_ADDR \
    --l1-amount-sats 100000 \
    --l2-recipient $BOB_L2_ADDR \
    --l2-amount-sats 100000 \
    --required-confirmations 3)

# 8. Mine block to include SwapCreate
./coinshift-cli mine

# 9. Check swap status (should be Pending)
./coinshift-cli get-swap-status --swap-id $SWAP_ID

# 10. Bob sends BTC to Alice (simulated - in real test, use regtest)
# This would be done via Bitcoin RPC or test framework
BTC_TXID="<simulated BTC transaction ID>"

# 11. Update swap with L1 transaction ID
./coinshift-cli update-swap-l1-txid \
    --swap-id $SWAP_ID \
    --l1-txid $BTC_TXID

# 12. Wait for confirmations (simulated)
# In real test, mine Bitcoin blocks or wait
sleep 30  # Wait for background task to update

# 13. Check swap status (should be ReadyToClaim)
./coinshift-cli get-swap-status --swap-id $SWAP_ID

# 14. Bob claims swap
CLAIM_TXID=$(./coinshift-cli claim-swap --swap-id $SWAP_ID)

# 15. Mine block to include SwapClaim
./coinshift-cli mine

# 16. Verify final state
./coinshift-cli get-swap-status --swap-id $SWAP_ID  # Should be Completed
./coinshift-cli bitcoin-balance  # Bob should have 100k sats
```

## Algorithm Verification

### Swap State Machine

```
Pending
  ↓ (L1 tx detected via set_l1_txid)
WaitingConfirmations { current: 0, required: 3 }
  ↓ (confirmations increase via update_state)
WaitingConfirmations { current: 1, required: 3 }
  ↓
WaitingConfirmations { current: 2, required: 3 }
  ↓ (confirmations >= required)
ReadyToClaim
  ↓ (SwapClaim transaction)
Completed
```

### Key Algorithm Details

1. **Swap ID Generation** (for L2→L1):
   ```rust
   hash(l1_recipient_address || l1_amount || l2_sender || l2_recipient)
   ```
   - Deterministic: Same parameters = same swap ID
   - Prevents duplicate swaps

2. **Coin Locking**:
   - When SwapCreate is processed, ALL outputs are locked to swap ID
   - Locked outputs can ONLY be spent by SwapClaim for that swap
   - Prevents double-spending

3. **L1 Transaction ID Update**:
   - Initially: placeholder (all zeros) for L2→L1 swaps
   - Bob calls `set_l1_txid()` when he sends BTC
   - Must be in `Pending` state to update
   - After update, background task detects transaction

4. **State Updates**:
   - Background task runs every 30 seconds
   - Calls `update_state()` for each swap
   - Queries parent chain client for transaction status
   - Updates confirmation count automatically
   - Saves updated state to database

### Coin Locking Flow

```
1. SwapCreate:
   - Inputs: Alice's UTXOs (100k sats)
   - Outputs: Locked to swap ID
   - State: Locked outputs in database

2. SwapClaim:
   - Inputs: Locked outputs (must be locked to swap)
   - Outputs: Bob's address (100k sats)
   - State: Unlock outputs, mark swap completed
```

### Validation Checks

1. **SwapCreate Validation**:
   - ✓ Transaction spends at least `l2_amount`
   - ✓ No inputs are already locked
   - ✓ Swap ID is correctly computed
   - ✓ Outputs are locked to swap

2. **SwapClaim Validation**:
   - ✓ Swap exists
   - ✓ Swap state is ReadyToClaim
   - ✓ Inputs are locked to this swap
   - ✓ Outputs go to swap.l2_recipient

3. **State Updates**:
   - ✓ Background task updates swap states
   - ✓ Confirmations are tracked correctly
   - ✓ State transitions are valid

## Testing Considerations

### Mock Parent Chain Client

For testing, we need a mock parent chain client that can:
- Simulate Bitcoin transactions
- Return configurable confirmation counts
- Allow manual confirmation updates

### Test Data

- Use regtest Bitcoin network
- Use test wallets with known seeds
- Pre-fund addresses for testing

### Error Cases

Test error scenarios:
- SwapCreate with insufficient funds
- SwapClaim before ReadyToClaim
- Spending locked outputs in regular transaction
- Invalid L1 transaction ID
- Wrong L1 amount or address

## Next Steps

1. ✅ Implement coin locking (DONE)
2. ⏳ Add RPC endpoints for swap operations
3. ⏳ Add CLI commands
4. ⏳ Add wallet methods for swap transactions
5. ⏳ Create functional test script
6. ⏳ Test with mock parent chain client
7. ⏳ Test with real regtest Bitcoin node

