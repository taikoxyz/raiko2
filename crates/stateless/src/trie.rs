use alloy_consensus::TrieAccount;
use alloy_primitives::{
    Address, B256, U256,
    map::{AddressMap, B256Map},
};
use raiko2_primitives::{ExecutionWitness, StatelessValidationError, WitnessStateNode};
use reth_errors::ProviderError;
use reth_revm::state::Bytecode;
use reth_trie_common::HashedPostState;

pub trait StatelessTrie: core::fmt::Debug {
    /// Initialize the trie from the execution witness and expected pre-state root.
    ///
    /// # Errors
    ///
    /// Returns an error if the witness cannot be materialized into a valid trie or if the
    /// pre-state root does not match.
    fn new(
        witness: &ExecutionWitness,
        pre_state_root: B256,
    ) -> Result<(Self, B256Map<Bytecode>), StatelessValidationError>
    where
        Self: Sized;

    /// Initialize the trie from a witness plus a proposal-level shared state node pool.
    ///
    /// Implementations may ignore `shared_state_nodes` when the witness already carries inline
    /// state. Proposal-mode callers use this to avoid embedding duplicate state node bytes in each
    /// witness.
    ///
    /// # Errors
    ///
    /// Returns an error if the witness cannot be materialized into a valid trie or if the
    /// pre-state root does not match.
    fn new_with_state_pool(
        witness: &ExecutionWitness,
        shared_state_nodes: &[WitnessStateNode],
        pre_state_root: B256,
    ) -> Result<(Self, B256Map<Bytecode>), StatelessValidationError>
    where
        Self: Sized,
    {
        let _ = shared_state_nodes;
        Self::new(witness, pre_state_root)
    }

    /// Return the account data for an address.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying witness is incomplete or cannot decode account data.
    fn account(&self, address: Address) -> Result<Option<TrieAccount>, ProviderError>;

    /// Return the storage value for an address and slot.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying witness is incomplete or cannot decode storage data.
    fn storage(&self, address: Address, slot: U256) -> Result<U256, ProviderError>;

    /// Compute the post-state root after applying the hashed post-state changes.
    ///
    /// # Errors
    ///
    /// Returns an error if the trie cannot apply the state changes or recompute the root.
    fn calculate_state_root(
        &mut self,
        state: HashedPostState,
    ) -> Result<B256, StatelessValidationError>;
}

#[allow(clippy::redundant_pub_crate)]
pub(super) trait StatelessTrieExt: StatelessTrie {
    fn append_callers(&mut self, callers: AddressMap<TrieAccount>);
}
