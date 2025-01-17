//! # Ethereum Beacon Client
#![cfg_attr(not(feature = "std"), no_std)]

pub mod config;
mod merkleization;
#[cfg(test)]
mod mock;
pub mod weights;

mod ssz;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_mainnet;
#[cfg(test)]
mod tests_minimal;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub use weights::WeightInfo;

use crate::merkleization::get_sync_committee_bits;
use frame_support::{dispatch::DispatchResult, log, traits::UnixTime, transactional};
use frame_system::ensure_signed;
use snowbridge_beacon_primitives::{
	BeaconHeader, BlockUpdate, Domain, ExecutionHeader, ExecutionHeaderState, FinalizedHeaderState,
	FinalizedHeaderUpdate, ForkData, ForkVersion, InitialSync, PublicKey, Root, SigningData,
	SyncCommittee, SyncCommitteePeriodUpdate,
};
use snowbridge_core::{Message, Verifier};
use sp_core::H256;
use sp_io::hashing::sha2_256;
use sp_std::prelude::*;

pub use pallet::*;

pub type BlockUpdateOf<T> = BlockUpdate<
	<T as Config>::MaxFeeRecipientSize,
	<T as Config>::MaxLogsBloomSize,
	<T as Config>::MaxExtraDataSize,
	<T as Config>::MaxDepositDataSize,
	<T as Config>::MaxPublicKeySize,
	<T as Config>::MaxSignatureSize,
	<T as Config>::MaxProofBranchSize,
	<T as Config>::MaxProposerSlashingSize,
	<T as Config>::MaxAttesterSlashingSize,
	<T as Config>::MaxVoluntaryExitSize,
	<T as Config>::MaxAttestationSize,
	<T as Config>::MaxValidatorsPerCommittee,
	<T as Config>::MaxSyncCommitteeSize,
>;
pub type InitialSyncOf<T> =
	InitialSync<<T as Config>::MaxSyncCommitteeSize, <T as Config>::MaxProofBranchSize>;
pub type SyncCommitteePeriodUpdateOf<T> = SyncCommitteePeriodUpdate<
	<T as Config>::MaxSignatureSize,
	<T as Config>::MaxProofBranchSize,
	<T as Config>::MaxSyncCommitteeSize,
>;
pub type FinalizedHeaderUpdateOf<T> = FinalizedHeaderUpdate<
	<T as Config>::MaxSignatureSize,
	<T as Config>::MaxProofBranchSize,
	<T as Config>::MaxSyncCommitteeSize,
>;
pub type ExecutionHeaderOf<T> =
	ExecutionHeader<<T as Config>::MaxLogsBloomSize, <T as Config>::MaxExtraDataSize>;
pub type SyncCommitteeOf<T> = SyncCommittee<<T as Config>::MaxSyncCommitteeSize>;

#[frame_support::pallet]
pub mod pallet {
	use super::*;

	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;

	use milagro_bls::{AggregatePublicKey, AggregateSignature, AmclError, Signature};
	use snowbridge_beacon_primitives::ForkVersions;
	use snowbridge_core::Proof;
	use snowbridge_ethereum::{Header as EthereumHeader, Log, Receipt};
	use sp_core::{H160, U256};

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		type TimeProvider: UnixTime;
		#[pallet::constant]
		type MaxSyncCommitteeSize: Get<u32>;
		#[pallet::constant]
		type MaxProofBranchSize: Get<u32>;
		#[pallet::constant]
		type MaxExtraDataSize: Get<u32>;
		#[pallet::constant]
		type MaxLogsBloomSize: Get<u32>;
		#[pallet::constant]
		type MaxFeeRecipientSize: Get<u32>;
		#[pallet::constant]
		type MaxDepositDataSize: Get<u32>;
		#[pallet::constant]
		type MaxPublicKeySize: Get<u32>;
		#[pallet::constant]
		type MaxSignatureSize: Get<u32>;
		#[pallet::constant]
		type MaxProposerSlashingSize: Get<u32>;
		#[pallet::constant]
		type MaxAttesterSlashingSize: Get<u32>;
		#[pallet::constant]
		type MaxVoluntaryExitSize: Get<u32>;
		#[pallet::constant]
		type MaxAttestationSize: Get<u32>;
		#[pallet::constant]
		type MaxValidatorsPerCommittee: Get<u32>;
		#[pallet::constant]
		type MaxSlotsPerHistoricalRoot: Get<u64>;
		#[pallet::constant]
		type MaxFinalizedHeaderSlotArray: Get<u32>;
		#[pallet::constant]
		type ForkVersions: Get<ForkVersions>;
		type WeightInfo: WeightInfo;
		type WeakSubjectivityPeriodSeconds: Get<u64>;
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		BeaconHeaderImported { block_hash: H256, slot: u64 },
		ExecutionHeaderImported { block_hash: H256, block_number: u64 },
		SyncCommitteeUpdated { period: u64 },
	}

	#[pallet::error]
	pub enum Error<T> {
		AncientHeader,
		SkippedSyncCommitteePeriod,
		SyncCommitteeMissing,
		Unknown,
		SyncCommitteeParticipantsNotSupermajority,
		InvalidHeaderMerkleProof,
		InvalidSyncCommitteeMerkleProof,
		InvalidAncestryMerkleProof,
		InvalidSignature,
		InvalidSignaturePoint,
		InvalidAggregatePublicKeys,
		InvalidHash,
		InvalidSyncCommitteeBits,
		SignatureVerificationFailed,
		NoBranchExpected,
		HeaderNotFinalized,
		MissingHeader,
		InvalidProof,
		InvalidBlockRootAtSlot,
		DecodeFailed,
		BlockBodyHashTreeRootFailed,
		BlockRootsHashTreeRootFailed,
		HeaderHashTreeRootFailed,
		SyncCommitteeHashTreeRootFailed,
		SigningRootHashTreeRootFailed,
		ForkDataHashTreeRootFailed,
		ExecutionHeaderNotLatest,
		UnexpectedHeaderSlotPosition,
		ExpectedFinalizedHeaderNotStored,
		BridgeBlocked,
		InvalidSyncCommitteeHeaderUpdate,
		InvalidSyncCommitteePeriodUpdateWithGap,
		InvalidSyncCommitteePeriodUpdateWithDuplication,
		InvalidFinalizedHeaderUpdate,
		InvalidFinalizedPeriodUpdate,
		InvalidExecutionHeaderUpdate,
		FinalizedBeaconHeaderSlotsExceeded,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

	#[pallet::storage]
	pub(super) type FinalizedBeaconHeaders<T: Config> =
		StorageMap<_, Identity, H256, BeaconHeader, OptionQuery>;

	#[pallet::storage]
	pub(super) type FinalizedBeaconHeaderSlots<T: Config> =
		StorageValue<_, BoundedVec<u64, T::MaxFinalizedHeaderSlotArray>, ValueQuery>;

	#[pallet::storage]
	pub(super) type FinalizedBeaconHeadersBlockRoot<T: Config> =
		StorageMap<_, Identity, H256, H256, ValueQuery>;

	#[pallet::storage]
	pub(super) type ExecutionHeaders<T: Config> =
		StorageMap<_, Identity, H256, ExecutionHeaderOf<T>, OptionQuery>;

	/// Current sync committee corresponding to the active header.
	/// TODO  prune older sync committees than xxx
	#[pallet::storage]
	pub(super) type SyncCommittees<T: Config> =
		StorageMap<_, Identity, u64, SyncCommitteeOf<T>, ValueQuery>;

	#[pallet::storage]
	pub(super) type ValidatorsRoot<T: Config> = StorageValue<_, H256, ValueQuery>;

	#[pallet::storage]
	pub(super) type LatestFinalizedHeaderState<T: Config> =
		StorageValue<_, FinalizedHeaderState, ValueQuery>;

	#[pallet::storage]
	pub(super) type LatestExecutionHeaderState<T: Config> =
		StorageValue<_, ExecutionHeaderState, ValueQuery>;

	#[pallet::storage]
	pub(super) type LatestSyncCommitteePeriod<T: Config> = StorageValue<_, u64, ValueQuery>;

	#[pallet::storage]
	pub(super) type Blocked<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub initial_sync: Option<InitialSyncOf<T>>,
	}

	#[cfg(feature = "std")]
	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			GenesisConfig { initial_sync: None }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
		fn build(&self) {
			log::info!(
				target: "ethereum-beacon-client",
				"💫 Sync committee size is: {}",
				config::SYNC_COMMITTEE_SIZE
			);

			if let Some(initial_sync) = self.initial_sync.clone() {
				Pallet::<T>::initial_sync(initial_sync).unwrap();
			}
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::sync_committee_period_update())]
		#[transactional]
		pub fn sync_committee_period_update(
			origin: OriginFor<T>,
			sync_committee_period_update: SyncCommitteePeriodUpdateOf<T>,
		) -> DispatchResult {
			let _sender = ensure_signed(origin)?;

			Self::check_bridge_blocked_state()?;

			let sync_committee_period = sync_committee_period_update.sync_committee_period;
			log::info!(
				target: "ethereum-beacon-client",
				"💫 Received sync committee update for period {}. Applying update",
				sync_committee_period
			);

			if let Err(err) =
				Self::process_sync_committee_period_update(sync_committee_period_update)
			{
				log::error!(
					target: "ethereum-beacon-client",
					"💫 Sync committee period update failed with error {:?}",
					err
				);
				return Err(err)
			}

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Sync committee period update for period {} succeeded.",
				sync_committee_period
			);

			Ok(())
		}

		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::import_finalized_header())]
		#[transactional]
		pub fn import_finalized_header(
			origin: OriginFor<T>,
			finalized_header_update: FinalizedHeaderUpdateOf<T>,
		) -> DispatchResult {
			let _sender = ensure_signed(origin)?;

			Self::check_bridge_blocked_state()?;

			let slot = finalized_header_update.finalized_header.slot;

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Received finalized header for slot {}.",
				slot
			);

			if let Err(err) = Self::process_finalized_header(finalized_header_update) {
				log::error!(
					target: "ethereum-beacon-client",
					"💫 Finalized header update failed with error {:?}",
					err
				);
				return Err(err)
			}

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Stored finalized beacon header at slot {}.",
				slot
			);

			Ok(())
		}

		#[pallet::call_index(2)]
		#[pallet::weight(T::WeightInfo::import_execution_header())]
		#[transactional]
		pub fn import_execution_header(
			origin: OriginFor<T>,
			update: BlockUpdateOf<T>,
		) -> DispatchResult {
			let _sender = ensure_signed(origin)?;

			Self::check_bridge_blocked_state()?;

			let slot = update.block.slot;
			let block_hash = update.block.body.execution_payload.block_hash;

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Received header update for slot {}.",
				slot
			);

			if let Err(err) = Self::process_header(update) {
				log::error!(
					target: "ethereum-beacon-client",
					"💫 Header update failed with error {:?}",
					err
				);
				return Err(err)
			}

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Stored execution header {} at beacon slot {}.",
				block_hash,
				slot
			);

			Ok(())
		}

		#[pallet::call_index(3)]
		#[pallet::weight(1000)]
		#[transactional]
		pub fn unblock_bridge(origin: OriginFor<T>) -> DispatchResult {
			let _sender = ensure_root(origin)?;

			<Blocked<T>>::set(false);

			log::info!(target: "ethereum-beacon-client","💫 syncing bridge from governance provided checkpoint.");

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		fn process_initial_sync(initial_sync: InitialSyncOf<T>) -> DispatchResult {
			Self::verify_sync_committee(
				initial_sync.current_sync_committee.clone(),
				initial_sync.current_sync_committee_branch,
				initial_sync.header.state_root,
				config::CURRENT_SYNC_COMMITTEE_DEPTH,
				config::CURRENT_SYNC_COMMITTEE_INDEX,
			)?;

			let period = Self::compute_current_sync_period(initial_sync.header.slot);

			let block_root: H256 =
				merkleization::hash_tree_root_beacon_header(initial_sync.header.clone())
					.map_err(|_| Error::<T>::HeaderHashTreeRootFailed)?
					.into();

			Self::store_sync_committee(period, initial_sync.current_sync_committee);
			Self::store_validators_root(initial_sync.validators_root);

			let slot = initial_sync.header.slot;

			let last_finalized_header = FinalizedHeaderState {
				beacon_block_root: block_root,
				beacon_slot: slot,
				import_time: initial_sync.import_time,
			};

			<FinalizedBeaconHeaders<T>>::insert(block_root, initial_sync.header);
			Self::add_finalized_header_slot(slot)?;
			<LatestFinalizedHeaderState<T>>::set(last_finalized_header);

			Ok(())
		}

		fn process_sync_committee_period_update(
			update: SyncCommitteePeriodUpdateOf<T>,
		) -> DispatchResult {
			ensure!(
				update.signature_slot > update.attested_header.slot &&
					update.attested_header.slot >= update.finalized_header.slot,
				Error::<T>::InvalidSyncCommitteeHeaderUpdate
			);
			let sync_committee_bits =
				get_sync_committee_bits(update.sync_aggregate.sync_committee_bits.clone())
					.map_err(|_| Error::<T>::InvalidSyncCommitteeBits)?;
			Self::sync_committee_participation_is_supermajority(sync_committee_bits.clone())?;
			Self::verify_sync_committee(
				update.next_sync_committee.clone(),
				update.next_sync_committee_branch,
				update.attested_header.state_root,
				config::NEXT_SYNC_COMMITTEE_DEPTH,
				config::NEXT_SYNC_COMMITTEE_INDEX,
			)?;

			let block_root: H256 =
				merkleization::hash_tree_root_beacon_header(update.finalized_header.clone())
					.map_err(|_| Error::<T>::HeaderHashTreeRootFailed)?
					.into();
			Self::verify_header(
				block_root,
				update.finality_branch,
				update.attested_header.state_root,
				config::FINALIZED_ROOT_DEPTH,
				config::FINALIZED_ROOT_INDEX,
			)?;

			let current_period = Self::compute_current_sync_period(update.attested_header.slot);
			let latest_committee_period = <LatestSyncCommitteePeriod<T>>::get();
			ensure!(
				<SyncCommittees<T>>::contains_key(current_period),
				Error::<T>::SyncCommitteeMissing
			);
			let next_period = current_period + 1;
			ensure!(
				!<SyncCommittees<T>>::contains_key(next_period),
				Error::<T>::InvalidSyncCommitteePeriodUpdateWithDuplication
			);
			ensure!(
				(next_period == latest_committee_period + 1),
				Error::<T>::InvalidSyncCommitteePeriodUpdateWithGap
			);

			let current_sync_committee = Self::get_sync_committee_for_period(current_period)?;
			let validators_root = <ValidatorsRoot<T>>::get();

			Self::verify_signed_header(
				sync_committee_bits,
				update.sync_aggregate.sync_committee_signature,
				current_sync_committee.pubkeys,
				update.attested_header,
				validators_root,
				update.signature_slot,
			)?;

			ensure!(
				Self::is_valid_merkle_branch(
					update.block_roots_hash,
					update.block_roots_proof,
					config::BLOCK_ROOTS_INDEX,
					config::BLOCK_ROOTS_DEPTH,
					update.finalized_header.state_root
				),
				Error::<T>::InvalidAncestryMerkleProof
			);

			Self::store_block_root(update.block_roots_hash, block_root);
			Self::store_sync_committee(next_period, update.next_sync_committee);
			Self::store_finalized_header(block_root, update.finalized_header)?;

			Ok(())
		}

		fn process_finalized_header(update: FinalizedHeaderUpdateOf<T>) -> DispatchResult {
			let last_finalized_header = <LatestFinalizedHeaderState<T>>::get();
			ensure!(
				update.signature_slot > update.attested_header.slot &&
					update.attested_header.slot >= update.finalized_header.slot &&
					update.finalized_header.slot > last_finalized_header.beacon_slot,
				Error::<T>::InvalidFinalizedHeaderUpdate
			);
			let import_time = last_finalized_header.import_time;
			let weak_subjectivity_period_check =
				import_time + T::WeakSubjectivityPeriodSeconds::get() as u64;
			let time: u64 = T::TimeProvider::now().as_secs();

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Checking weak subjectivity period. Current time is :{:?} Weak subjectivity period check: {:?}.",
				time,
				weak_subjectivity_period_check
			);

			if time > weak_subjectivity_period_check {
				log::info!(target: "ethereum-beacon-client","💫 Weak subjectivity period exceeded, blocking bridge.",);
				<Blocked<T>>::set(true);
				return Err(Error::<T>::BridgeBlocked.into())
			}

			let sync_committee_bits =
				get_sync_committee_bits(update.sync_aggregate.sync_committee_bits.clone())
					.map_err(|_| Error::<T>::InvalidSyncCommitteeBits)?;
			Self::sync_committee_participation_is_supermajority(sync_committee_bits.clone())?;

			let block_root: H256 =
				merkleization::hash_tree_root_beacon_header(update.finalized_header.clone())
					.map_err(|_| Error::<T>::HeaderHashTreeRootFailed)?
					.into();
			Self::verify_header(
				block_root,
				update.finality_branch,
				update.attested_header.state_root,
				config::FINALIZED_ROOT_DEPTH,
				config::FINALIZED_ROOT_INDEX,
			)?;

			let last_finalized_period =
				Self::compute_current_sync_period(last_finalized_header.beacon_slot);
			let current_period = Self::compute_current_sync_period(update.attested_header.slot);
			ensure!(
				(current_period == last_finalized_period ||
					current_period == last_finalized_period + 1),
				Error::<T>::InvalidFinalizedPeriodUpdate
			);
			let sync_committee = Self::get_sync_committee_for_period(current_period)?;

			let validators_root = <ValidatorsRoot<T>>::get();
			Self::verify_signed_header(
				sync_committee_bits,
				update.sync_aggregate.sync_committee_signature,
				sync_committee.pubkeys,
				update.attested_header,
				validators_root,
				update.signature_slot,
			)?;

			ensure!(
				Self::is_valid_merkle_branch(
					update.block_roots_hash,
					update.block_roots_proof,
					config::BLOCK_ROOTS_INDEX,
					config::BLOCK_ROOTS_DEPTH,
					update.finalized_header.state_root
				),
				Error::<T>::InvalidAncestryMerkleProof
			);

			Self::store_block_root(update.block_roots_hash, block_root);

			Self::store_finalized_header(block_root, update.finalized_header)?;

			Ok(())
		}

		fn store_block_root(block_roots_hash: H256, finalized_header_block_root: H256) {
			<FinalizedBeaconHeadersBlockRoot<T>>::insert(
				finalized_header_block_root,
				block_roots_hash,
			);
		}

		fn process_header(update: BlockUpdateOf<T>) -> DispatchResult {
			let last_finalized_header = <LatestFinalizedHeaderState<T>>::get();
			let latest_finalized_header_slot = last_finalized_header.beacon_slot;
			let block_slot = update.block.slot;
			ensure!(block_slot <= latest_finalized_header_slot, Error::<T>::HeaderNotFinalized);

			let execution_header_state = <LatestExecutionHeaderState<T>>::get();
			let execution_payload = update.block.body.execution_payload.clone();
			ensure!(
				execution_payload.block_number > execution_header_state.block_number,
				Error::<T>::InvalidExecutionHeaderUpdate
			);

			let body_root = merkleization::hash_tree_root_beacon_body(update.block.body.clone())
				.map_err(|_| Error::<T>::BlockBodyHashTreeRootFailed)?;
			let body_root_hash: H256 = body_root.into();

			let header = BeaconHeader {
				slot: update.block.slot,
				proposer_index: update.block.proposer_index,
				parent_root: update.block.parent_root,
				state_root: update.block.state_root,
				body_root: body_root_hash,
			};

			let beacon_block_root: H256 =
				merkleization::hash_tree_root_beacon_header(header.clone())
					.map_err(|_| Error::<T>::HeaderHashTreeRootFailed)?
					.into();

			Self::ancestry_proof(
				update.block_root_proof,
				block_slot,
				beacon_block_root,
				update.block_root_proof_finalized_header,
			)?;

			let current_period = Self::compute_current_sync_period(update.block.slot);
			let sync_committee = Self::get_sync_committee_for_period(current_period)?;

			let validators_root = <ValidatorsRoot<T>>::get();
			let sync_committee_bits =
				get_sync_committee_bits(update.sync_aggregate.sync_committee_bits.clone())
					.map_err(|_| Error::<T>::InvalidSyncCommitteeBits)?;
			Self::verify_signed_header(
				sync_committee_bits,
				update.sync_aggregate.sync_committee_signature,
				sync_committee.pubkeys,
				header,
				validators_root,
				update.signature_slot,
			)?;

			let mut fee_recipient = [0u8; 20];
			let fee_slice = execution_payload.fee_recipient.as_slice();
			if fee_slice.len() == 20 {
				fee_recipient[0..20].copy_from_slice(&(fee_slice));
			} else {
				log::trace!(
					target: "ethereum-beacon-client",
					"💫 fee recipient not 20 characters, len is: {}.",
					fee_slice.len()
				);
			}

			Self::store_execution_header(
				execution_payload.block_hash,
				ExecutionHeader {
					parent_hash: execution_payload.parent_hash,
					fee_recipient: H160::from(fee_recipient),
					state_root: execution_payload.state_root,
					receipts_root: execution_payload.receipts_root,
					logs_bloom: execution_payload.logs_bloom,
					prev_randao: execution_payload.prev_randao,
					block_number: execution_payload.block_number,
					gas_used: execution_payload.gas_used,
					gas_limit: execution_payload.gas_limit,
					timestamp: execution_payload.timestamp,
					extra_data: execution_payload.extra_data,
					base_fee_per_gas: execution_payload.base_fee_per_gas,
					block_hash: execution_payload.block_hash,
					transactions_root: execution_payload.transactions_root,
				},
				block_slot,
				beacon_block_root,
			);

			Ok(())
		}

		fn ancestry_proof(
			block_root_proof: BoundedVec<H256, T::MaxProofBranchSize>,
			block_slot: u64,
			beacon_block_root: H256,
			finalized_header_root: H256,
		) -> DispatchResult {
			// If the block root proof is empty, we know that we expect this header to be a
			// finalized header. We need to check that the header hash matches the finalized header
			// root at the expected slot.
			if block_root_proof.len() == 0 {
				let stored_finalized_header = <FinalizedBeaconHeaders<T>>::get(beacon_block_root);
				if stored_finalized_header.is_none() {
					log::error!(
						target: "ethereum-beacon-client",
						"💫 Finalized block root {} slot {} for ancestry proof (for a finalized header) not found.", beacon_block_root, block_slot
					);
					return Err(Error::<T>::ExpectedFinalizedHeaderNotStored.into())
				}

				let header = stored_finalized_header.unwrap();
				if header.slot != block_slot {
					log::error!(
						target: "ethereum-beacon-client",
						"💫 Finalized block root {} slot {} does not match expected slot {}.", beacon_block_root, block_slot, header.slot
					);
					return Err(Error::<T>::UnexpectedHeaderSlotPosition.into())
				}

				return Ok(())
			}

			let finalized_block_root_hash =
				<FinalizedBeaconHeadersBlockRoot<T>>::get(finalized_header_root);

			if finalized_block_root_hash.is_zero() {
				log::error!(
					target: "ethereum-beacon-client",
					"💫 Finalized block root {} slot {} for ancestry proof not found.", beacon_block_root, block_slot
				);
				return Err(Error::<T>::ExpectedFinalizedHeaderNotStored.into())
			}

			let max_slots_per_historical_root = T::MaxSlotsPerHistoricalRoot::get();
			let index_in_array = block_slot % max_slots_per_historical_root;
			let leaf_index = max_slots_per_historical_root + index_in_array;

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Depth: {} leaf_index: {}", config::BLOCK_ROOT_AT_INDEX_PROOF_DEPTH, leaf_index
			);

			ensure!(
				Self::is_valid_merkle_branch(
					beacon_block_root,
					block_root_proof,
					config::BLOCK_ROOT_AT_INDEX_PROOF_DEPTH,
					leaf_index,
					finalized_block_root_hash
				),
				Error::<T>::InvalidAncestryMerkleProof
			);

			Ok(())
		}

		fn check_bridge_blocked_state() -> DispatchResult {
			if <Blocked<T>>::get() {
				return Err(Error::<T>::BridgeBlocked.into())
			}

			Ok(())
		}

		pub(super) fn verify_signed_header(
			sync_committee_bits: Vec<u8>,
			sync_committee_signature: BoundedVec<u8, T::MaxSignatureSize>,
			sync_committee_pubkeys: BoundedVec<PublicKey, T::MaxSyncCommitteeSize>,
			header: BeaconHeader,
			validators_root: H256,
			signature_slot: u64,
		) -> DispatchResult {
			let mut participant_pubkeys: Vec<PublicKey> = Vec::new();
			// Gathers all the pubkeys of the sync committee members that participated in signing
			// the header.
			for (bit, pubkey) in sync_committee_bits.iter().zip(sync_committee_pubkeys.iter()) {
				if *bit == 1 as u8 {
					let pubk = pubkey.clone();
					participant_pubkeys.push(pubk);
				}
			}

			let fork_version = Self::compute_fork_version(Self::compute_epoch_at_slot(
				signature_slot,
				config::SLOTS_PER_EPOCH,
			));
			let domain_type = config::DOMAIN_SYNC_COMMITTEE.to_vec();
			// Domains are used for for seeds, for signatures, and for selecting aggregators.
			let domain = Self::compute_domain(domain_type, fork_version, validators_root)?;
			// Hash tree root of SigningData - object root + domain
			let signing_root = Self::compute_signing_root(header, domain)?;

			// Verify sync committee aggregate signature.
			Self::bls_fast_aggregate_verify(
				participant_pubkeys,
				signing_root,
				sync_committee_signature,
			)?;

			Ok(())
		}

		pub(super) fn compute_epoch_at_slot(signature_slot: u64, slots_per_epoch: u64) -> u64 {
			return signature_slot / slots_per_epoch
		}

		pub(super) fn bls_fast_aggregate_verify(
			pubkeys: Vec<PublicKey>,
			message: H256,
			signature: BoundedVec<u8, T::MaxSignatureSize>,
		) -> DispatchResult {
			let sig = Signature::from_bytes(&signature[..]);
			if let Err(_e) = sig {
				return Err(Error::<T>::InvalidSignature.into())
			}

			let agg_sig = AggregateSignature::from_signature(&sig.unwrap());

			let public_keys_res: Result<Vec<milagro_bls::PublicKey>, _> = pubkeys
				.iter()
				.map(|bytes| milagro_bls::PublicKey::from_bytes_unchecked(&bytes.0))
				.collect();
			if let Err(e) = public_keys_res {
				match e {
					AmclError::InvalidPoint => return Err(Error::<T>::InvalidSignaturePoint.into()),
					_ => return Err(Error::<T>::InvalidSignature.into()),
				};
			}

			let agg_pub_key_res = AggregatePublicKey::into_aggregate(&public_keys_res.unwrap());
			if let Err(e) = agg_pub_key_res {
				log::error!(target: "ethereum-beacon-client", "💫 invalid public keys: {:?}.", e);
				return Err(Error::<T>::InvalidAggregatePublicKeys.into())
			}

			ensure!(
				agg_sig.fast_aggregate_verify_pre_aggregated(
					&message.as_bytes(),
					&agg_pub_key_res.unwrap()
				),
				Error::<T>::SignatureVerificationFailed
			);

			Ok(())
		}

		pub(super) fn compute_signing_root(
			beacon_header: BeaconHeader,
			domain: Domain,
		) -> Result<Root, DispatchError> {
			let beacon_header_root = merkleization::hash_tree_root_beacon_header(beacon_header)
				.map_err(|_| Error::<T>::HeaderHashTreeRootFailed)?;

			let header_hash_tree_root: H256 = beacon_header_root.into();

			let hash_root = merkleization::hash_tree_root_signing_data(SigningData {
				object_root: header_hash_tree_root,
				domain,
			})
			.map_err(|_| Error::<T>::SigningRootHashTreeRootFailed)?;

			Ok(hash_root.into())
		}

		fn verify_sync_committee(
			sync_committee: SyncCommitteeOf<T>,
			sync_committee_branch: BoundedVec<H256, T::MaxProofBranchSize>,
			header_state_root: H256,
			depth: u64,
			index: u64,
		) -> DispatchResult {
			let sync_committee_root = merkleization::hash_tree_root_sync_committee(sync_committee)
				.map_err(|_| Error::<T>::SyncCommitteeHashTreeRootFailed)?;

			ensure!(
				Self::is_valid_merkle_branch(
					sync_committee_root.into(),
					sync_committee_branch,
					depth,
					index,
					header_state_root
				),
				Error::<T>::InvalidSyncCommitteeMerkleProof
			);

			Ok(())
		}

		fn verify_header(
			block_root: H256,
			proof_branch: BoundedVec<H256, T::MaxProofBranchSize>,
			attested_header_state_root: H256,
			depth: u64,
			index: u64,
		) -> DispatchResult {
			ensure!(
				Self::is_valid_merkle_branch(
					block_root,
					proof_branch,
					depth,
					index,
					attested_header_state_root
				),
				Error::<T>::InvalidHeaderMerkleProof
			);

			Ok(())
		}

		fn store_sync_committee(period: u64, sync_committee: SyncCommitteeOf<T>) {
			<SyncCommittees<T>>::insert(period, sync_committee);

			log::trace!(
				target: "ethereum-beacon-client",
				"💫 Updated latest sync committee period stored to {}.",
				period
			);

			<LatestSyncCommitteePeriod<T>>::set(period);

			Self::deposit_event(Event::SyncCommitteeUpdated { period });
		}

		fn store_finalized_header(block_root: Root, header: BeaconHeader) -> DispatchResult {
			let slot = header.slot;

			<FinalizedBeaconHeaders<T>>::insert(block_root, header);
			Self::add_finalized_header_slot(slot)?;

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Updated latest finalized block root {} at slot {}.",
				block_root,
				slot
			);

			LatestFinalizedHeaderState::<T>::mutate(|s| {
				s.import_time = T::TimeProvider::now().as_secs();
				s.beacon_block_root = block_root;
				s.beacon_slot = slot;
			});

			Self::deposit_event(Event::BeaconHeaderImported { block_hash: block_root, slot });

			Ok(())
		}

		fn add_finalized_header_slot(slot: u64) -> DispatchResult {
			<FinalizedBeaconHeaderSlots<T>>::try_mutate(|b_vec| {
				if b_vec.len() as u32 == T::MaxFinalizedHeaderSlotArray::get() {
					b_vec.remove(0);
				}
				b_vec.try_push(slot)
			})
			.map_err(|_| <Error<T>>::FinalizedBeaconHeaderSlotsExceeded)?;

			Ok(())
		}

		fn store_execution_header(
			block_hash: H256,
			header: ExecutionHeaderOf<T>,
			beacon_slot: u64,
			beacon_block_root: H256,
		) {
			let block_number = header.block_number;

			<ExecutionHeaders<T>>::insert(block_hash, header);

			log::trace!(
				target: "ethereum-beacon-client",
				"💫 Updated latest execution block at {} to number {}.",
				block_hash,
				block_number
			);

			LatestExecutionHeaderState::<T>::mutate(|s| {
				s.beacon_block_root = beacon_block_root;
				s.beacon_slot = beacon_slot;
				s.block_hash = block_hash;
				s.block_number = block_number;
			});

			Self::deposit_event(Event::ExecutionHeaderImported { block_hash, block_number });
		}

		fn store_validators_root(validators_root: H256) {
			<ValidatorsRoot<T>>::set(validators_root);
		}

		/// Sums the bit vector of sync committee particpation.
		///
		/// # Examples
		///
		/// let sync_committee_bits = vec![0, 1, 0, 1, 1, 1];
		/// ensure!(get_sync_committee_sum(sync_committee_bits), 4);
		pub(super) fn get_sync_committee_sum(sync_committee_bits: Vec<u8>) -> u64 {
			sync_committee_bits.iter().fold(0, |acc: u64, x| acc + *x as u64)
		}

		pub(super) fn compute_current_sync_period(slot: u64) -> u64 {
			slot / config::SLOTS_PER_EPOCH / config::EPOCHS_PER_SYNC_COMMITTEE_PERIOD
		}

		/// Return the domain for the domain_type and fork_version.
		pub(super) fn compute_domain(
			domain_type: Vec<u8>,
			fork_version: ForkVersion,
			genesis_validators_root: Root,
		) -> Result<Domain, DispatchError> {
			let fork_data_root =
				Self::compute_fork_data_root(fork_version, genesis_validators_root)?;

			let mut domain = [0u8; 32];
			domain[0..4].copy_from_slice(&(domain_type));
			domain[4..32].copy_from_slice(&(fork_data_root.0[..28]));

			Ok(domain.into())
		}

		fn compute_fork_data_root(
			current_version: ForkVersion,
			genesis_validators_root: Root,
		) -> Result<Root, DispatchError> {
			let hash_root = merkleization::hash_tree_root_fork_data(ForkData {
				current_version,
				genesis_validators_root: genesis_validators_root.into(),
			})
			.map_err(|_| Error::<T>::ForkDataHashTreeRootFailed)?;

			Ok(hash_root.into())
		}

		pub(super) fn is_valid_merkle_branch(
			leaf: H256,
			branch: BoundedVec<H256, T::MaxProofBranchSize>,
			depth: u64,
			index: u64,
			root: Root,
		) -> bool {
			if branch.len() as u64 != depth {
				log::error!(target: "ethereum-beacon-client", "Merkle proof branch length doesn't match depth.");

				return false
			}
			let mut value = leaf;
			if leaf.as_bytes().len() < 32 as usize {
				log::error!(target: "ethereum-beacon-client", "Merkle proof leaf not 32 bytes.");

				return false
			}
			for i in 0..depth {
				if branch[i as usize].as_bytes().len() < 32 as usize {
					log::error!(target: "ethereum-beacon-client", "Merkle proof branch not 32 bytes.");

					return false
				}
				if (index / (2u32.pow(i as u32) as u64) % 2) == 0 {
					// left node
					let mut data = [0u8; 64];
					data[0..32].copy_from_slice(&(value.0));
					data[32..64].copy_from_slice(&(branch[i as usize].0));
					value = sha2_256(&data).into();
				} else {
					let mut data = [0u8; 64]; // right node
					data[0..32].copy_from_slice(&(branch[i as usize].0));
					data[32..64].copy_from_slice(&(value.0));
					value = sha2_256(&data).into();
				}
			}

			return value == root
		}

		pub(super) fn sync_committee_participation_is_supermajority(
			sync_committee_bits: Vec<u8>,
		) -> DispatchResult {
			let sync_committee_sum = Self::get_sync_committee_sum(sync_committee_bits.clone());
			ensure!(
				(sync_committee_sum * 3 >= sync_committee_bits.clone().len() as u64 * 2),
				Error::<T>::SyncCommitteeParticipantsNotSupermajority
			);

			Ok(())
		}

		pub(super) fn get_sync_committee_for_period(
			period: u64,
		) -> Result<SyncCommitteeOf<T>, DispatchError> {
			let sync_committee = <SyncCommittees<T>>::get(period);

			if sync_committee.pubkeys.len() == 0 {
				log::error!(target: "ethereum-beacon-client", "💫 Sync committee for period {} missing", period);
				return Err(Error::<T>::SyncCommitteeMissing.into())
			}

			Ok(sync_committee)
		}

		pub(super) fn compute_fork_version(epoch: u64) -> ForkVersion {
			let fork_versions = T::ForkVersions::get();

			if epoch >= fork_versions.bellatrix.epoch {
				return fork_versions.bellatrix.version
			}
			if epoch >= fork_versions.altair.epoch {
				return fork_versions.altair.version
			}

			return fork_versions.genesis.version
		}

		pub(super) fn initial_sync(initial_sync: InitialSyncOf<T>) -> Result<(), &'static str> {
			log::info!(
				target: "ethereum-beacon-client",
				"💫 Received initial sync, starting processing.",
			);

			if let Err(err) = Self::process_initial_sync(initial_sync) {
				log::error!(
					target: "ethereum-beacon-client",
					"Initial sync failed with error {:?}",
					err
				);
				return Err(<&str>::from(err))
			}

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Initial sync processing succeeded.",
			);

			Ok(())
		}

		// Verifies that the receipt encoded in proof.data is included
		// in the block given by proof.block_hash. Inclusion is only
		// recognized if the block has been finalized.
		fn verify_receipt_inclusion(
			stored_header: ExecutionHeaderOf<T>,
			proof: &Proof,
		) -> Result<Receipt, DispatchError> {
			let result = stored_header
				.check_receipt_proof(&proof.data.1)
				.ok_or(Error::<T>::InvalidProof)?;

			match result {
				Ok(receipt) => Ok(receipt),
				Err(err) => {
					log::trace!(
						target: "ethereum-beacon-client",
						"💫 Failed to decode transaction receipt: {}",
						err
					);
					Err(Error::<T>::InvalidProof.into())
				},
			}
		}
	}

	impl<T: Config> Verifier for Pallet<T> {
		/// Verify a message by verifying the existence of the corresponding
		/// Ethereum log in a block. Returns the log if successful.
		fn verify(message: &Message) -> Result<(Log, u64), DispatchError> {
			log::info!(
				target: "ethereum-beacon-client",
				"💫 Verifying message with block hash {}",
				message.proof.block_hash,
			);

			let stored_header = <ExecutionHeaders<T>>::get(message.proof.block_hash)
				.ok_or(Error::<T>::MissingHeader)?;

			let block_number = stored_header.block_number;

			let receipt = match Self::verify_receipt_inclusion(stored_header, &message.proof) {
				Ok(receipt) => receipt,
				Err(err) => {
					log::error!(
						target: "ethereum-beacon-client",
						"💫 Verify receipt inclusion failed for block {}: {:?}",
						message.proof.block_hash,
						err
					);
					return Err(err)
				},
			};

			log::trace!(
				target: "ethereum-beacon-client",
				"💫 Verified receipt inclusion for transaction at index {} in block {}",
				message.proof.tx_index, message.proof.block_hash,
			);

			let log = match rlp::decode(&message.data) {
				Ok(log) => log,
				Err(err) => {
					log::error!(
						target: "ethereum-beacon-client",
						"💫 RLP log decoded failed {}: {:?}",
						message.proof.block_hash,
						err
					);
					return Err(Error::<T>::DecodeFailed.into())
				},
			};

			if !receipt.contains_log(&log) {
				log::error!(
					target: "ethereum-beacon-client",
					"💫 Event log not found in receipt for transaction at index {} in block {}",
					message.proof.tx_index, message.proof.block_hash,
				);
				return Err(Error::<T>::InvalidProof.into())
			}

			log::info!(
				target: "ethereum-beacon-client",
				"💫 Receipt verification successful for {}",
				message.proof.block_hash,
			);

			Ok((log, block_number))
		}

		// Empty implementation, not necessary for the beacon client,
		// but needs to be declared to implement Verifier interface.
		fn initialize_storage(
			_headers: Vec<EthereumHeader>,
			_initial_difficulty: U256,
			_descendants_until_final: u8,
		) -> Result<(), &'static str> {
			Ok(())
		}
	}
}
