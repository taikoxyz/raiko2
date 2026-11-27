//! Oracle values obtained independently from frozen LibPublicInput.sol and Foundry cast.
use alloy_primitives::{Address, B256, U256, address, b256, keccak256};
use raiko2_protocol_shasta::libhash::hash_public_input;

fn manual_input(statement: B256, verifier: Address, signer: Address, chain: u64) -> B256 {
    let mut words = [0u8; 160];
    words[..12].copy_from_slice(b"VERIFY_PROOF");
    words[32..64].copy_from_slice(&U256::from(chain).to_be_bytes::<32>());
    words[64..96].copy_from_slice(verifier.into_word().as_slice());
    words[96..128].copy_from_slice(statement.as_slice());
    words[128..160].copy_from_slice(signer.into_word().as_slice());
    keccak256(words)
}

#[test]
fn independent_sgx_contract_five_word_vectors_and_full_chain_width() {
    for (chain, verifier, statement, signer, expected) in [
        (
            167_001,
            Address::repeat_byte(0x11),
            B256::repeat_byte(0x22),
            Address::repeat_byte(0x33),
            b256!("d8db0a864c6ad06d00c7217fdf0c04bbc8f024f4e773a4d1f06ba331cbb0e1c8"),
        ),
        (
            u64::MAX,
            address!("0102030405060708090a0b0c0d0e0f1011121314"),
            B256::repeat_byte(0xfe),
            address!("14131211100f0e0d0c0b0a090807060504030201"),
            b256!("6f271d576dc13b09921705c615c4d7213fbd1b380491e3c30654aada08397b33"),
        ),
        (
            0,
            Address::ZERO,
            B256::repeat_byte(1),
            Address::ZERO,
            b256!("b087e938d4ff58461f4305a54e339cc40ff0d6652e9e2dd7c328cf6efa4d9c7f"),
        ),
    ] {
        assert_eq!(manual_input(statement, verifier, signer, chain), expected);
        assert_eq!(
            hash_public_input(statement, chain, verifier, signer),
            expected
        );
    }
}

#[test]
fn independent_sgx_domain_binds_chain_verifier_statement_and_actual_signer() {
    let statement = B256::repeat_byte(0x22);
    let verifier = Address::repeat_byte(0x11);
    let signer = Address::repeat_byte(0x33);
    let original = hash_public_input(statement, 167_001, verifier, signer);
    for (s, v, p, c) in [
        (statement, verifier, signer, 167_002),
        (statement, Address::repeat_byte(0xaa), signer, 167_001),
        (B256::repeat_byte(0xaa), verifier, signer, 167_001),
        (statement, verifier, Address::repeat_byte(0xaa), 167_001),
    ] {
        assert_ne!(hash_public_input(s, c, v, p), original);
    }
    assert_ne!(original, statement);
    assert_ne!(original, keccak256(statement));
}
