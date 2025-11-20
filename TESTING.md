# Swap Functionality Tests

This document describes the test suite for the swap functionality implemented in Phase 1.

## Test Coverage

### Unit Tests

#### Swap Module Tests (`lib/parent_chain/swap.rs`)

1. **`test_swap_creation`**
   - Tests basic swap creation with all required parameters
   - Verifies swap is initialized in `Pending` state
   - Checks default confirmation count is used when not specified

2. **`test_swap_id_deterministic`**
   - Verifies swap ID generation is deterministic
   - Tests that different recipients produce different swap IDs
   - Ensures same parameters produce same swap ID

3. **`test_swap_id_same_for_same_params`**
   - Confirms swap ID consistency for identical parameters

4. **`test_swap_mark_completed`**
   - Tests state transition from `ReadyToClaim` to `Completed`
   - Verifies error when trying to mark completed from invalid states

5. **`test_swap_manager_create`**
   - Tests SwapManager creation and swap insertion
   - Verifies swap retrieval from manager

6. **`test_swap_expiration`**
   - Tests swap expiration height setting

7. **`test_swap_custom_confirmations`**
   - Tests custom confirmation count override

#### State Module Tests (`lib/state/mod.rs` - `swap_tests` module)

1. **`test_save_and_load_swap`**
   - Tests saving swap to database
   - Tests loading swap from database
   - Verifies all swap fields are persisted correctly

2. **`test_get_swap_by_l1_txid`**
   - Tests lookup by parent chain and L1 transaction ID
   - Verifies index is maintained correctly

3. **`test_get_swaps_by_recipient`**
   - Tests lookup by recipient address
   - Verifies multiple swaps can be associated with one recipient
   - Tests index maintenance for recipient lookups

4. **`test_delete_swap`**
   - Tests swap deletion from all indices
   - Verifies swap is removed from main table and all lookup tables

5. **`test_load_all_swaps`**
   - Tests loading all swaps from database
   - Verifies multiple swaps can be stored and retrieved

## Running Tests

### Run all swap-related unit tests:
```bash
cargo test --package plain_bitassets --lib parent_chain::swap::tests
cargo test --package plain_bitassets --lib state::swap_tests
```

### Run all tests:
```bash
cargo test --package plain_bitassets
```

## Test Dependencies

- `tempfile`: Used for creating temporary database environments in state tests
  - Added as dev-dependency in `lib/Cargo.toml`

## Future Test Additions

The following tests should be added as the swap functionality is extended:

### Integration Tests (Phase 5.2)
- [ ] Test full swap flow with mock parent chain client
- [ ] Test swap with real parent chain node (BTC)
- [ ] Test swap claim transaction submission
- [ ] Test swap state updates during block processing
- [ ] Test swap rollback scenarios

### Transaction Validation Tests
- [ ] Test SwapCreate transaction validation
- [ ] Test SwapClaim transaction validation
- [ ] Test invalid swap ID rejection
- [ ] Test duplicate swap creation rejection
- [ ] Test claim validation for non-ready swaps

### State Transition Tests
- [ ] Test swap state transitions with mock client
- [ ] Test expiration handling
- [ ] Test transaction disappearance handling

## Notes

- Tests use temporary directories for database operations to ensure isolation
- Mock parent chain clients will be needed for full integration testing
- Some tests require async runtime (tokio) for testing state update logic

