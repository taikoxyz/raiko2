//! Provides the [`WitnessDatabase`] type, an implementation of [`reth_revm::Database`]
//! specifically designed for stateless execution environments.

use crate::trie::StatelessTrie;
use alloy_primitives::{Address, B256, U256, map::B256Map};
use reth_errors::ProviderError;
use reth_revm::{Database, bytecode::Bytecode, state::AccountInfo};
use std::format;

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
    /// The window's contiguity is verified by the caller against the parent chain.
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
    /// 3. The `ancestor_hashes` map contains hashes for a contiguous sequence of completed
    ///    ancestor blocks ending at `current_block - 1`; the current block is never part of the
    ///    set, and revm only ever observes the most recent 256 entries (see
    ///    [`Database::block_hash`] on this type for the exact window contract). The caller is
    ///    responsible for verifying contiguity; no 256-entry limit is enforced here, and any
    ///    older entries are simply never queried.
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

    /// Get block hash by block number from the provided ancestor hash window.
    ///
    /// The revm interpreter only consults the database for `BLOCKHASH` requests strictly within
    /// the 256-block history window of the executing block; current-block and out-of-window
    /// requests are answered with zero by the interpreter itself and never reach this method
    /// (verified against the pinned `revm-interpreter` 35.0.1 `blockhash` instruction).
    ///
    /// A number missing here is therefore always an in-window read the host failed to
    /// provision. Erroring makes revm halt with a fatal external error, so the block is
    /// unprovable rather than provable with a wrong hash: keep this failing closed and never
    /// map a miss to `Ok(B256::ZERO)`.
    fn block_hash(&mut self, block_number: u64) -> Result<B256, Self::Error> {
        self.block_hashes
            .get(block_number)
            .ok_or(ProviderError::StateForNumberNotFound(block_number))
    }
}
