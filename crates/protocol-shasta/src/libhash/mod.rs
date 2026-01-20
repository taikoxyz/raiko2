// rust impl of taiko-mono/packages/protocol/contracts/layer1/shasta/libs/LibHashing.sol

mod derivation;
mod encode;
mod shasta;
mod values;

pub use derivation::{hash_derivation, hash_derivation_source};
pub use encode::{VERIFY_PROOF_B256, address_to_b256, u48_to_b256, u64_to_b256};
pub use shasta::{
    hash_checkpoint, hash_commitment, hash_core_state, hash_proposal, hash_public_input,
    hash_shasta_subproof_input, hash_shasta_transition_input,
};
pub use values::{
    hash_five_values, hash_four_values, hash_six_values, hash_three_values, hash_two_values,
};

#[cfg(test)]
mod test {
    use crate::shasta::{BlobSlice, Derivation, DerivationSource, Proposal};
    use alloy_primitives::{Address, B256, Uint, address, b256, hex};

    use super::*;

    #[test]
    fn test_hash_proposal() {
        let proposal = Proposal {
            id: Uint::from(12_345u64),
            timestamp: Uint::from(193_828_690u64),
            endOfSubmissionWindowTimestamp: Uint::from(193_829_690u64),
            proposer: address!("1234567890abcdef1234567890abcdef12345678"),
            parentProposalHash: b256!(
                "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
            ),
            originBlockNumber: Uint::from(73_826u64),
            originBlockHash: b256!(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            ),
            basefeeSharingPctg: 42,
            sources: vec![
                DerivationSource {
                    isForcedInclusion: true,
                    blobSlice: BlobSlice {
                        blobHashes: vec![b256!(
                            "67890abcdef1234567890abcdef123451234567890abcdef1234567890abcdef"
                        )],
                        offset: Uint::from(0u32),
                        timestamp: Uint::from(100u64),
                    },
                },
                DerivationSource {
                    isForcedInclusion: false,
                    blobSlice: BlobSlice {
                        blobHashes: vec![b256!(
                            "567890abcdef123451234567890abcdef123456767890abcdef1234890abcdef"
                        )],
                        offset: Uint::from(100u32),
                        timestamp: Uint::from(200u64),
                    },
                },
            ],
        };
        let proposal_hash = hash_proposal(&proposal);
        assert_eq!(
            hex::encode(proposal_hash),
            "13af2d05799894db3462512e3ecf5ae8877b80b1e2db3963654ac70f6dd49f88"
        );
    }

    #[test]
    fn test_hash_derivation_empty_source() {
        // Create a test derivation with one source
        let derivation = Derivation {
            originBlockNumber: Uint::from(155u64),
            originBlockHash: b256!(
                "10746c6d70f2b59483dc2e0a1315758799fb3655f87e430568e71591589f76f9"
            ),
            basefeeSharingPctg: 75,
            sources: Vec::new(),
        };

        let derivation_hash = hash_derivation(&derivation);

        // The hash should be deterministic and match the expected value
        // This test verifies the implementation works without errors
        assert_ne!(derivation_hash, B256::ZERO);
        assert_eq!(
            hex::encode(derivation_hash),
            "1da64d2dd5bda3fb186ecf02433b32f1a24661030600a8ff150ed8c346dcc5ba"
        );
    }

    #[test]
    fn test_hash_derivation() {
        // Create a test derivation with one source
        let derivation = Derivation {
            originBlockNumber: Uint::from(155u64),
            originBlockHash: b256!(
                "10746c6d70f2b59483dc2e0a1315758799fb3655f87e430568e71591589f76f9"
            ),
            basefeeSharingPctg: 75,
            sources: vec![DerivationSource {
                isForcedInclusion: false,
                blobSlice: BlobSlice {
                    blobHashes: vec![b256!(
                        "0189ea2792db70c7d2165c397be7bc37b7d45b1ed082bec866e9cb62e90cb4a0"
                    )],
                    offset: Uint::from(0u32),
                    timestamp: Uint::from(1758948572u64),
                },
            }],
        };

        let derivation_hash = hash_derivation(&derivation);

        // The hash should be deterministic and match the expected value
        // This test verifies the implementation works without errors
        assert_ne!(derivation_hash, B256::ZERO);
        println!("Derivation hash: 0x{}", hex::encode(derivation_hash));
    }

    #[test]
    fn test_hash_public_input() {
        let aggregated_proving_hash =
            b256!("b836ee1f972e8bcd4766bede4a9fa5267d8b6ec7cd6088562aca0b07b15f57bc");
        let chain_id = 167001u64;
        let verifier_address = address!("00f9f60C79e38c08b785eE4F1a849900693C6630");
        let public_input_hash = hash_public_input(
            aggregated_proving_hash,
            chain_id,
            verifier_address,
            Address::ZERO,
        );
        assert_eq!(
            hex::encode(public_input_hash),
            "6d0ea3eb338aa3e2d85b21394d3ea426574ab7764726376a5364dee132fcd3d7"
        );
    }

    #[test]
    fn test_hash_prove_input() {
        // Setup a sample ProveInput with minimal structure to test only that hash_prove_input is called and behaves as expected.
        // This matches the test structure and dummy field values from the Solidity reference.

        let prove_input = crate::shasta::Commitment {
            firstProposalId: Uint::from(42u64),
            firstProposalParentBlockHash: b256!(
                "0000000000000000000000000000000000000000000000000000000000000999"
            ),
            lastProposalHash: b256!(
                "0000000000000000000000000000000000000000000000000000000000123456"
            ),
            actualProver: address!("0000000000000000000000000000000000012345"),
            endBlockNumber: Uint::from(1000u64),
            endStateRoot: b256!("0000000000000000000000000000000000000000000000000000000000abcdef"),
            transitions: vec![],
        };
        let prove_input_hash = hash_commitment(&prove_input);
        assert_eq!(
            alloy_primitives::hex::encode_prefixed(prove_input_hash),
            "0x0f1c0b0391c2617d236a059287ba55aeaa668cacfcd9abf6d537de314ae9fce8"
        );
    }
}
