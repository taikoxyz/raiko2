//! Provides the [`WitnessDatabase`] type, an implementation of [`reth_revm::Database`]
//! specifically designed for stateless execution environments.

use crate::trie::StatelessTrie;
use alloy_primitives::{Address, B256, Bytes, StorageKey, StorageValue, U256, map::B256Map};
use reth_errors::{ProviderError, ProviderResult};
use reth_primitives_traits::{Account, Bytecode as RethBytecode};
use reth_revm::{Database, bytecode::Bytecode, state::AccountInfo};
use reth_storage_api::{
    AccountReader, BlockHashReader, BytecodeReader, HashedPostStateProvider, StateProofProvider,
    StateProvider, StateRootProvider, StorageRootProvider,
};
use reth_trie_common::{
    AccountProof, ExecutionWitnessMode, HashedPostState, HashedStorage, KeccakKeyHasher,
    MultiProof, MultiProofTargets, StorageMultiProof, StorageProof, TrieInput,
    updates::TrieUpdates,
};
use std::format;
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, Default)]
pub(super) struct AncestorHashes {
    start_block_number: u64,
    hashes: Vec<B256>,
}

impl AncestorHashes {
    pub(super) const fn new(start_block_number: u64, hashes: Vec<B256>) -> Self {
        Self {
            start_block_number,
            hashes,
        }
    }

    fn get(&self, block_number: u64) -> Option<B256> {
        let index = usize::try_from(block_number.checked_sub(self.start_block_number)?).ok()?;
        self.hashes.get(index).copied()
    }
}

/// An EVM database implementation backed by witness data.
///
/// This struct implements the [`reth_revm::Database`] trait, allowing the EVM to execute
/// transactions using:
///  - Account and storage slot data provided by a [`StatelessTrie`] implementation.
///  - Bytecode and ancestor block hashes provided by in-memory maps.
///
/// This is designed for stateless execution scenarios where direct access to a full node's
/// database is not available or desired.
#[derive(Debug)]
#[allow(clippy::redundant_pub_crate)]
pub(super) struct WitnessDatabase<'a, T>
where
    T: StatelessTrie,
{
    /// Contiguous window of ancestor block hashes used to service the `BLOCKHASH` opcode.
    block_hashes: AncestorHashes,
    /// Map of code hashes to bytecode.
    /// Used to fetch contract code needed during execution.
    bytecode: B256Map<Bytecode>,
    /// The sparse Merkle Patricia Trie containing account and storage state.
    /// This is used to provide account/storage values during EVM execution.
    /// TODO: Ideally we do not have this trie and instead a simple map.
    /// TODO: Then as a corollary we can avoid unnecessary hashing in `Database::storage`
    /// TODO: and `Database::basic` without needing to cache the hashed Addresses and Keys
    trie: &'a T,
}

impl<'a, T> WitnessDatabase<'a, T>
where
    T: StatelessTrie,
{
    /// Creates a new [`WitnessDatabase`] instance.
    ///
    /// # Assumptions
    ///
    /// This function assumes:
    /// 1. The provided `trie` has been populated with state data consistent with a known state root
    ///    (e.g., using witness data and verifying against a parent block's state root).
    /// 2. The `bytecode` map contains all bytecode corresponding to code hashes present in the
    ///    account data within the `trie`.
    /// 3. The `ancestor_hashes` map contains the block hashes for the relevant ancestor blocks (up
    ///    to 256 including the current block number). It assumes these hashes correspond to a
    ///    contiguous chain of blocks. The caller is responsible for verifying the contiguity and
    ///    the block limit.
    pub(super) const fn new(
        trie: &'a T,
        bytecode: B256Map<Bytecode>,
        ancestor_hashes: AncestorHashes,
    ) -> Self {
        Self {
            trie,
            block_hashes: ancestor_hashes,
            bytecode,
        }
    }
}

impl<T> Database for WitnessDatabase<'_, T>
where
    T: StatelessTrie,
{
    /// The database error type.
    type Error = ProviderError;

    /// Get basic account information by hashing the address and looking up the account RLP
    /// in the underlying [`StatelessTrie`] implementation.
    ///
    /// Returns `Ok(None)` if the account is not found in the trie.
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.trie.account(address).map(|opt| {
            opt.map(|account| AccountInfo {
                balance: account.balance,
                nonce: account.nonce,
                code_hash: account.code_hash,
                account_id: None,
                code: None,
            })
        })
    }

    /// Get storage value of an account at a specific slot.
    ///
    /// Returns `U256::ZERO` if the slot is not found in the trie.
    fn storage(&mut self, address: Address, slot: U256) -> Result<U256, Self::Error> {
        self.trie.storage(address, slot)
    }

    /// Get account code by its hash from the provided bytecode map.
    ///
    /// Returns an error if the bytecode for the given hash is not found in the map.
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.bytecode.get(&code_hash).cloned().ok_or_else(|| {
            ProviderError::TrieWitnessError(format!("bytecode for {code_hash} not found"))
        })
    }

    /// Get block hash by block number from the provided ancestor hashes map.
    ///
    /// Returns an error if the hash for the given block number is not found in the map.
    fn block_hash(&mut self, block_number: u64) -> Result<B256, Self::Error> {
        self.block_hashes
            .get(block_number)
            .ok_or(ProviderError::StateForNumberNotFound(block_number))
    }
}

/// A witness-backed state provider that shares the same sparse trie state used during execution.
#[derive(Debug, Clone)]
#[allow(clippy::redundant_pub_crate)]
pub(super) struct WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    trie: Rc<RefCell<T>>,
    block_hashes: AncestorHashes,
    bytecode: B256Map<Bytecode>,
}

impl<T> WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    pub(super) fn new(
        trie: T,
        bytecode: B256Map<Bytecode>,
        ancestor_hashes: AncestorHashes,
    ) -> Self {
        Self {
            trie: Rc::new(RefCell::new(trie)),
            block_hashes: ancestor_hashes,
            bytecode,
        }
    }
}

impl<T> AccountReader for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn basic_account(&self, address: &Address) -> ProviderResult<Option<Account>> {
        self.trie
            .borrow()
            .account(*address)
            .map(|account| account.map(Into::into))
    }
}

impl<T> BlockHashReader for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn block_hash(&self, number: u64) -> ProviderResult<Option<B256>> {
        Ok(self.block_hashes.get(number))
    }

    fn canonical_hashes_range(&self, start: u64, end: u64) -> ProviderResult<Vec<B256>> {
        Ok((start..end)
            .filter_map(|block_number| self.block_hashes.get(block_number))
            .collect())
    }
}

impl<T> BytecodeReader for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<RethBytecode>> {
        Ok(self.bytecode.get(code_hash).cloned().map(RethBytecode))
    }
}

impl<T> HashedPostStateProvider for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn hashed_post_state(&self, bundle_state: &reth_revm::db::BundleState) -> HashedPostState {
        HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle_state.state())
    }
}

impl<T> StateRootProvider for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn state_root(&self, hashed_state: HashedPostState) -> ProviderResult<B256> {
        self.trie
            .borrow_mut()
            .calculate_state_root(hashed_state)
            .map_err(|err| ProviderError::TrieWitnessError(err.to_string()))
    }

    fn state_root_from_nodes(&self, _input: TrieInput) -> ProviderResult<B256> {
        unimplemented!("state_root_from_nodes is not used by stateless witness reconstruction")
    }

    fn state_root_with_updates(
        &self,
        hashed_state: HashedPostState,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        self.state_root(hashed_state)
            .map(|state_root| (state_root, TrieUpdates::default()))
    }

    fn state_root_from_nodes_with_updates(
        &self,
        _input: TrieInput,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        unimplemented!(
            "state_root_from_nodes_with_updates is not used by stateless witness reconstruction"
        )
    }
}

impl<T> StorageRootProvider for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn storage_root(
        &self,
        _address: Address,
        _hashed_storage: HashedStorage,
    ) -> ProviderResult<B256> {
        unimplemented!("storage_root is not used by stateless witness reconstruction")
    }

    fn storage_proof(
        &self,
        _address: Address,
        _slot: B256,
        _hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageProof> {
        unimplemented!("storage_proof is not used by stateless witness reconstruction")
    }

    fn storage_multiproof(
        &self,
        _address: Address,
        _slots: &[B256],
        _hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageMultiProof> {
        unimplemented!("storage_multiproof is not used by stateless witness reconstruction")
    }
}

impl<T> StateProofProvider for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn proof(
        &self,
        _input: TrieInput,
        _address: Address,
        _slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        unimplemented!("proof generation is not used by stateless witness reconstruction")
    }

    fn multiproof(
        &self,
        _input: TrieInput,
        _targets: MultiProofTargets,
    ) -> ProviderResult<MultiProof> {
        unimplemented!("multiproof generation is not used by stateless witness reconstruction")
    }

    fn witness(
        &self,
        _input: TrieInput,
        _target: HashedPostState,
        _mode: ExecutionWitnessMode,
    ) -> ProviderResult<Vec<Bytes>> {
        unimplemented!("witness generation is not used by stateless witness reconstruction")
    }
}

impl<T> StateProvider for WitnessStateProvider<T>
where
    T: StatelessTrie,
{
    fn storage(
        &self,
        account: Address,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        self.trie
            .borrow()
            .storage(account, U256::from_be_bytes(storage_key.0))
            .map(Some)
    }
}
