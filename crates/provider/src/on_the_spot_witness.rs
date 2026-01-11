// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use alloy::{
    consensus::{BlockHeader, Transaction},
    eips::BlockNumberOrTag,
    network::{BlockResponse, Network, primitives::HeaderResponse},
    primitives::Bytes,
    providers::Provider,
};
use anyhow::{Context, Result};
use reth_evm::{ConfigureEvm, execute::Executor};
use reth_primitives_traits::{Block, BlockBody, NodePrimitives};
use reth_stateless::ExecutionWitness;
use std::collections::HashSet;
use tracing::{Span, debug};

pub use db::{PreflightDb, ProviderConfig, ProviderDb};
pub use rpc::{DebugApi, StorageRangeQueryResponse, StorageRangeQueryResponseEntry};
pub use trie::{handle_modified_account, handle_new_account, handle_removed_account};

pub mod db {
    use alloy::{
        consensus::BlockHeader,
        eips::eip2930::{AccessList, AccessListItem},
        network::{BlockResponse, Network},
        providers::Provider,
        rlp::decode_exact,
        rpc::types::EIP1186AccountProofResponse,
    };
    use alloy_primitives::{
        Address, B256, BlockNumber, Bytes, KECCAK256_EMPTY, StorageKey, StorageValue, U256,
        keccak256,
        map::{
            AddressHashMap, AddressMap, B256HashMap, B256HashSet, B256Map, HashMap, HashSet,
            hash_map,
        },
    };
    use alloy_trie::{EMPTY_ROOT_HASH, TrieAccount as StateAccount};
    use anyhow::{Context, Result, ensure};
    use itertools::Itertools;
    use revm::{
        Database as RevmDatabase,
        context::DBErrorMarker,
        state::{AccountInfo, Bytecode},
    };
    use risc0_ethereum_trie::{Trie as MerkleTrie, Trie};
    use std::{
        fmt::{self, Debug},
        hash::{BuildHasher, Hash},
    };

    pub(crate) mod provider {
        use alloy::{
            network::{BlockResponse, Network, primitives::HeaderResponse},
            providers::Provider,
            rpc::types::EIP1186AccountProofResponse,
            transports::TransportError,
        };
        use alloy_primitives::{Address, B256, BlockHash, StorageKey, U256, map::B256HashMap};
        use revm::{
            Database as RevmDatabase,
            database::DBErrorMarker,
            primitives::KECCAK_EMPTY,
            state::{AccountInfo, Bytecode},
        };
        use std::{future::IntoFuture, marker::PhantomData};
        use tokio::runtime::Handle;
        use tracing::trace;

        /// Errors returned by the [ProviderDb].
        #[derive(Debug, thiserror::Error)]
        pub enum Error {
            #[error("{0} failed")]
            Rpc(&'static str, #[source] TransportError),
            #[error("block not found")]
            BlockNotFound,
            #[error("inconsistent RPC response: {0}")]
            InconsistentResponse(&'static str),
        }

        impl DBErrorMarker for Error {}

        /// A [RevmDatabase] backed by an alloy [Provider].
        ///
        /// When accessing the database, it'll use the given provider to fetch the corresponding
        /// account's data. It will block the current thread to execute provider calls, Therefore,
        /// its methods must *not* be executed inside an async runtime, or it will panic when trying
        /// to block. If the immediate context is only synchronous, but a transitive caller is
        /// async, use [tokio::task::spawn_blocking] around the calls that need to be blocked.
        #[derive(Clone)]
        pub struct ProviderDb<N: Network, P: Provider<N>> {
            /// Provider to fetch the data from.
            provider: P,
            /// Configuration of the provider.
            provider_config: ProviderConfig,
            /// Hash of the block on which the queries will be based.
            block: BlockHash,
            /// Handle to the Tokio runtime.
            handle: Handle,
            /// Bytecode cache to allow querying bytecode by hash instead of address.
            contracts: B256HashMap<Bytecode>,

            phantom: PhantomData<N>,
        }

        /// Additional configuration for a [Provider].
        #[derive(Clone, Debug)]
        #[non_exhaustive]
        pub struct ProviderConfig {
            /// Max number of storage keys to request in a single `eth_getProof` call.
            pub eip1186_proof_chunk_size: usize,
        }

        impl Default for ProviderConfig {
            fn default() -> Self {
                Self {
                    eip1186_proof_chunk_size: 1000,
                }
            }
        }

        impl<N: Network, P: Provider<N>> ProviderDb<N, P> {
            /// Creates a new AlloyDb instance, with a [Provider] and a block.
            ///
            /// This will panic if called outside the context of a Tokio runtime.
            pub(crate) fn new(provider: P, config: ProviderConfig, block_hash: BlockHash) -> Self {
                Self {
                    provider,
                    provider_config: config,
                    block: block_hash,
                    handle: Handle::current(),
                    contracts: Default::default(),
                    phantom: PhantomData,
                }
            }

            /// Returns the [Provider].
            pub(crate) const fn provider(&self) -> &P {
                &self.provider
            }

            /// Returns the block hash used for the queries.
            pub(crate) const fn block(&self) -> BlockHash {
                self.block
            }

            /// Gets the bytecode located at the corresponding [Address].
            pub(crate) fn get_code_at(&mut self, address: Address) -> Result<Bytecode, Error> {
                trace!(%address, "eth_getCode");
                let get_code = self.provider.get_code_at(address).hash(self.block);
                let code = self
                    .handle
                    .block_on(get_code.into_future())
                    .map_err(|err| Error::Rpc("eth_getCode", err))?;

                if code.is_empty() {
                    return Ok(Bytecode::new());
                }

                let bytecode = Bytecode::new_raw(code.0.into());
                self.contracts
                    .insert(bytecode.hash_slow(), bytecode.clone());

                Ok(bytecode)
            }

            /// Get the EIP-1186 account and storage merkle proofs.
            pub(crate) async fn get_proof(
                &self,
                address: Address,
                mut keys: Vec<StorageKey>,
            ) -> Result<EIP1186AccountProofResponse, Error> {
                trace!(%address, num_keys=keys.len(), "eth_getProof");
                let block = self.block();

                // for certain RPC nodes it seemed beneficial when the keys are in the correct order
                keys.sort_unstable();

                let mut iter = keys.chunks(self.provider_config.eip1186_proof_chunk_size);
                // always make at least one call even if the keys are empty
                let mut account_proof = self
                    .provider()
                    .get_proof(address, iter.next().unwrap_or_default().into())
                    .hash(block)
                    .await
                    .map_err(|err| Error::Rpc("eth_getProof", err))?;
                if account_proof.address != address {
                    return Err(Error::InconsistentResponse(
                        "response does not match request",
                    ));
                }

                for keys in iter {
                    let proof = self
                        .provider()
                        .get_proof(address, keys.into())
                        .hash(block)
                        .await
                        .map_err(|err| Error::Rpc("eth_getProof", err))?;
                    // only the keys have changed, the account proof should not change
                    if proof.account_proof != account_proof.account_proof {
                        return Err(Error::InconsistentResponse(
                            "account_proof not consistent between calls",
                        ));
                    }

                    account_proof.storage_proof.extend(proof.storage_proof);
                }

                Ok(account_proof)
            }

            /// Get the EIP-1186 account and storage merkle proofs.
            pub(crate) fn get_proof_blocking(
                &self,
                address: Address,
                keys: Vec<StorageKey>,
            ) -> Result<EIP1186AccountProofResponse, Error> {
                self.handle.block_on(self.get_proof(address, keys))
            }
        }

        impl<N: Network, P: Provider<N>> RevmDatabase for ProviderDb<N, P> {
            type Error = Error;

            fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
                trace!(%address, "getAccountInfo");
                let f = async {
                    let get_nonce = self
                        .provider
                        .get_transaction_count(address)
                        .hash(self.block);
                    let get_balance = self.provider.get_balance(address).hash(self.block);
                    let get_code = self.provider.get_code_at(address).hash(self.block);

                    tokio::join!(
                        get_nonce.into_future(),
                        get_balance.into_future(),
                        get_code.into_future()
                    )
                };
                let (nonce, balance, code) = self.handle.block_on(f);

                let nonce = nonce.map_err(|err| Error::Rpc("eth_getTransactionCount", err))?;
                let balance = balance.map_err(|err| Error::Rpc("eth_getBalance", err))?;
                let code = code.map_err(|err| Error::Rpc("eth_getCode", err))?;
                let bytecode = Bytecode::new_raw(code.0.into());

                // if the account is empty return None
                // in the EVM, emptiness is treated as equivalent to nonexistence
                if nonce == 0 && balance.is_zero() && bytecode.is_empty() {
                    return Ok(None);
                }

                // index the code by its hash, so that we can later use code_by_hash
                let code_hash = bytecode.hash_slow();
                self.contracts.insert(code_hash, bytecode);

                Ok(Some(AccountInfo {
                    nonce,
                    balance,
                    code_hash,
                    code: None, // will be queried later using code_by_hash
                }))
            }

            fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
                if code_hash == KECCAK_EMPTY {
                    return Ok(Bytecode::new());
                }

                // this works because `basic` is always called first
                let code = self
                    .contracts
                    .get(&code_hash)
                    .expect("`basic` must be called first for the corresponding account");

                Ok(code.clone())
            }

            fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
                trace!(%address, %index, "eth_getStorageAt");
                let storage = self
                    .handle
                    .block_on(
                        self.provider
                            .get_storage_at(address, index)
                            .hash(self.block)
                            .into_future(),
                    )
                    .map_err(|err| Error::Rpc("eth_getStorageAt", err))?;

                Ok(storage)
            }

            fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
                trace!(number, "eth_getBlockByNumber");
                let block_response = self
                    .handle
                    .block_on(
                        self.provider
                            .get_block_by_number(number.into())
                            .into_future(),
                    )
                    .map_err(|err| Error::Rpc("eth_getBlockByNumber", err))?;
                let block = block_response.ok_or(Error::BlockNotFound)?;

                Ok(block.header().hash())
            }
        }
    }

    pub use provider::ProviderConfig;
    pub use provider::ProviderDb;

    /// A simple revm [RevmDatabase] wrapper that records all DB queries.
    #[derive(Clone, Default)]
    pub struct PreflightDb<D> {
        accounts: AddressHashMap<B256HashSet>,
        contracts: B256HashMap<Bytes>,
        block_hash_numbers: HashSet<BlockNumber>,

        code_addresses: B256Map<Address>,
        proofs: AccountProofs,
        inner: D,
    }

    #[derive(Clone, Default)]
    struct AccountProofs(AddressHashMap<AccountProof>);

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct AccountProof {
        /// The account information as stored in the account trie.
        account: Option<StateAccount>,
        /// The inclusion proof for this account.
        account_proof: Vec<Bytes>,
        /// The MPT inclusion proofs for several storage slots.
        storage_proofs: B256HashMap<StorageProof>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct StorageProof {
        /// The value that this key holds.
        value: StorageValue,
        /// In MPT inclusion proof for this particular slot.
        proof: Vec<Bytes>,
    }

    impl<D> Debug for PreflightDb<D> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PreflightDb")
                .field("accounts", &self.accounts)
                .field("contracts", &self.contracts)
                .field("block_hash_numbers", &self.block_hash_numbers)
                .finish()
        }
    }

    impl<D> PreflightDb<D> {
        /// Creates a new ProofDb instance, with a [RevmDatabase].
        pub(crate) fn new(db: D) -> Self
        where
            D: RevmDatabase,
        {
            Self {
                accounts: Default::default(),
                contracts: Default::default(),
                block_hash_numbers: Default::default(),
                code_addresses: Default::default(),
                proofs: Default::default(),
                inner: db,
            }
        }

        /// Returns the referenced contracts
        pub(crate) const fn contracts(&self) -> &B256HashMap<Bytes> {
            &self.contracts
        }
    }

    impl<N: Network, P: Provider<N>> PreflightDb<ProviderDb<N, P>> {
        /// Fetches all the EIP-1186 storage proofs from the `access_list` and stores them in the DB.
        pub(crate) async fn add_access_list(&mut self, access_list: &AccessList) -> Result<()> {
            for AccessListItem {
                address,
                storage_keys,
            } in &access_list.0
            {
                if let Some(keys) = self.proofs.missing_proof(address, storage_keys) {
                    let proof = self.inner.get_proof(*address, keys).await?;
                    self.proofs
                        .add(proof)
                        .context("invalid eth_getProof response")?;
                }
            }

            Ok(())
        }

        /// Returns the chain of ancestor headers starting from `start_hash`.
        ///
        /// This trace continues until it reaches a block number lower than the minimum
        /// number recorded in `self.block_hash_numbers`.
        pub(crate) async fn ancestor_proof(
            &self,
            start_hash: B256,
        ) -> Result<Vec<<N as Network>::HeaderResponse>> {
            let provider = self.inner.provider();
            let mut ancestors = Vec::new();
            let mut current_hash = start_hash;
            let mut min_number: Option<u64> = None;

            loop {
                let rpc_block = provider
                    .get_block_by_hash(current_hash)
                    .await
                    .context("eth_getBlockByHash failed")?
                    .with_context(|| format!("block {current_hash} not found"))?;
                let header = rpc_block.header().clone();

                // lazily determine the minimum block number on the first iteration
                let block_hash_min_number = *min_number.get_or_insert_with(|| {
                    *self
                        .block_hash_numbers
                        .iter()
                        .min()
                        .unwrap_or(&header.number())
                });

                current_hash = header.parent_hash();
                let block_number = header.number();
                ancestors.push(header);

                if block_number <= block_hash_min_number {
                    break;
                }
            }

            Ok(ancestors)
        }

        /// Returns the merkle proofs (sparse [MerkleTrie]) for the state and all storage queries
        /// recorded by the [RevmDatabase].
        pub(crate) async fn state_proof(
            &mut self,
        ) -> Result<(MerkleTrie, AddressMap<MerkleTrie>)> {
            // if no accounts were accessed, use the state root of the corresponding block as is
            if self.accounts.is_empty() {
                let hash = self.inner.block();
                let block = self
                    .inner
                    .provider()
                    .get_block_by_hash(hash)
                    .await
                    .context("eth_getBlockByHash failed")?
                    .with_context(|| format!("block {hash} not found"))?;

                return Ok((
                    MerkleTrie::from_digest(block.header().state_root()),
                    AddressMap::default(),
                ));
            }

            let proofs = &mut self.proofs;
            for (address, storage_keys) in &self.accounts {
                if let Some(keys) = proofs.missing_proof(address, storage_keys) {
                    let proof = self.inner.get_proof(*address, keys).await?;
                    proofs.add(proof).context("invalid eth_getProof response")?;
                }
            }

            let state_nodes = self
                .accounts
                .keys()
                .filter_map(|address| proofs.get(address))
                .flat_map(|proof| proof.account_proof.iter());
            let state_trie = MerkleTrie::from_rlp(state_nodes).context("accountProof invalid")?;

            let mut storage_tries: AddressMap<MerkleTrie> = AddressMap::default();
            for (address, storage_keys) in &self.accounts {
                // safe unwrap: added a proof for each account in the previous loop
                let proof = proofs.get(address).unwrap();

                // create a new trie for this root
                let storage_root = proof
                    .account
                    .map(|a| a.storage_root)
                    .unwrap_or(EMPTY_ROOT_HASH);
                let mut storage_trie = MerkleTrie::from_digest(storage_root);

                // hydrate the trie if storage slots were accessed
                if !storage_keys.is_empty() {
                    let storage_nodes = storage_keys
                        .iter()
                        .filter_map(|key| proof.storage_proofs.get(key))
                        .flat_map(|proof| proof.proof.iter());

                    storage_trie
                        .hydrate_from_rlp(storage_nodes)
                        .with_context(|| format!("invalid storage proof for address {address}"))?;
                }

                ensure!(
                    storage_trie.hash_slow() == storage_root,
                    "storage root mismatch"
                );
                storage_tries.insert(*address, storage_trie);
            }

            Ok((state_trie, storage_tries))
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub enum DbError {
        #[error("provider error")]
        Provider(#[from] provider::Error),
        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    impl DBErrorMarker for DbError {}

    impl<N: Network, P: Provider<N>> RevmDatabase for PreflightDb<ProviderDb<N, P>> {
        type Error = DbError;

        fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
            self.accounts.entry(address).or_default();

            let account = match self.proofs.get(&address) {
                Some(proof) => proof.account,
                None => {
                    let proof = self.inner.get_proof_blocking(address, vec![])?;
                    self.proofs.add(proof).context("invalid proof response")?
                }
            };
            let code_hash = account.map(|acc| acc.code_hash).unwrap_or(KECCAK256_EMPTY);
            if code_hash != KECCAK256_EMPTY {
                self.code_addresses.insert(code_hash, address);
            }

            Ok(account.map(|acc| AccountInfo {
                balance: acc.balance,
                nonce: acc.nonce,
                code_hash: acc.code_hash,
                code: None, // will be queried later using code_by_hash
            }))
        }

        fn code_by_hash(&mut self, hash: B256) -> Result<Bytecode, Self::Error> {
            let code = match self.code_addresses.get(&hash) {
                None => self.inner.code_by_hash(hash)?,
                Some(address) => self.inner.get_code_at(*address)?,
            };
            self.contracts.insert(hash, code.original_bytes());

            Ok(code)
        }

        fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
            let key = StorageKey::from(index);
            self.accounts.entry(address).or_default().insert(key);

            // try to get the storage value from the loaded proofs before querying the underlying DB
            match self
                .proofs
                .get(&address)
                .and_then(|account| account.storage_proofs.get(&key))
            {
                Some(storage_proof) => Ok(storage_proof.value),
                None => Ok(self.inner.storage(address, index)?),
            }
        }

        fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
            self.block_hash_numbers.insert(number);

            Ok(self.inner.block_hash(number)?)
        }
    }

    impl AccountProofs {
        fn get(&self, address: &Address) -> Option<&AccountProof> {
            self.0.get(address)
        }

        fn add(
            &mut self,
            proof_response: EIP1186AccountProofResponse,
        ) -> Result<Option<StateAccount>> {
            // extract the actual state account from the proof
            let account = decode_account(&proof_response).context("invalid account proof")?;

            // convert the response into a StorageProof
            let storage_proofs = proof_response
                .storage_proof
                .into_iter()
                .map(|proof| {
                    (
                        proof.key.as_b256(),
                        StorageProof {
                            value: proof.value,
                            proof: proof.proof,
                        },
                    )
                })
                .collect();

            match self.0.entry(proof_response.address) {
                hash_map::Entry::Occupied(mut entry) => {
                    let account_proof = entry.get_mut();
                    ensure!(
                        account_proof.account == account
                            && account_proof.account_proof == proof_response.account_proof,
                        "inconsistent account proof"
                    );
                    account_proof.storage_proofs = merge_checked_maps(
                        std::mem::take(&mut account_proof.storage_proofs),
                        storage_proofs,
                    );
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(AccountProof {
                        account,
                        account_proof: proof_response.account_proof,
                        storage_proofs,
                    });
                }
            }

            Ok(account)
        }

        fn missing_proof<'a>(
            &self,
            address: &Address,
            keys: impl IntoIterator<Item = &'a StorageKey>,
        ) -> Option<Vec<StorageKey>> {
            let Some(proof) = self.get(address) else {
                return Some(keys.into_iter().cloned().unique().collect());
            };

            let storage_root = proof.account.map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
            if storage_root == EMPTY_ROOT_HASH {
                return None;
            }

            let new_key = |k: &&StorageKey| !proof.storage_proofs.contains_key(*k);
            let missing_keys: Vec<_> =
                keys.into_iter().filter(new_key).cloned().unique().collect();

            // we only need to request additional proofs if some keys are missing
            if missing_keys.is_empty() {
                None
            } else {
                Some(missing_keys)
            }
        }
    }

    /// Merges two HashMaps, checking for consistency on overlapping keys.
    /// Panics if values for the same key are different. Consumes both maps.
    fn merge_checked_maps<K, V, S, T>(mut map: HashMap<K, V, S>, iter: T) -> HashMap<K, V, S>
    where
        K: Eq + Hash + Debug,
        V: PartialEq + Debug,
        S: BuildHasher,
        T: IntoIterator<Item = (K, V)>,
    {
        let iter = iter.into_iter();
        let (lower_bound, _) = iter.size_hint();
        map.reserve(lower_bound);

        for (key, value2) in iter {
            match map.entry(key) {
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(value2);
                }
                hash_map::Entry::Occupied(entry) => {
                    let value1 = entry.get();
                    if value1 != &value2 {
                        panic!(
                            "mismatching values for key {:?}: existing={:?}, other={:?}",
                            entry.key(),
                            value1,
                            value2
                        );
                    }
                }
            }
        }

        map
    }

    fn decode_account(proof_response: &EIP1186AccountProofResponse) -> Result<Option<StateAccount>> {
        let trie = Trie::from_rlp(&proof_response.account_proof)?;
        match trie.get(keccak256(proof_response.address)) {
            None => Ok(None),
            Some(rlp) => Ok(Some(decode_exact(rlp)?)),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::merge_checked_maps;
        use alloy_primitives::map::HashMap;

        #[test]
        fn merge_checked_maps_accepts_non_overlapping_keys() {
            let mut left: HashMap<u8, u8> = HashMap::default();
            left.insert(1, 10);
            let right = vec![(2, 20)];

            let merged = merge_checked_maps(left, right);
            assert_eq!(merged.get(&1), Some(&10));
            assert_eq!(merged.get(&2), Some(&20));
        }

        #[test]
        fn merge_checked_maps_accepts_matching_overlaps() {
            let mut left: HashMap<u8, u8> = HashMap::default();
            left.insert(1, 10);
            let right = vec![(1, 10), (2, 20)];

            let merged = merge_checked_maps(left, right);
            assert_eq!(merged.get(&1), Some(&10));
            assert_eq!(merged.get(&2), Some(&20));
        }

        #[test]
        #[should_panic(expected = "mismatching values for key")]
        fn merge_checked_maps_panics_on_conflicts() {
            let mut left: HashMap<u8, u8> = HashMap::default();
            left.insert(1, 10);
            let right = vec![(1, 11)];

            let _ = merge_checked_maps(left, right);
        }
    }
}

pub mod rpc {
    use alloy::{
        network::Network,
        primitives::{B256, U256},
        providers::Provider,
        serde::JsonStorageKey,
    };
    use alloy_primitives::{Address, keccak256};
    use anyhow::{Context, ensure};
    use async_trait::async_trait;
    use risc0_ethereum_trie::Nibbles;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use tracing::trace;

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StorageRangeQueryResponse {
        pub storage: HashMap<B256, StorageRangeQueryResponseEntry>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        pub next_key: Option<B256>,
    }

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct StorageRangeQueryResponseEntry {
        pub key: Option<JsonStorageKey>,
        pub value: U256,
    }

    /// An extension trait for Alloy providers that adds custom debug RPC methods.
    #[async_trait]
    pub trait DebugApi<N: Network>: Provider<N> {
        /// Fetches the next storage key for an address using `debug_storageRangeAt`.
        async fn get_next_storage_key(
            &self,
            block_hash: B256,
            address: Address,
            prefix: Nibbles,
        ) -> anyhow::Result<B256>;
    }

    #[async_trait]
    impl<P, N> DebugApi<N> for P
    where
        P: Provider<N>,
        N: Network,
    {
        async fn get_next_storage_key(
            &self,
            block_hash: B256,
            address: Address,
            prefix: Nibbles,
        ) -> anyhow::Result<B256> {
            trace!(%address, ?prefix, "debug_storageRangeAt");

            let start_key = B256::right_padding_from(&prefix.pack());
            let params = (block_hash, 0, address, start_key, 1);

            let response: StorageRangeQueryResponse = self
                .client()
                .request("debug_storageRangeAt", params)
                .await
                .context("debug_storageRangeAt failed")?;

            let (_, entry) = response
                .storage
                .into_iter()
                .next()
                .context("No storage slot returned from debug_storageRangeAt")?;

            let storage_key = entry
                .key
                .context("Preimage storage key is missing from debug response")?
                .as_b256();

            // perform simple sanity checks, as this RPC is known to be wonky
            ensure!(
                Nibbles::unpack(keccak256(storage_key)).starts_with(&prefix),
                "Invalid debug_storageRangeAt response"
            );

            Ok(storage_key)
        }
    }
}

pub mod trie {
    use super::rpc::DebugApi;
    use alloy::{
        network::Network,
        primitives::{Address, B256, keccak256, map::B256Set},
        providers::Provider,
    };
    use anyhow::{Context, Result, bail};
    use revm::database::StorageWithOriginalValues;
    use risc0_ethereum_trie::{Nibbles, Trie, orphan};
    use std::collections::HashSet;
    use tracing::{debug, trace};

    pub async fn handle_removed_account<P, N>(
        provider: &P,
        block_hash: B256,
        address: Address,
        state_trie: &mut Trie,
    ) -> Result<()>
    where
        P: Provider<N>,
        N: Network,
    {
        trace!(%address, "Hydrating proof for destroyed account");
        let proof = provider
            .get_proof(address, vec![])
            .hash(block_hash)
            .await
            .context("eth_getProof failed")?;
        state_trie.hydrate_from_rlp(&proof.account_proof)?;
        state_trie.resolve_orphan(keccak256(address), &proof.account_proof)?;

        Ok(())
    }

    pub async fn handle_new_account<P, N>(
        provider: &P,
        block_hash: B256,
        address: Address,
        state_trie: &mut Trie,
    ) -> Result<()>
    where
        P: Provider<N>,
        N: Network,
    {
        trace!(%address, "Hydrating proof for new account");
        let proof = provider
            .get_proof(address, vec![])
            .hash(block_hash)
            .await
            .context("eth_getProof failed")?;
        state_trie.hydrate_from_rlp(proof.account_proof)?;

        Ok(())
    }

    pub async fn handle_modified_account<P, N>(
        provider: &P,
        block_hash: B256,
        address: Address,
        storage: &StorageWithOriginalValues,
        storage_trie: &mut Trie,
    ) -> Result<()>
    where
        P: Provider<N>,
        N: Network,
    {
        // collect the storage keys for any new or removed slot
        let keys: Vec<B256> = storage
            .iter()
            .filter_map(|(key, slot)| {
                if slot.original_value().is_zero() != slot.present_value().is_zero() {
                    Some(B256::from(*key))
                } else {
                    None
                }
            })
            .collect();

        if keys.is_empty() {
            return Ok(());
        }

        trace!(%address, num_keys = keys.len(), "Hydrating proof for new or removed slots");
        let proof = provider
            .get_proof(address, keys)
            .hash(block_hash)
            .await
            .context("eth_getProof failed")?;

        let mut unresolvable: HashSet<Nibbles> = HashSet::default();
        for storage_proof in proof.storage_proof {
            let hashed_key = keccak256(storage_proof.key.as_b256());
            storage_trie.hydrate_from_rlp(&storage_proof.proof)?;
            if storage_proof.value.is_zero() {
                match storage_trie.resolve_orphan(hashed_key, &storage_proof.proof) {
                    Ok(_) => {}
                    Err(orphan::Error::Unresolvable(prefix)) => {
                        unresolvable.insert(prefix);
                    }
                    Err(err) => bail!(err),
                }
            }
        }

        if unresolvable.is_empty() {
            return Ok(());
        }

        debug!(%address, "Using debug_storageRangeAt to find preimages for orphan nodes");

        let mut missing_storage_keys = B256Set::default();
        for prefix in unresolvable {
            let storage_key = provider
                .get_next_storage_key(block_hash, address, prefix)
                .await?;
            missing_storage_keys.insert(storage_key);
        }

        if !missing_storage_keys.is_empty() {
            trace!(%address, keys=?missing_storage_keys, "Fetching final proofs for missing storage keys");
            let proof = provider
                .get_proof(address, missing_storage_keys.into_iter().collect())
                .hash(block_hash)
                .await
                .context("eth_getProof failed")?;

            storage_trie.hydrate_from_rlp(proof.storage_proof.iter().flat_map(|p| &p.proof))?;
        }

        Ok(())
    }
}

pub async fn execution_witness<E, P, N>(
    evm_config: E,
    provider: &P,
    block_id: BlockNumberOrTag,
) -> Result<ExecutionWitness>
where
    E: ConfigureEvm + 'static,
    P: Provider<N> + Clone + Send + Sync + 'static,
    N: Network,
    <E::Primitives as NodePrimitives>::Block: TryFrom<<N as Network>::BlockResponse>,
    <<E::Primitives as NodePrimitives>::Block as TryFrom<<N as Network>::BlockResponse>>::Error:
        std::error::Error + Send + Sync + 'static,
    <E::Primitives as NodePrimitives>::BlockHeader: TryFrom<<N as Network>::HeaderResponse>,
    <<E::Primitives as NodePrimitives>::BlockHeader as TryFrom<<N as Network>::HeaderResponse>>::Error:
        std::error::Error + Send + Sync + 'static,
{
    debug!(%block_id, "Fetching block data");
    let rpc_block = provider
        .get_block(block_id.into())
        .full()
        .await
        .context("eth_getBlock failed")?
        .with_context(|| format!("Block {block_id} not found"))?;
    let block_hash = rpc_block.header().hash();
    let parent_hash = rpc_block.header().parent_hash();

    let block: <E::Primitives as NodePrimitives>::Block = rpc_block.try_into()?;
    let recovered_block = block.try_into_recovered()?;

    let mut db = db::PreflightDb::new(db::ProviderDb::new(
        provider.clone(),
        db::ProviderConfig::default(),
        parent_hash,
    ));

    debug!(%block_hash, "Preprocessing transactions with access lists");
    for tx in recovered_block.body().transactions() {
        if let Some(access_list) = tx.access_list() {
            db.add_access_list(access_list).await?;
        }
    }

    debug!(%block_hash, "Executing block on dedicated thread");
    let current_span = Span::current();

    let (execution_result, db) = tokio::task::spawn_blocking(move || {
        current_span.in_scope(|| {
            let executor = evm_config.executor(db);
            let mut database_capture: Option<Box<db::PreflightDb<db::ProviderDb<N, P>>>> = None;
            let outcome = executor.execute_with_state_closure(&recovered_block, |state| {
                database_capture = Some(Box::new(state.database.clone()));
            });
            (outcome, database_capture)
        })
    })
    .await?;
    let execution_outcome = execution_result?;
    let mut db = db.unwrap();

    debug!("Building pre-state proofs");
    let (mut state_trie, mut storage_tries) = db.state_proof().await?;
    let ancestors = db
        .ancestor_proof(parent_hash)
        .await
        .context("failed to find ancestors")?;

    debug!("Building post-state proofs");
    for (addr, account) in execution_outcome.state.state {
        match (account.original_info.is_some(), account.info.is_some()) {
            (false, true) => {
                trie::handle_new_account(provider, block_hash, addr, &mut state_trie).await?
            }
            (true, false) => {
                trie::handle_removed_account(provider, block_hash, addr, &mut state_trie).await?
            }
            (true, true) => {
                let storage = storage_tries.get_mut(&addr).unwrap();
                trie::handle_modified_account(provider, block_hash, addr, &account.storage, storage)
                    .await?;
            }
            _ => {}
        }
    }

    // 5. Assemble the Execution Witness
    let mut state: HashSet<Bytes> = HashSet::new();
    state.extend(state_trie.rlp_nodes());
    for storage_trie in storage_tries.values() {
        state.extend(storage_trie.rlp_nodes());
    }

    let mut headers = Vec::new();
    for header in ancestors {
        let header: <E::Primitives as NodePrimitives>::BlockHeader = header.try_into()?;
        headers.push(alloy::rlp::encode(header).into());
    }

    debug!("Preflight check completed successfully");

    Ok(ExecutionWitness {
        state: state.into_iter().collect(),
        codes: db.contracts().values().cloned().collect(),
        keys: vec![],
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::execution_witness;
    use alloy::{
        eips::BlockNumberOrTag,
        providers::{Provider as AlloyProvider, ProviderBuilder},
        rpc::client::RpcClient,
    };
    use alethia_reth_block::config::TaikoEvmConfig;
    use alethia_reth_chainspec::{TAIKO_DEVNET, TAIKO_HOODI, TAIKO_MAINNET};
    use alloy_chains::NamedChain;
    use reth_chainspec::{HOLESKY, HOODI, MAINNET, SEPOLIA};
    use reth_evm_ethereum::EthEvmConfig;
    use std::sync::Arc;

    #[tokio::test]
    async fn execution_witness_from_env_rpc() {
        let rpc_url = std::env::var("ON_THE_SPOT_WITNESS_RPC_URL").ok();
        let Some(rpc_url) = rpc_url else {
            return;
        };

        let url = reqwest::Url::parse(&rpc_url).expect("Invalid ON_THE_SPOT_WITNESS_RPC_URL");
        let client = RpcClient::builder().http(url);
        let provider = ProviderBuilder::new().connect_client(client);

        let chain_id = provider
            .get_chain_id()
            .await
            .expect("eth_chainId failed");
        let block_id = std::env::var("ON_THE_SPOT_WITNESS_BLOCK")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(BlockNumberOrTag::from)
            .unwrap_or(BlockNumberOrTag::Latest);

        let witness = match chain_id {
            167000 => {
                let evm_config = Arc::new(TaikoEvmConfig::new(TAIKO_MAINNET.clone()));
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
            167001 => {
                let evm_config = Arc::new(TaikoEvmConfig::new(TAIKO_DEVNET.clone()));
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
            167013 => {
                let evm_config = Arc::new(TaikoEvmConfig::new(TAIKO_HOODI.clone()));
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
            _ => {
                let chain: NamedChain = chain_id.try_into().expect("Invalid chain_id");
                let evm_config = match chain {
                    NamedChain::Mainnet => Arc::new(EthEvmConfig::ethereum(MAINNET.clone())),
                    NamedChain::Holesky => Arc::new(EthEvmConfig::ethereum(HOLESKY.clone())),
                    NamedChain::Hoodi => Arc::new(EthEvmConfig::ethereum(HOODI.clone())),
                    NamedChain::Sepolia => Arc::new(EthEvmConfig::ethereum(SEPOLIA.clone())),
                    _ => panic!("Unsupported chain: {chain}"),
                };
                execution_witness(evm_config, &provider, block_id)
                    .await
                    .expect("execution_witness failed")
            }
        };

        assert!(!witness.state.is_empty(), "witness state should not be empty");
        assert!(
            !witness.headers.is_empty(),
            "witness headers should not be empty"
        );
    }
}
