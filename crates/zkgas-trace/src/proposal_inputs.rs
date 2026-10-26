#![allow(clippy::redundant_pub_crate)]

use alloy_consensus::Transaction as _;
use alloy_eips::eip2718::Encodable2718 as _;
use anyhow::{Context, Result, bail};
use raiko2_primitives_shasta::GuestInput;
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalPayloadSelectors {
    pub total_transaction_count: u64,
    pub canonical_transaction_bytes: u64,
    pub nonempty_payload_transactions: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalBlobSelectors {
    pub source_blob_counts: Vec<u64>,
    pub proposal_blob_hash_count: u64,
    pub provider_blob_count: u64,
    pub provider_commitment_count: u64,
    pub provider_proof_count: u64,
}

#[derive(Default)]
pub(crate) struct PayloadAccumulator {
    total_transaction_count: u64,
    canonical_transaction_bytes: u64,
    nonempty_payload_transactions: u64,
}

impl PayloadAccumulator {
    pub(crate) fn record_block(
        &mut self,
        anchor: &TransactionSigned,
        candidates: &[TransactionSigned],
    ) -> Result<()> {
        self.record_transaction(anchor)?;
        for transaction in candidates {
            self.record_transaction(transaction)?;
        }
        Ok(())
    }

    fn record_transaction(&mut self, transaction: &TransactionSigned) -> Result<()> {
        self.total_transaction_count = self
            .total_transaction_count
            .checked_add(1)
            .context("proposal transaction count overflows")?;
        self.canonical_transaction_bytes = self
            .canonical_transaction_bytes
            .checked_add(
                u64::try_from(transaction.encoded_2718().len())
                    .context("canonical transaction length exceeds u64")?,
            )
            .context("canonical transaction byte total overflows")?;
        if !transaction.input().is_empty() {
            self.nonempty_payload_transactions = self
                .nonempty_payload_transactions
                .checked_add(1)
                .context("nonempty transaction count overflows")?;
        }
        Ok(())
    }

    pub(crate) const fn finish(self) -> ProposalPayloadSelectors {
        ProposalPayloadSelectors {
            total_transaction_count: self.total_transaction_count,
            canonical_transaction_bytes: self.canonical_transaction_bytes,
            nonempty_payload_transactions: self.nonempty_payload_transactions,
        }
    }
}

pub(crate) fn blob_selectors(input: &GuestInput) -> Result<ProposalBlobSelectors> {
    let proposal_sources = &input.taiko.proposal_event.proposal.sources;
    let provider_sources = &input.taiko.data_sources;
    if proposal_sources.is_empty() || proposal_sources.len() != provider_sources.len() {
        bail!("proposal requires matched non-empty proposal/provider sources");
    }

    let mut selectors = ProposalBlobSelectors {
        source_blob_counts: Vec::with_capacity(proposal_sources.len()),
        proposal_blob_hash_count: 0,
        provider_blob_count: 0,
        provider_commitment_count: 0,
        provider_proof_count: 0,
    };
    for (proposal, provider) in proposal_sources.iter().zip(provider_sources) {
        let add = |total: &mut u64, value: usize, label: &'static str| -> Result<()> {
            *total = total
                .checked_add(u64::try_from(value).with_context(|| format!("{label} exceeds u64"))?)
                .with_context(|| format!("{label} overflows"))?;
            Ok(())
        };
        let proposal_count = u64::try_from(proposal.blobSlice.blobHashes.len())
            .context("source blob count exceeds u64")?;
        selectors.source_blob_counts.push(proposal_count);
        add(
            &mut selectors.proposal_blob_hash_count,
            proposal.blobSlice.blobHashes.len(),
            "proposal blob hash count",
        )?;
        add(
            &mut selectors.provider_blob_count,
            provider.tx_data_from_blob.len(),
            "provider blob count",
        )?;
        add(
            &mut selectors.provider_commitment_count,
            provider.blob_commitments.len(),
            "provider commitment count",
        )?;
        add(
            &mut selectors.provider_proof_count,
            provider.blob_proofs.len(),
            "provider proof count",
        )?;
    }
    Ok(selectors)
}

pub(crate) fn guest_input_identity(input: &GuestInput) -> Result<(String, usize)> {
    let encoded = bincode::serialize(input).context("serialize current GuestInput")?;
    Ok((
        alloy_primitives::hex::encode(Sha256::digest(&encoded)),
        encoded.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_primitives::{Address, Bytes, Signature, TxKind, U256};
    use std::{fs, path::Path};

    fn transaction(payload: Bytes) -> TransactionSigned {
        TxEip1559 {
            chain_id: 167_000,
            nonce: 0,
            gas_limit: 30_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::with_last_byte(1)),
            value: U256::ZERO,
            access_list: Vec::new().into(),
            input: payload,
        }
        .into_signed(Signature::test_signature())
        .into()
    }

    #[test]
    fn payload_accumulator_counts_anchor_candidates_and_nonempty_payloads() {
        let anchor = transaction(Bytes::from_static(&[1, 2]));
        let candidates = [
            transaction(Bytes::new()),
            transaction(Bytes::from_static(&[3])),
        ];
        let expected_bytes = u64::try_from(
            anchor.encoded_2718().len()
                + candidates
                    .iter()
                    .map(|transaction| transaction.encoded_2718().len())
                    .sum::<usize>(),
        )
        .unwrap();
        let mut accumulator = PayloadAccumulator::default();
        accumulator.record_block(&anchor, &candidates).unwrap();

        assert_eq!(
            accumulator.finish(),
            ProposalPayloadSelectors {
                total_transaction_count: 3,
                canonical_transaction_bytes: expected_bytes,
                nonempty_payload_transactions: 2,
            }
        );
    }

    #[test]
    fn blob_selector_extraction_does_not_verify_cryptographic_proofs() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_41455_l2_12556056_12556247.json",
        );
        let mut input: GuestInput = serde_json::from_str(
            &fs::read_to_string(&fixture)
                .unwrap_or_else(|error| panic!("read {}: {error}", fixture.display())),
        )
        .expect("parse checked-in current GuestInput");
        input.taiko.data_sources[0].blob_proofs[0][0] ^= 0x01;

        assert_eq!(
            blob_selectors(&input).expect("selector extraction is structural"),
            ProposalBlobSelectors {
                source_blob_counts: vec![1],
                proposal_blob_hash_count: 1,
                provider_blob_count: 1,
                provider_commitment_count: 1,
                provider_proof_count: 1,
            }
        );
    }
}
