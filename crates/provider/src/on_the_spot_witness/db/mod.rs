use alloy::{
    consensus::BlockHeader,
    eips::eip2930::{AccessList, AccessListItem},
    network::{BlockResponse, Network},
    providers::Provider,
    rlp::decode_exact,
    rpc::types::EIP1186AccountProofResponse,
};
use alloy_primitives::{
    Address, B256, BlockNumber, Bytes, KECCAK256_EMPTY, StorageKey, StorageValue, U256, keccak256,
    map::{
        AddressHashMap, AddressMap, B256HashMap, B256HashSet, B256Map, HashMap, HashSet, hash_map,
    },
};
use alloy_trie::{EMPTY_ROOT_HASH, TrieAccount as StateAccount};
use anyhow::{Context, Result, anyhow, ensure};
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

mod provider;

pub use provider::{ProviderConfig, ProviderDb};

/// A simple `revm` [`RevmDatabase`] wrapper that records all DB queries.
#[derive(Clone, Default)]
pub struct PreflightDb<D> {
    accounts: AddressHashMap<B256HashSet>,
    contracts: B256HashMap<Bytes>,
    block_hash_numbers: HashSet<BlockNumber>,

    code_addresses: B256Map<Address>,
    proofs: AccountProofs,
    inner: D,
}

#[derive(Clone, Default, Debug)]
struct AccountProofs(AddressHashMap<AccountProof>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct AccountProof {
    /// The account information as stored in the account trie.
    account: Option<StateAccount>,
    /// The inclusion proof for this account.
    account_rlp_proof: Vec<Bytes>,
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
            .field("code_addresses", &self.code_addresses)
            .field("proofs", &self.proofs)
            .field("inner", &"<opaque>")
            .finish()
    }
}

impl<D> PreflightDb<D> {
    /// Creates a new `PreflightDb` instance, with a [`RevmDatabase`].
    pub(crate) fn new(db: D) -> Self
    where
        D: RevmDatabase,
    {
        Self {
            accounts: AddressHashMap::default(),
            contracts: B256HashMap::default(),
            block_hash_numbers: HashSet::default(),
            code_addresses: B256Map::default(),
            proofs: AccountProofs::default(),
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

    /// Returns the Merkle proofs (sparse [`MerkleTrie`]) for the state and all storage queries
    /// recorded by the [`RevmDatabase`].
    pub(crate) async fn state_proof(&mut self) -> Result<(MerkleTrie, AddressMap<MerkleTrie>)> {
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
            .flat_map(|proof| proof.account_rlp_proof.iter());
        let state_trie = MerkleTrie::from_rlp(state_nodes).context("accountProof invalid")?;

        let mut storage_tries: AddressMap<MerkleTrie> = AddressMap::default();
        for (address, storage_keys) in &self.accounts {
            let proof = proofs
                .get(address)
                .with_context(|| format!("missing proof for address {address}"))?;

            // create a new trie for this root
            let storage_root = proof.account.map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
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

        let account = if let Some(proof) = self.proofs.get(&address) {
            proof.account
        } else {
            let proof = self.inner.get_proof_blocking(address, vec![])?;
            self.proofs.add(proof).context("invalid proof response")?
        };
        let code_hash = account.map_or(KECCAK256_EMPTY, |acc| acc.code_hash);
        if code_hash != KECCAK256_EMPTY {
            self.code_addresses.insert(code_hash, address);
        }

        Ok(account.map(|acc| AccountInfo {
            account_id: None,
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

    fn add(&mut self, proof_response: EIP1186AccountProofResponse) -> Result<Option<StateAccount>> {
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
                        && account_proof.account_rlp_proof == proof_response.account_proof,
                    "inconsistent account proof"
                );
                account_proof.storage_proofs = merge_checked_maps(
                    std::mem::take(&mut account_proof.storage_proofs),
                    storage_proofs,
                )?;
            }
            hash_map::Entry::Vacant(entry) => {
                entry.insert(AccountProof {
                    account,
                    account_rlp_proof: proof_response.account_proof,
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
            return Some(keys.into_iter().copied().unique().collect());
        };

        let storage_root = proof.account.map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
        if storage_root == EMPTY_ROOT_HASH {
            return None;
        }

        let new_key = |k: &&StorageKey| !proof.storage_proofs.contains_key(*k);
        let missing_keys: Vec<_> = keys.into_iter().filter(new_key).copied().unique().collect();

        // we only need to request additional proofs if some keys are missing
        if missing_keys.is_empty() {
            None
        } else {
            Some(missing_keys)
        }
    }
}

/// Merges two `HashMaps`, checking for consistency on overlapping keys.
/// Returns an error if values for the same key are different. Consumes both maps.
fn merge_checked_maps<K, V, S, T>(mut map: HashMap<K, V, S>, iter: T) -> Result<HashMap<K, V, S>>
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
                    return Err(anyhow!(
                        "mismatching values for key {:?}: existing={:?}, other={:?}",
                        entry.key(),
                        value1,
                        value2
                    ));
                }
            }
        }
    }

    Ok(map)
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
    use super::Result;
    use super::merge_checked_maps;
    use alloy_primitives::map::HashMap;

    #[test]
    fn merge_checked_maps_accepts_non_overlapping_keys() -> Result<()> {
        let mut left: HashMap<u8, u8> = HashMap::default();
        left.insert(1, 10);
        let right = vec![(2, 20)];

        let merged = merge_checked_maps(left, right)?;
        assert_eq!(merged.get(&1), Some(&10));
        assert_eq!(merged.get(&2), Some(&20));
        Ok(())
    }

    #[test]
    fn merge_checked_maps_accepts_matching_overlaps() -> Result<()> {
        let mut left: HashMap<u8, u8> = HashMap::default();
        left.insert(1, 10);
        let right = vec![(1, 10), (2, 20)];

        let merged = merge_checked_maps(left, right)?;
        assert_eq!(merged.get(&1), Some(&10));
        assert_eq!(merged.get(&2), Some(&20));
        Ok(())
    }

    #[test]
    fn merge_checked_maps_rejects_conflicts() {
        let mut left: HashMap<u8, u8> = HashMap::default();
        left.insert(1, 10);
        let right = vec![(1, 11)];

        let err = merge_checked_maps(left, right).expect_err("merge should fail");
        assert!(err.to_string().contains("mismatching values for key"));
    }
}
