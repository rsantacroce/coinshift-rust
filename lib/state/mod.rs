use std::collections::{BTreeMap, HashMap, HashSet};

use fallible_iterator::FallibleIterator;
use futures::Stream;
use heed::types::SerdeBincode;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use sneed::{DatabaseUnique, RoDatabaseUnique, RoTxn, RwTxn, UnitKey};

use crate::{
    authorization::Authorization,
    parent_chain::{swap::Swap, SwapId, client::TxId, config::ParentChainType},
    types::{
        Address, AmountOverflowError, Authorized, AuthorizedTransaction,
        BitAssetId, BlockHash, Body, FilledOutput, FilledTransaction,
        GetAddress as _, GetBitcoinValue as _, Header, InPoint, M6id, OutPoint,
        OutPointKey, SpentOutput, Transaction, TxData, VERSION, Verify as _,
        Version, WithdrawalBundle, WithdrawalBundleStatus,
        proto::mainchain::TwoWayPegData,
    },
    util::Watchable,
};

mod amm;
pub mod bitassets;
mod block;
mod dutch_auction;
pub mod error;
mod rollback;
mod two_way_peg_data;

pub use amm::{AmmPair, PoolState as AmmPoolState};
pub use bitassets::SeqId as BitAssetSeqId;
pub use dutch_auction::DutchAuctionState;
pub use error::Error;
use rollback::{HeightStamped, RollBack};

pub const WITHDRAWAL_BUNDLE_FAILURE_GAP: u32 = 4;

/// Prevalidated block data containing computed values from validation
/// to avoid redundant computation during connection
pub struct PrevalidatedBlock {
    pub filled_transactions: Vec<FilledTransaction>,
    pub computed_merkle_root: crate::types::BlockHash,
    pub total_fees: bitcoin::Amount,
    pub coinbase_value: bitcoin::Amount,
    pub next_height: u32, // Precomputed next height to avoid DB read in write txn
}

/// Information we have regarding a withdrawal bundle
#[derive(Debug, Deserialize, Serialize)]
enum WithdrawalBundleInfo {
    /// Withdrawal bundle is known
    Known(WithdrawalBundle),
    /// Withdrawal bundle is unknown but unconfirmed / failed
    Unknown,
    /// If an unknown withdrawal bundle is confirmed, ALL UTXOs are
    /// considered spent.
    UnknownConfirmed {
        spend_utxos: BTreeMap<OutPoint, FilledOutput>,
    },
}

impl WithdrawalBundleInfo {
    fn is_known(&self) -> bool {
        match self {
            Self::Known(_) => true,
            Self::Unknown | Self::UnknownConfirmed { .. } => false,
        }
    }
}

type WithdrawalBundlesDb = DatabaseUnique<
    SerdeBincode<M6id>,
    SerdeBincode<(
        WithdrawalBundleInfo,
        RollBack<HeightStamped<WithdrawalBundleStatus>>,
    )>,
>;

#[derive(Clone)]
pub struct State {
    /// Current tip
    tip: DatabaseUnique<UnitKey, SerdeBincode<BlockHash>>,
    /// Current height
    height: DatabaseUnique<UnitKey, SerdeBincode<u32>>,
    /// Associates ordered pairs of BitAssets to their AMM pool states
    amm_pools: amm::PoolsDb,
    bitassets: bitassets::Dbs,
    /// Associates Dutch auction sequence numbers with auction state
    dutch_auctions: dutch_auction::Db,
    utxos: DatabaseUnique<OutPointKey, SerdeBincode<FilledOutput>>,
    stxos: DatabaseUnique<OutPointKey, SerdeBincode<SpentOutput>>,
    /// Pending withdrawal bundle and block height
    pending_withdrawal_bundle:
        DatabaseUnique<UnitKey, SerdeBincode<(WithdrawalBundle, u32)>>,
    /// Latest failed (known) withdrawal bundle
    latest_failed_withdrawal_bundle:
        DatabaseUnique<UnitKey, SerdeBincode<RollBack<HeightStamped<M6id>>>>,
    /// Withdrawal bundles and their status.
    /// Some withdrawal bundles may be unknown.
    /// in which case they are `None`.
    withdrawal_bundles: WithdrawalBundlesDb,
    /// Deposit blocks and the height at which they were applied, keyed sequentially
    deposit_blocks: DatabaseUnique<
        SerdeBincode<u32>,
        SerdeBincode<(bitcoin::BlockHash, u32)>,
    >,
    /// Withdrawal bundle event blocks and the height at which they were applied, keyed sequentially
    withdrawal_bundle_event_blocks: DatabaseUnique<
        SerdeBincode<u32>,
        SerdeBincode<(bitcoin::BlockHash, u32)>,
    >,
    /// Active swaps keyed by swap ID
    swaps: DatabaseUnique<SerdeBincode<SwapId>, SerdeBincode<Swap>>,
    /// Lookup swap ID by parent chain and L1 transaction ID
    swaps_by_l1_txid: DatabaseUnique<
        SerdeBincode<(ParentChainType, TxId)>,
        SerdeBincode<SwapId>,
    >,
    /// Lookup swap IDs by recipient address
    swaps_by_recipient: DatabaseUnique<
        SerdeBincode<Address>,
        SerdeBincode<Vec<SwapId>>,
    >,
    /// Outputs locked to swaps (can only be spent by SwapClaim)
    /// Maps OutPoint -> SwapId for L2 → L1 swaps
    locked_swap_outputs: DatabaseUnique<
        SerdeBincode<OutPointKey>,
        SerdeBincode<SwapId>,
    >,
    _version: DatabaseUnique<UnitKey, SerdeBincode<Version>>,
}

impl State {
    pub const NUM_DBS: u32 = bitassets::Dbs::NUM_DBS + 16; // Added 3 swap databases + 1 locked outputs database

    pub fn new(env: &sneed::Env) -> Result<Self, Error> {
        let mut rwtxn = env.write_txn()?;
        let tip = DatabaseUnique::create(env, &mut rwtxn, "tip")?;
        let height = DatabaseUnique::create(env, &mut rwtxn, "height")?;
        let amm_pools = DatabaseUnique::create(env, &mut rwtxn, "amm_pools")?;
        let bitassets = bitassets::Dbs::new(env, &mut rwtxn)?;
        let dutch_auctions =
            DatabaseUnique::create(env, &mut rwtxn, "dutch_auctions")?;
        let utxos = DatabaseUnique::create(env, &mut rwtxn, "utxos")?;
        let stxos = DatabaseUnique::create(env, &mut rwtxn, "stxos")?;
        let pending_withdrawal_bundle = DatabaseUnique::create(
            env,
            &mut rwtxn,
            "pending_withdrawal_bundle",
        )?;
        let latest_failed_withdrawal_bundle = DatabaseUnique::create(
            env,
            &mut rwtxn,
            "latest_failed_withdrawal_bundle",
        )?;
        let withdrawal_bundles =
            DatabaseUnique::create(env, &mut rwtxn, "withdrawal_bundles")?;
        let deposit_blocks =
            DatabaseUnique::create(env, &mut rwtxn, "deposit_blocks")?;
        let withdrawal_bundle_event_blocks = DatabaseUnique::create(
            env,
            &mut rwtxn,
            "withdrawal_bundle_event_blocks",
        )?;
        let swaps = DatabaseUnique::create(env, &mut rwtxn, "swaps")?;
        let swaps_by_l1_txid = DatabaseUnique::create(
            env,
            &mut rwtxn,
            "swaps_by_l1_txid",
        )?;
        let swaps_by_recipient = DatabaseUnique::create(
            env,
            &mut rwtxn,
            "swaps_by_recipient",
        )?;
        let locked_swap_outputs = DatabaseUnique::create(
            env,
            &mut rwtxn,
            "locked_swap_outputs",
        )?;
        let version = DatabaseUnique::create(env, &mut rwtxn, "state_version")?;
        if version.try_get(&rwtxn, &())?.is_none() {
            version.put(&mut rwtxn, &(), &*VERSION)?;
        }
        rwtxn.commit()?;
        Ok(Self {
            tip,
            height,
            amm_pools,
            bitassets,
            dutch_auctions,
            utxos,
            stxos,
            pending_withdrawal_bundle,
            latest_failed_withdrawal_bundle,
            withdrawal_bundles,
            withdrawal_bundle_event_blocks,
            deposit_blocks,
            swaps,
            swaps_by_l1_txid,
            swaps_by_recipient,
            locked_swap_outputs,
            _version: version,
        })
    }

    pub fn amm_pools(&self) -> &amm::RoPoolsDb {
        &self.amm_pools
    }

    pub fn bitassets(&self) -> &bitassets::Dbs {
        &self.bitassets
    }

    pub fn deposit_blocks(
        &self,
    ) -> &RoDatabaseUnique<
        SerdeBincode<u32>,
        SerdeBincode<(bitcoin::BlockHash, u32)>,
    > {
        &self.deposit_blocks
    }

    pub fn dutch_auctions(&self) -> &dutch_auction::RoDb {
        &self.dutch_auctions
    }

    /// Get swap by ID
    pub fn get_swap(
        &self,
        rotxn: &RoTxn,
        swap_id: &SwapId,
    ) -> Result<Option<Swap>, Error> {
        Ok(self.swaps.try_get(rotxn, swap_id)?)
    }

    /// Get swap by parent chain and L1 transaction ID
    pub fn get_swap_by_l1_txid(
        &self,
        rotxn: &RoTxn,
        parent_chain: &ParentChainType,
        l1_txid: &TxId,
    ) -> Result<Option<Swap>, Error> {
        let key = (parent_chain.clone(), l1_txid.clone());
        if let Some(swap_id) = self.swaps_by_l1_txid.try_get(rotxn, &key)? {
            self.get_swap(rotxn, &swap_id)
        } else {
            Ok(None)
        }
    }

    /// Get all swaps for a recipient address
    pub fn get_swaps_by_recipient(
        &self,
        rotxn: &RoTxn,
        recipient: &Address,
    ) -> Result<Vec<Swap>, Error> {
        if let Some(swap_ids) = self.swaps_by_recipient.try_get(rotxn, recipient)? {
            let mut swaps = Vec::new();
            for swap_id in swap_ids {
                if let Some(swap) = self.swaps.try_get(rotxn, &swap_id)? {
                    swaps.push(swap);
                }
            }
            Ok(swaps)
        } else {
            Ok(Vec::new())
        }
    }

    /// Save a swap to the database
    pub fn save_swap(
        &self,
        rwtxn: &mut RwTxn,
        swap: &Swap,
    ) -> Result<(), Error> {
        // Save swap by ID
        self.swaps.put(rwtxn, &swap.id, swap)?;

        // Save lookup by L1 txid
        let l1_key = (swap.parent_chain.clone(), swap.l1_txid.clone());
        self.swaps_by_l1_txid.put(rwtxn, &l1_key, &swap.id)?;

        // Update recipient index
        let mut swap_ids = self
            .swaps_by_recipient
            .try_get(rwtxn, &swap.l2_recipient)?
            .unwrap_or_default();
        if !swap_ids.contains(&swap.id) {
            swap_ids.push(swap.id.clone());
            self.swaps_by_recipient.put(rwtxn, &swap.l2_recipient, &swap_ids)?;
        }

        Ok(())
    }

    /// Delete a swap from the database
    pub fn delete_swap(
        &self,
        rwtxn: &mut RwTxn,
        swap_id: &SwapId,
    ) -> Result<(), Error> {
        if let Some(swap) = self.swaps.try_get(rwtxn, swap_id)? {
            // Delete from main swaps table
            self.swaps.delete(rwtxn, swap_id)?;

            // Delete from L1 txid lookup
            let l1_key = (swap.parent_chain.clone(), swap.l1_txid.clone());
            self.swaps_by_l1_txid.delete(rwtxn, &l1_key)?;

            // Update recipient index
            if let Some(mut swap_ids) = self
                .swaps_by_recipient
                .try_get(rwtxn, &swap.l2_recipient)?
            {
                swap_ids.retain(|id| id != swap_id);
                if swap_ids.is_empty() {
                    self.swaps_by_recipient.delete(rwtxn, &swap.l2_recipient)?;
                } else {
                    self.swaps_by_recipient.put(rwtxn, &swap.l2_recipient, &swap_ids)?;
                }
            }
        }
        Ok(())
    }

    /// Load all swaps from database
    pub fn load_all_swaps(&self, rotxn: &RoTxn) -> Result<Vec<Swap>, Error> {
        let swaps: Vec<Swap> = self
            .swaps
            .iter(rotxn)?
            .map(|(_, swap)| Ok(swap))
            .collect()?;
        Ok(swaps)
    }

    pub fn stxos(
        &self,
    ) -> &RoDatabaseUnique<OutPointKey, SerdeBincode<SpentOutput>> {
        &self.stxos
    }

    pub fn withdrawal_bundle_event_blocks(
        &self,
    ) -> &RoDatabaseUnique<
        SerdeBincode<u32>,
        SerdeBincode<(bitcoin::BlockHash, u32)>,
    > {
        &self.withdrawal_bundle_event_blocks
    }

    pub fn try_get_tip(
        &self,
        rotxn: &RoTxn,
    ) -> Result<Option<BlockHash>, Error> {
        let tip = self.tip.try_get(rotxn, &())?;
        Ok(tip)
    }

    pub fn try_get_height(&self, rotxn: &RoTxn) -> Result<Option<u32>, Error> {
        let height = self.height.try_get(rotxn, &())?;
        Ok(height)
    }

    pub fn get_utxos(
        &self,
        rotxn: &RoTxn,
    ) -> Result<HashMap<OutPoint, FilledOutput>, Error> {
        let utxos: HashMap<OutPoint, FilledOutput> = self
            .utxos
            .iter(rotxn)?
            .map(|(key, output)| Ok((key.to_outpoint(), output)))
            .collect()?;
        Ok(utxos)
    }

    pub fn get_utxos_by_addresses(
        &self,
        rotxn: &RoTxn,
        addresses: &HashSet<Address>,
    ) -> Result<HashMap<OutPoint, FilledOutput>, Error> {
        let utxos: HashMap<OutPoint, FilledOutput> = self
            .utxos
            .iter(rotxn)?
            .filter(|(_, output)| Ok(addresses.contains(&output.address)))
            .map(|(key, output)| Ok((key.to_outpoint(), output)))
            .collect()?;
        Ok(utxos)
    }

    /// Get the latest failed withdrawal bundle, and the height at which it failed
    pub fn get_latest_failed_withdrawal_bundle(
        &self,
        rotxn: &RoTxn,
    ) -> Result<Option<(u32, M6id)>, Error> {
        let Some(latest_failed_m6id) =
            self.latest_failed_withdrawal_bundle.try_get(rotxn, &())?
        else {
            return Ok(None);
        };
        let latest_failed_m6id = latest_failed_m6id.latest().value;
        let (_bundle, bundle_status) = self.withdrawal_bundles.try_get(rotxn, &latest_failed_m6id)?
            .expect("Inconsistent DBs: latest failed m6id should exist in withdrawal_bundles");
        let bundle_status = bundle_status.latest();
        assert_eq!(bundle_status.value, WithdrawalBundleStatus::Failed);
        Ok(Some((bundle_status.height, latest_failed_m6id)))
    }

    pub fn fill_transaction(
        &self,
        rotxn: &RoTxn,
        transaction: &Transaction,
    ) -> Result<FilledTransaction, Error> {
        let mut spent_utxos = Vec::with_capacity(transaction.inputs.len());
        for input in &transaction.inputs {
            let key = OutPointKey::from_outpoint(input);
            let utxo = self
                .utxos
                .try_get(rotxn, &key)?
                .ok_or(Error::NoUtxo { outpoint: *input })?;
            spent_utxos.push(utxo);
        }
        Ok(FilledTransaction {
            spent_utxos,
            transaction: transaction.clone(),
        })
    }

    /// Fill a transaction that has already been applied
    pub fn fill_transaction_from_stxos(
        &self,
        rotxn: &RoTxn,
        tx: Transaction,
    ) -> Result<FilledTransaction, Error> {
        let txid = tx.txid();
        let mut spent_utxos = Vec::with_capacity(tx.inputs.len());
        // fill inputs last-to-first
        for (vin, input) in tx.inputs.iter().enumerate().rev() {
            let key = OutPointKey::from_outpoint(input);
            let stxo = self
                .stxos
                .try_get(rotxn, &key)?
                .ok_or(Error::NoStxo { outpoint: *input })?;
            assert_eq!(
                stxo.inpoint,
                InPoint::Regular {
                    txid,
                    vin: vin as u32
                }
            );
            spent_utxos.push(stxo.output);
        }
        spent_utxos.reverse();
        Ok(FilledTransaction {
            spent_utxos,
            transaction: tx,
        })
    }

    pub fn fill_authorized_transaction(
        &self,
        rotxn: &RoTxn,
        transaction: AuthorizedTransaction,
    ) -> Result<Authorized<FilledTransaction>, Error> {
        let filled_tx =
            self.fill_transaction(rotxn, &transaction.transaction)?;
        let authorizations = transaction.authorizations;
        Ok(Authorized {
            transaction: filled_tx,
            authorizations,
        })
    }

    /// Get pending withdrawal bundle and block height
    pub fn get_pending_withdrawal_bundle(
        &self,
        txn: &RoTxn,
    ) -> Result<Option<(WithdrawalBundle, u32)>, Error> {
        Ok(self.pending_withdrawal_bundle.try_get(txn, &())?)
    }

    /// Check that
    /// * If the tx is a BitAsset reservation, then the number of bitasset
    ///   reservations in the outputs is exactly one more than the number of
    ///   bitasset reservations in the inputs.
    /// * If the tx is a BitAsset
    ///   registration, then the number of bitasset reservations in the outputs
    ///   is exactly one less than the number of bitasset reservations in the
    ///   inputs.
    /// * Otherwise, the number of bitasset reservations in the outputs
    ///   is exactly equal to the number of bitasset reservations in the inputs.
    pub fn validate_reservations(
        &self,
        tx: &FilledTransaction,
    ) -> Result<(), Error> {
        let n_reservation_inputs: usize = tx.spent_reservations().count();
        let n_reservation_outputs: usize = tx.reservation_outputs().count();
        if tx.is_reservation() {
            if n_reservation_outputs == n_reservation_inputs + 1 {
                return Ok(());
            }
        } else if tx.is_registration() {
            if n_reservation_inputs == n_reservation_outputs + 1 {
                return Ok(());
            }
        } else if n_reservation_inputs == n_reservation_outputs {
            return Ok(());
        }
        Err(Error::UnbalancedReservations {
            n_reservation_inputs,
            n_reservation_outputs,
        })
    }

    /** Check that
     *  * If the tx is a BitAsset registration, then
     *    * The number of BitAsset control coins in the outputs is exactly
     *      one more than the number of BitAsset control coins in the
     *      inputs
     *    * The number of BitAsset outputs is at least
     *      * The number of unique BitAsset inputs,
     *        if the initial supply is zero
     *      * One more than the number of unique BitAsset inputs,
     *        if the initial supply is nonzero.
     *    * The newly registered BitAsset must have been unregistered,
     *      prior to the registration tx.
     *    * The last output must be a BitAsset control coin
     *    * If the initial supply is nonzero,
     *      the second-to-last output must be a BitAsset output
     *    * Otherwise,
     *      * The number of BitAsset control coin outputs is exactly the number
     *        of BitAsset control coin inputs
     *      * The number of BitAsset outputs is at least
     *        the number of unique BitAssets in the inputs.
     *  * If the tx is a BitAsset update, then there must be at least one
     *    BitAsset control coin input and output.
     *  * If the tx is an AMM Burn, then
     *    * There must be at least two unique BitAsset outputs
     *    * The number of unique BitAsset outputs must be at most two more than
     *      the number of unique BitAsset inputs
     *    * The number of unique BitAsset inputs must be at most equal to the
     *      number of unique BitAsset outputs
     *  * If the tx is an AMM Mint, then
     *    * There must be at least two BitAsset inputs
     *    * The number of unique BitAsset outputs must be at most equal to the
     *      number of unique BitAsset inputs
     *    * The number of unique BitAsset inputs must be at most two more than
     *      the number of unique BitAsset outputs.
     *  * If the tx is an AMM Swap, then
     *    * There must be at least one BitAsset input
     *    * The number of unique BitAsset outputs must be one less than,
     *      one greater than, or equal to, the number of unique BitAsset inputs.
     *  * If the tx is a Dutch auction create, then
     *    * There must be at least one unique BitAsset input
     *    * The number of unique BitAsset outputs must be at most equal to the
     *      number of unique BitAsset inputs
     *    * The number of unique BitAsset inputs must be at most one more than
     *      the number of unique BitAsset outputs.
     *  * If the tx is a Dutch auction bid, then
     *    * There must be at least one BitAsset input
     *    * The number of unique BitAsset outputs must be one less than,
     *      one greater than, or equal to, the number of unique BitAsset inputs.
     *  * If the tx is a Dutch auction collect, then
     *    * There must be at least one unique BitAsset output
     *    * The number of unique BitAsset outputs must be at most two more than
     *      the number of unique BitAsset inputs
     *    * The number of unique BitAsset inputs must be at most equal to the
     *      number of unique BitAsset outputs
     * */
    pub fn validate_bitassets(
        &self,
        rotxn: &RoTxn,
        tx: &FilledTransaction,
    ) -> Result<(), Error> {
        // number of unique bitassets in the inputs
        let n_unique_bitasset_inputs: usize = tx
            .spent_bitassets()
            .filter_map(|(_, output)| output.bitasset())
            .unique()
            .count();
        let n_bitasset_control_inputs: usize =
            tx.spent_bitasset_controls().count();
        let n_bitasset_outputs: usize = tx.bitasset_outputs().count();
        let n_unique_bitasset_outputs: usize =
            tx.unique_spent_bitassets().len();
        let n_bitasset_control_outputs: usize =
            tx.bitasset_control_outputs().count();
        if tx.is_update()
            && (n_bitasset_control_inputs < 1 || n_bitasset_control_outputs < 1)
        {
            return Err(error::BitAsset::NoBitAssetsToUpdate.into());
        };
        if tx.is_amm_burn()
            && (n_unique_bitasset_outputs < 2
                || n_unique_bitasset_inputs > n_unique_bitasset_outputs
                || n_unique_bitasset_outputs > n_unique_bitasset_inputs + 2)
        {
            return Err(error::Amm::InvalidBurn.into());
        };
        if tx.is_amm_mint()
            && (n_unique_bitasset_inputs < 2
                || n_unique_bitasset_outputs > n_unique_bitasset_inputs
                || n_unique_bitasset_inputs > n_unique_bitasset_outputs + 2)
        {
            return Err(error::Amm::TooFewBitAssetsToMint.into());
        };
        if (tx.is_amm_swap() || tx.is_dutch_auction_bid())
            && (n_unique_bitasset_inputs < 1
                || !{
                    let min_unique_bitasset_outputs =
                        n_unique_bitasset_inputs.saturating_sub(1);
                    let max_unique_bitasset_outputs =
                        n_unique_bitasset_inputs + 1;
                    (min_unique_bitasset_outputs..=max_unique_bitasset_outputs)
                        .contains(&n_unique_bitasset_outputs)
                })
        {
            let err = error::dutch_auction::Bid::Invalid;
            return Err(Error::DutchAuction(err.into()));
        };
        if tx.is_dutch_auction_create()
            && (n_unique_bitasset_inputs < 1
                || n_unique_bitasset_outputs > n_unique_bitasset_inputs
                || n_unique_bitasset_inputs > n_unique_bitasset_outputs + 1)
        {
            return Err(error::DutchAuction::TooFewBitAssetsToCreate.into());
        };
        if tx.is_dutch_auction_collect()
            && (n_unique_bitasset_outputs < 1
                || n_unique_bitasset_inputs > n_unique_bitasset_outputs
                || n_unique_bitasset_outputs > n_unique_bitasset_inputs + 2)
        {
            let err = error::dutch_auction::Collect::Invalid;
            return Err(Error::DutchAuction(err.into()));
        };
        if let Some(TxData::BitAssetRegistration {
            name_hash,
            initial_supply,
            ..
        }) = tx.data()
        {
            if n_bitasset_control_outputs != n_bitasset_control_inputs + 1 {
                return Err(Error::UnbalancedBitAssetControls {
                    n_bitasset_control_inputs,
                    n_bitasset_control_outputs,
                });
            };
            if !tx
                .outputs()
                .last()
                .is_some_and(|last_output| last_output.is_bitasset_control())
            {
                return Err(Error::LastOutputNotControlCoin);
            }
            if *initial_supply == 0 {
                if n_bitasset_outputs < n_unique_bitasset_inputs {
                    return Err(Error::UnbalancedBitAssets {
                        n_unique_bitasset_inputs,
                        n_bitasset_outputs,
                    });
                }
            } else {
                if n_bitasset_outputs < n_unique_bitasset_inputs + 1 {
                    return Err(Error::UnbalancedBitAssets {
                        n_unique_bitasset_inputs,
                        n_bitasset_outputs,
                    });
                }
                let outputs = tx.outputs();
                let second_to_last_output = outputs.get(outputs.len() - 2);
                if !second_to_last_output
                    .is_some_and(|s2l_output| s2l_output.is_bitasset())
                {
                    return Err(Error::SecondLastOutputNotBitAsset);
                }
            }
            if self
                .bitassets
                .try_get_bitasset(rotxn, &BitAssetId(*name_hash))?
                .is_some()
            {
                return Err(Error::BitAssetAlreadyRegistered {
                    name_hash: *name_hash,
                });
            };
            Ok(())
        } else {
            if n_bitasset_control_outputs != n_bitasset_control_inputs {
                return Err(Error::UnbalancedBitAssetControls {
                    n_bitasset_control_inputs,
                    n_bitasset_control_outputs,
                });
            };
            if n_bitasset_outputs < n_unique_bitasset_inputs {
                return Err(Error::UnbalancedBitAssets {
                    n_unique_bitasset_inputs,
                    n_bitasset_outputs,
                });
            }
            if n_unique_bitasset_inputs == 0 && n_bitasset_outputs != 0 {
                return Err(Error::UnbalancedBitAssets {
                    n_unique_bitasset_inputs,
                    n_bitasset_outputs,
                });
            }
            Ok(())
        }
    }

    /// Validates a filled transaction, and returns the fee
    pub fn validate_filled_transaction(
        &self,
        rotxn: &RoTxn,
        tx: &FilledTransaction,
    ) -> Result<bitcoin::Amount, Error> {
        let () = self.validate_reservations(tx)?;
        let () = self.validate_bitassets(rotxn, tx)?;
        
        // Validate swap transactions
        if let Some(TxData::SwapCreate { swap_id, parent_chain, l1_txid_bytes, l2_amount, l2_recipient, l1_recipient_address, .. }) = &tx.transaction.data {
            // Verify swap doesn't already exist
            let swap_id = SwapId(*swap_id);
            if self.get_swap(rotxn, &swap_id)?.is_some() {
                return Err(Error::InvalidTransaction(
                    format!("Swap already exists: {:?}", swap_id)
                ));
            }
            
            // Verify amount is positive
            if *l2_amount == 0 {
                return Err(Error::InvalidTransaction(
                    "Swap amount must be positive".to_string()
                ));
            }
            
            // COIN LOCKING VALIDATION FOR L2 → L1 SWAPS
            // If l1_recipient_address is set, this is an L2 → L1 swap
            // Alice's coins must be locked when creating the swap
            if l1_recipient_address.is_some() {
                // Check that no inputs are already locked to another swap
                for input in &tx.transaction.inputs {
                    if let Some(locked_swap_id) = self.is_output_locked_to_swap(rotxn, input)? {
                        return Err(Error::InvalidTransaction(format!(
                            "Cannot spend locked output: {:?} is locked to swap {:?}",
                            input, locked_swap_id
                        )));
                    }
                }
                
                // Verify transaction spends at least l2_amount worth of Bitcoin
                let spent_value = tx.spent_bitcoin_value()
                    .map_err(|e| Error::InvalidTransaction(format!("Failed to calculate spent value: {:?}", e)))?;
                let required_amount = bitcoin::Amount::from_sat(*l2_amount);
                
                if spent_value < required_amount {
                    return Err(Error::InvalidTransaction(format!(
                        "SwapCreate must spend at least {} sats, but only spent {} sats",
                        required_amount.to_sat(), spent_value.to_sat()
                    )));
                }
            }
            
            // Verify swap ID is correctly computed
            let l1_txid = if l1_txid_bytes.len() == 32 {
                let mut hash32 = [0u8; 32];
                hash32.copy_from_slice(l1_txid_bytes);
                TxId::Hash32(hash32)
            } else {
                TxId::Hash(l1_txid_bytes.clone())
            };
            
            let swap = Swap::new(
                parent_chain.clone(),
                l1_txid,
                None, // Will use default confirmations
                *l2_recipient,
                bitcoin::Amount::from_sat(*l2_amount),
                0, // Height doesn't matter for validation
            );
            
            if swap.id.0 != *swap_id {
                return Err(Error::InvalidTransaction(format!(
                    "Swap ID mismatch: expected {:?}, got {:?}",
                    swap.id.0, swap_id
                )));
            }
        } else if let Some(TxData::SwapClaim { swap_id, .. }) = &tx.transaction.data {
            let swap_id = SwapId(*swap_id);
            
            // Verify swap exists
            let swap = self.get_swap(rotxn, &swap_id)?
                .ok_or_else(|| Error::InvalidTransaction(
                    format!("Swap not found: {:?}", swap_id)
                ))?;
            
            // Verify swap is ready to claim
            if !matches!(swap.state, crate::parent_chain::swap::SwapState::ReadyToClaim) {
                return Err(Error::InvalidTransaction(format!(
                    "Swap not ready to claim: {:?}",
                    swap.state
                )));
            }
            
            // COIN UNLOCKING VALIDATION FOR SWAP CLAIMS
            // Verify that at least one input is locked to this swap
            let mut has_locked_input = false;
            for input in &tx.transaction.inputs {
                if let Some(locked_swap_id) = self.is_output_locked_to_swap(rotxn, input)? {
                    if locked_swap_id == swap_id {
                        has_locked_input = true;
                    } else {
                        return Err(Error::InvalidTransaction(format!(
                            "Input {:?} is locked to different swap {:?}, expected {:?}",
                            input, locked_swap_id, swap_id
                        )));
                    }
                }
            }
            
            if !has_locked_input {
                return Err(Error::InvalidTransaction(
                    "SwapClaim must spend at least one output locked to this swap".to_string()
                ));
            }
            
            // Verify at least one output goes to the swap recipient
            let has_recipient_output = tx.transaction.outputs.iter()
                .any(|output| output.address == swap.l2_recipient);
            
            if !has_recipient_output {
                return Err(Error::InvalidTransaction(
                    "Swap claim transaction must have output to swap recipient".to_string()
                ));
            }
        }
        
        // Prevent locked outputs from being spent by non-swap transactions
        if !matches!(tx.transaction.data, Some(TxData::SwapClaim { .. })) {
            for input in &tx.transaction.inputs {
                if let Some(locked_swap_id) = self.is_output_locked_to_swap(rotxn, input)? {
                    return Err(Error::InvalidTransaction(format!(
                        "Cannot spend locked output: {:?} is locked to swap {:?}. Use SwapClaim to unlock.",
                        input, locked_swap_id
                    )));
                }
            }
        }
        
        tx.bitcoin_fee()?.ok_or(Error::NotEnoughValueIn)
    }

    pub fn validate_transaction(
        &self,
        rotxn: &RoTxn,
        transaction: &AuthorizedTransaction,
    ) -> Result<bitcoin::Amount, Error> {
        let filled_transaction =
            self.fill_transaction(rotxn, &transaction.transaction)?;
        for (authorization, spent_utxo) in transaction
            .authorizations
            .iter()
            .zip(filled_transaction.spent_utxos.iter())
        {
            if authorization.get_address() != spent_utxo.address {
                return Err(Error::WrongPubKeyForAddress);
            }
        }
        if Authorization::verify_transaction(transaction).is_err() {
            return Err(Error::AuthorizationError);
        }
        let fee =
            self.validate_filled_transaction(rotxn, &filled_transaction)?;
        Ok(fee)
    }

    pub fn get_last_deposit_block_hash(
        &self,
        rotxn: &RoTxn,
    ) -> Result<Option<bitcoin::BlockHash>, Error> {
        let block_hash = self
            .deposit_blocks
            .last(rotxn)?
            .map(|(_, (block_hash, _))| block_hash);
        Ok(block_hash)
    }

    pub fn get_last_withdrawal_bundle_event_block_hash(
        &self,
        rotxn: &RoTxn,
    ) -> Result<Option<bitcoin::BlockHash>, Error> {
        let block_hash = self
            .withdrawal_bundle_event_blocks
            .last(rotxn)?
            .map(|(_, (block_hash, _))| block_hash);
        Ok(block_hash)
    }

    /// Get total sidechain wealth in Bitcoin
    pub fn sidechain_wealth(
        &self,
        rotxn: &RoTxn,
    ) -> Result<bitcoin::Amount, Error> {
        let mut total_deposit_utxo_value = bitcoin::Amount::ZERO;
        self.utxos.iter(rotxn)?.map_err(Error::from).for_each(
            |(outpoint_key, output)| {
                let outpoint = outpoint_key.to_outpoint();
                if let OutPoint::Deposit(_) = outpoint {
                    total_deposit_utxo_value = total_deposit_utxo_value
                        .checked_add(output.get_bitcoin_value())
                        .ok_or(AmountOverflowError)?;
                }
                Ok::<_, Error>(())
            },
        )?;
        let mut total_deposit_stxo_value = bitcoin::Amount::ZERO;
        let mut total_withdrawal_stxo_value = bitcoin::Amount::ZERO;
        self.stxos.iter(rotxn)?.map_err(Error::from).for_each(
            |(outpoint_key, spent_output)| {
                let outpoint = outpoint_key.to_outpoint();
                if let OutPoint::Deposit(_) = outpoint {
                    total_deposit_stxo_value = total_deposit_stxo_value
                        .checked_add(spent_output.output.get_bitcoin_value())
                        .ok_or(AmountOverflowError)?;
                }
                if let InPoint::Withdrawal { .. } = spent_output.inpoint {
                    total_withdrawal_stxo_value = total_deposit_stxo_value
                        .checked_add(spent_output.output.get_bitcoin_value())
                        .ok_or(AmountOverflowError)?;
                }
                Ok::<_, Error>(())
            },
        )?;
        let total_wealth: bitcoin::Amount = total_deposit_utxo_value
            .checked_add(total_deposit_stxo_value)
            .ok_or(AmountOverflowError)?
            .checked_sub(total_withdrawal_stxo_value)
            .ok_or(AmountOverflowError)?;
        Ok(total_wealth)
    }

    pub fn validate_block(
        &self,
        rotxn: &RoTxn,
        header: &Header,
        body: &Body,
    ) -> Result<bitcoin::Amount, Error> {
        block::validate(self, rotxn, header, body)
    }

    pub fn connect_block(
        &self,
        rwtxn: &mut RwTxn,
        header: &Header,
        body: &Body,
    ) -> Result<(), Error> {
        block::connect(self, rwtxn, header, body)
    }

    pub fn disconnect_tip(
        &self,
        rwtxn: &mut RwTxn,
        header: &Header,
        body: &Body,
    ) -> Result<(), Error> {
        block::disconnect_tip(self, rwtxn, header, body)
    }

    pub fn connect_two_way_peg_data(
        &self,
        rwtxn: &mut RwTxn,
        two_way_peg_data: &TwoWayPegData,
    ) -> Result<(), Error> {
        two_way_peg_data::connect(self, rwtxn, two_way_peg_data)
    }

    pub fn disconnect_two_way_peg_data(
        &self,
        rwtxn: &mut RwTxn,
        two_way_peg_data: &TwoWayPegData,
    ) -> Result<(), Error> {
        two_way_peg_data::disconnect(self, rwtxn, two_way_peg_data)
    }

    pub fn prevalidate_block(
        &self,
        rotxn: &RoTxn,
        header: &Header,
        body: &Body,
    ) -> Result<PrevalidatedBlock, Error> {
        block::prevalidate(self, rotxn, header, body)
    }

    pub fn connect_prevalidated_block(
        &self,
        rwtxn: &mut RwTxn,
        header: &Header,
        body: &Body,
        prevalidated: PrevalidatedBlock,
    ) -> Result<(), Error> {
        block::connect_prevalidated(self, rwtxn, header, body, prevalidated)
    }

    pub fn apply_block(
        &self,
        rwtxn: &mut RwTxn,
        header: &Header,
        body: &Body,
    ) -> Result<(), Error> {
        let prevalidated = self.prevalidate_block(rwtxn, header, body)?;
        self.connect_prevalidated_block(rwtxn, header, body, prevalidated)?;
        Ok(())
    }
}

impl Watchable<()> for State {
    type WatchStream = impl Stream<Item = ()>;

    /// Get a signal that notifies whenever the tip changes
    fn watch(&self) -> Self::WatchStream {
        tokio_stream::wrappers::WatchStream::new(self.tip.watch().clone())
    }
}

#[cfg(test)]
mod swap_tests {
    use super::*;
    use crate::parent_chain::{swap::Swap, SwapId, client::TxId, config::ParentChainType};
    use crate::types::Address;
    use tempfile::TempDir;

    fn create_test_state() -> (State, sneed::Env, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let env_path = temp_dir.path().join("test.mdb");
        std::fs::create_dir_all(&env_path).unwrap();
        
        let mut env_open_opts = heed::EnvOpenOptions::new();
        env_open_opts.map_size(1024 * 1024); // 1MB
        env_open_opts.max_dbs(State::NUM_DBS);
        let env = unsafe { heed::Env::open(&env_open_opts, &env_path).unwrap() };
        let state = State::new(&env).unwrap();
        (state, env, temp_dir)
    }

    fn create_test_swap() -> Swap {
        Swap::new(
            ParentChainType::Btc,
            TxId::Hash32([1u8; 32]),
            Some(3),
            Address([2u8; 32]),
            bitcoin::Amount::from_sat(100_000),
            100,
        )
    }

    #[test]
    fn test_save_and_load_swap() {
        let (state, env, _temp_dir) = create_test_state();
        let swap = create_test_swap();

        // Save swap
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.save_swap(&mut rwtxn, &swap).unwrap();
            rwtxn.commit().unwrap();
        }

        // Load swap
        let rotxn = env.read_txn().unwrap();
        let loaded_swap = state.get_swap(&rotxn, &swap.id).unwrap();
        assert!(loaded_swap.is_some());
        let loaded_swap = loaded_swap.unwrap();
        assert_eq!(loaded_swap.id, swap.id);
        assert_eq!(loaded_swap.l2_amount, swap.l2_amount);
        assert_eq!(loaded_swap.l2_recipient, swap.l2_recipient);
    }

    #[test]
    fn test_get_swap_by_l1_txid() {
        let (state, env, _temp_dir) = create_test_state();
        let swap = create_test_swap();

        // Save swap
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.save_swap(&mut rwtxn, &swap).unwrap();
            rwtxn.commit().unwrap();
        }

        // Get by L1 txid
        let rotxn = env.read_txn().unwrap();
        let found_swap = state.get_swap_by_l1_txid(
            &rotxn,
            &swap.parent_chain,
            &swap.l1_txid,
        ).unwrap();
        assert!(found_swap.is_some());
        assert_eq!(found_swap.unwrap().id, swap.id);
    }

    #[test]
    fn test_get_swaps_by_recipient() {
        let (state, env, _temp_dir) = create_test_state();
        let recipient = Address([2u8; 32]);
        
        let swap1 = Swap::new(
            ParentChainType::Btc,
            TxId::Hash32([1u8; 32]),
            Some(3),
            recipient,
            bitcoin::Amount::from_sat(100_000),
            100,
        );

        let swap2 = Swap::new(
            ParentChainType::Btc,
            TxId::Hash32([3u8; 32]),
            Some(3),
            recipient,
            bitcoin::Amount::from_sat(200_000),
            101,
        );

        // Save both swaps
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.save_swap(&mut rwtxn, &swap1).unwrap();
            state.save_swap(&mut rwtxn, &swap2).unwrap();
            rwtxn.commit().unwrap();
        }

        // Get by recipient
        let rotxn = env.read_txn().unwrap();
        let swaps = state.get_swaps_by_recipient(&rotxn, &recipient).unwrap();
        assert_eq!(swaps.len(), 2);
        assert!(swaps.iter().any(|s| s.id == swap1.id));
        assert!(swaps.iter().any(|s| s.id == swap2.id));
    }

    #[test]
    fn test_delete_swap() {
        let (state, env, _temp_dir) = create_test_state();
        let swap = create_test_swap();

        // Save swap
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.save_swap(&mut rwtxn, &swap).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify it exists
        {
            let rotxn = env.read_txn().unwrap();
            assert!(state.get_swap(&rotxn, &swap.id).unwrap().is_some());
        }

        // Delete swap
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.delete_swap(&mut rwtxn, &swap.id).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify it's gone
        {
            let rotxn = env.read_txn().unwrap();
            assert!(state.get_swap(&rotxn, &swap.id).unwrap().is_none());
            assert!(state.get_swap_by_l1_txid(
                &rotxn,
                &swap.parent_chain,
                &swap.l1_txid,
            ).unwrap().is_none());
        }
    }

    #[test]
    fn test_load_all_swaps() {
        let (state, env, _temp_dir) = create_test_state();
        
        let swap1 = Swap::new(
            ParentChainType::Btc,
            TxId::Hash32([1u8; 32]),
            Some(3),
            Address([2u8; 32]),
            bitcoin::Amount::from_sat(100_000),
            100,
        );

        let swap2 = Swap::new(
            ParentChainType::Bch,
            TxId::Hash32([3u8; 32]),
            Some(3),
            Address([4u8; 32]),
            bitcoin::Amount::from_sat(200_000),
            101,
        );

        // Save both swaps
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.save_swap(&mut rwtxn, &swap1).unwrap();
            state.save_swap(&mut rwtxn, &swap2).unwrap();
            rwtxn.commit().unwrap();
        }

        // Load all
        let rotxn = env.read_txn().unwrap();
        let swaps = state.load_all_swaps(&rotxn).unwrap();
        assert_eq!(swaps.len(), 2);
        assert!(swaps.iter().any(|s| s.id == swap1.id));
        assert!(swaps.iter().any(|s| s.id == swap2.id));
    }

    #[test]
    fn test_lock_output_to_swap() {
        let (state, env, _temp_dir) = create_test_state();
        let swap = create_test_swap();
        let swap_id = swap.id;
        let outpoint = crate::types::OutPoint::Regular {
            txid: crate::types::Txid::from([1u8; 32]),
            vout: 0,
        };

        // Lock output
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.lock_output_to_swap(&mut rwtxn, &outpoint, &swap_id).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify it's locked
        {
            let rotxn = env.read_txn().unwrap();
            let locked_swap_id = state.is_output_locked_to_swap(&rotxn, &outpoint).unwrap();
            assert!(locked_swap_id.is_some());
            assert_eq!(locked_swap_id.unwrap(), swap_id);
        }
    }

    #[test]
    fn test_unlock_output_from_swap() {
        let (state, env, _temp_dir) = create_test_state();
        let swap = create_test_swap();
        let swap_id = swap.id;
        let outpoint = crate::types::OutPoint::Regular {
            txid: crate::types::Txid::from([1u8; 32]),
            vout: 0,
        };

        // Lock output
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.lock_output_to_swap(&mut rwtxn, &outpoint, &swap_id).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify it's locked
        {
            let rotxn = env.read_txn().unwrap();
            assert!(state.is_output_locked_to_swap(&rotxn, &outpoint).unwrap().is_some());
        }

        // Unlock output
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.unlock_output_from_swap(&mut rwtxn, &outpoint).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify it's unlocked
        {
            let rotxn = env.read_txn().unwrap();
            assert!(state.is_output_locked_to_swap(&rotxn, &outpoint).unwrap().is_none());
        }
    }

    #[test]
    fn test_multiple_outputs_locked_to_same_swap() {
        let (state, env, _temp_dir) = create_test_state();
        let swap = create_test_swap();
        let swap_id = swap.id;
        
        let outpoint1 = crate::types::OutPoint::Regular {
            txid: crate::types::Txid::from([1u8; 32]),
            vout: 0,
        };
        let outpoint2 = crate::types::OutPoint::Regular {
            txid: crate::types::Txid::from([1u8; 32]),
            vout: 1,
        };

        // Lock both outputs
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.lock_output_to_swap(&mut rwtxn, &outpoint1, &swap_id).unwrap();
            state.lock_output_to_swap(&mut rwtxn, &outpoint2, &swap_id).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify both are locked
        {
            let rotxn = env.read_txn().unwrap();
            assert_eq!(state.is_output_locked_to_swap(&rotxn, &outpoint1).unwrap().unwrap(), swap_id);
            assert_eq!(state.is_output_locked_to_swap(&rotxn, &outpoint2).unwrap().unwrap(), swap_id);
        }
    }

    #[test]
    fn test_different_swaps_lock_different_outputs() {
        let (state, env, _temp_dir) = create_test_state();
        let swap1 = Swap::new(
            ParentChainType::Btc,
            TxId::Hash32([1u8; 32]),
            Some(3),
            Address([2u8; 32]),
            bitcoin::Amount::from_sat(100_000),
            100,
        );
        let swap2 = Swap::new(
            ParentChainType::Bch,
            TxId::Hash32([3u8; 32]),
            Some(3),
            Address([4u8; 32]),
            bitcoin::Amount::from_sat(200_000),
            101,
        );
        
        let outpoint1 = crate::types::OutPoint::Regular {
            txid: crate::types::Txid::from([1u8; 32]),
            vout: 0,
        };
        let outpoint2 = crate::types::OutPoint::Regular {
            txid: crate::types::Txid::from([2u8; 32]),
            vout: 0,
        };

        // Lock outputs to different swaps
        {
            let mut rwtxn = env.write_txn().unwrap();
            state.lock_output_to_swap(&mut rwtxn, &outpoint1, &swap1.id).unwrap();
            state.lock_output_to_swap(&mut rwtxn, &outpoint2, &swap2.id).unwrap();
            rwtxn.commit().unwrap();
        }

        // Verify each is locked to the correct swap
        {
            let rotxn = env.read_txn().unwrap();
            assert_eq!(state.is_output_locked_to_swap(&rotxn, &outpoint1).unwrap().unwrap(), swap1.id);
            assert_eq!(state.is_output_locked_to_swap(&rotxn, &outpoint2).unwrap().unwrap(), swap2.id);
        }
    }
}
