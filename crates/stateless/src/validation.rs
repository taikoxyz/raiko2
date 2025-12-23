use std::{collections::BTreeMap, sync::Arc};

use alethia_reth_block_lite::config::TaikoEvmConfig;
use alethia_reth_chainspec_lite::spec::TaikoChainSpec;
use alloy_consensus::{BlockHeader, Header, TrieAccount};
use alloy_primitives::{B256, map::AddressMap};
use alloy_rlp::Decodable;
use reth_ethereum_primitives::Block;
use reth_primitives_traits::{Block as _, RecoveredBlock};
use reth_stateless::{ExecutionWitness, validation::StatelessValidationError};

const BLOCKHASH_ANCESTOR_LIMIT: usize = 256;

/// Stateless validation for zkVM-friendly builds.
///
/// This performs minimal checks that are compatible with zkVM targets:
/// - Ancestor headers must exist and be contiguous.
/// - The witness header chain must link back to the current block.
///
/// Full execution/consensus validation is expected to run in the host pipeline.
pub fn validate_block(
    block: Block,
    witness: ExecutionWitness,
    _callers: AddressMap<TrieAccount>,
    _chain_spec: Arc<TaikoChainSpec>,
    _config: TaikoEvmConfig,
) -> Result<B256, StatelessValidationError> {
    let current_block = block
        .try_into_recovered()
        .map_err(|_| StatelessValidationError::SignerRecovery)?;

    let mut ancestor_headers: Vec<Header> = witness
        .headers
        .iter()
        .map(|serialized_header| {
            let bytes = serialized_header.as_ref();
            Header::decode(&mut &bytes[..])
                .map_err(|_| StatelessValidationError::HeaderDeserializationFailed)
        })
        .collect::<Result<_, _>>()?;

    if ancestor_headers.is_empty() {
        return Err(StatelessValidationError::MissingAncestorHeader);
    }

    if ancestor_headers.len() > BLOCKHASH_ANCESTOR_LIMIT {
        return Err(StatelessValidationError::AncestorHeaderLimitExceeded {
            count: ancestor_headers.len(),
            limit: BLOCKHASH_ANCESTOR_LIMIT,
        });
    }

    ancestor_headers.sort_by_key(|header| header.number());

    compute_ancestor_hashes(&current_block, &ancestor_headers)?;

    Ok(current_block.hash_slow())
}

fn compute_ancestor_hashes(
    current_block: &RecoveredBlock<Block>,
    ancestor_headers: &[Header],
) -> Result<BTreeMap<u64, B256>, StatelessValidationError> {
    let mut ancestor_hashes = BTreeMap::new();

    let mut child_header = current_block.header();

    for parent_header in ancestor_headers.iter().rev() {
        let parent_hash = child_header.parent_hash();
        ancestor_hashes.insert(parent_header.number, parent_hash);

        if parent_hash != parent_header.hash_slow() {
            return Err(StatelessValidationError::InvalidAncestorChain);
        }

        if parent_header.number + 1 != child_header.number {
            return Err(StatelessValidationError::InvalidAncestorChain);
        }

        child_header = parent_header
    }

    Ok(ancestor_hashes)
}
