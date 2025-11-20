# Swap Use Cases

This document describes the swap use cases supported by CoinShift.

## Note: L1 → L2 Deposits

**L1 → L2 deposits are handled by the BIP300 sidechain implementation itself**, not by the swap system. When users send BTC to deposit addresses, the sidechain automatically:
- Detects the deposit transaction
- Creates a `Deposit` event
- Mints corresponding L2 coins as UTXOs
- No swap mechanism is needed

The swap system described below is **only for L2 → L1 swaps** (withdrawals/exchanges between users).

## Use Case: L2 → L1 Swap (Withdrawal/Exchange) - Alice & Bob

**Scenario**: Alice has L2 coins and wants BTC. Bob has BTC and wants L2 coins.

**Flow**:
1. **Alice creates swap offer**: "I'll give 100,000 L2 sats if you send 0.001 BTC to my BTC address"
2. **Bob accepts**: Bob sends 0.001 BTC to Alice's BTC address
3. **System monitors**: System detects Bob's BTC transaction and waits for confirmations
4. **Bob claims**: After confirmations, Bob claims Alice's 100,000 L2 sats

**Detailed Steps**:

### Step 1: Alice Creates Swap Offer
```rust
let swap = Swap::new_l2_to_l1(
    ParentChainType::Btc,
    "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_string(), // Alice's BTC address
    bitcoin::Amount::from_sat(100_000),                        // Bob needs to send 0.001 BTC
    alice_l2_address,                                          // Alice's L2 address (has coins)
    bitcoin::Amount::from_sat(100_000),                       // Alice offers 100k L2 sats
    bob_l2_address,                                            // Bob's L2 address (will receive)
    Some(3),                                                   // 3 confirmations required
    current_height,
);
```

**What happens**:
- Alice's L2 coins are locked/escrowed (via SwapCreate transaction)
- Swap is in `Pending` state waiting for Bob's BTC transaction
- Swap ID is generated from: L1 address, L1 amount, L2 sender, L2 recipient

### Step 2: Bob Sends BTC to Alice
- Bob creates a Bitcoin transaction sending 0.001 BTC to Alice's BTC address
- Bob gets the transaction ID: `bob_btc_txid`
- Bob updates the swap with his transaction ID:
  ```rust
  swap.set_l1_txid(bob_btc_txid)?;
  ```

### Step 3: System Monitors Confirmation
- Background task detects Bob's BTC transaction
- Swap state transitions: `Pending` → `WaitingConfirmations` → `ReadyToClaim`
- After 3 confirmations, swap is ready

### Step 4: Bob Claims Alice's L2 Coins
- Bob creates a `SwapClaim` transaction
- Transaction sends Alice's 100,000 L2 sats to Bob's L2 address
- Swap state changes to `Completed`

**Result**:
- ✅ Alice receives 0.001 BTC in her BTC address
- ✅ Bob receives 100,000 L2 sats in his L2 address
- ✅ Trustless exchange completed!

## How This Differs from Native Deposits

| Aspect | Native L1 → L2 (BIP300) | L2 → L1 Swap (CoinShift) |
|--------|------------------------|--------------------------|
| **Direction** | Deposit | Withdrawal/Exchange |
| **Implementation** | Built into sidechain | CoinShift swap system |
| **Who initiates** | User with L1 coins | User with L2 coins (Alice) |
| **Who fills** | Same user (automatic) | Different user (Bob) |
| **L1 transaction** | User's deposit tx | Filler's payment tx (Bob) |
| **L2 coins** | Minted by sidechain | Transferred from Alice to Bob |
| **L1 recipient** | N/A | Alice's L1 address |
| **L2 recipient** | User's L2 address | Bob's L2 address |
| **Use case** | Getting onto sidechain | Exchanging with other users |

## Security Considerations

1. **Alice's L2 coins must be locked** when she creates the swap
   - This prevents double-spending
   - Implemented via SwapCreate transaction that locks the coins

2. **Bob must send exact amount** to Alice's address
   - System verifies the BTC transaction sends correct amount
   - Amount is specified in the swap

3. **First-come-first-served**
   - First person to send BTC and claim gets the L2 coins
   - Multiple people could try to fill the same swap (see "Multiple First Claimers" problem)

4. **Confirmation requirements**
   - Ensures L1 transaction is final before releasing L2 coins
   - Default: ~45 minutes of PoW security

## Test Coverage

The test `test_alice_bob_l2_to_l1_swap` demonstrates the complete flow:
- Swap creation with Alice's parameters
- Bob setting the L1 transaction ID
- State transitions through confirmations
- Final claim by Bob

Run the test:
```bash
cargo test --package plain_bitassets --lib parent_chain::swap::tests::test_alice_bob_l2_to_l1_swap
```

