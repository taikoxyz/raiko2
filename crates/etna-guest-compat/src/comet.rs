use alloy_primitives::{B256, U256, keccak256};
use etna_core::{
    comet::{
        codec::{CodecError, decode_canonical_block, decode_canonical_commit},
        native::{Commit, CommitSig, RawBlock, RawCommit, Signature},
        validators::Validators,
        verify::{
            BftError, CertifiedBlockRef, block_parts, header_hash, verify_body_at_origin,
            verify_native_commit,
        },
    },
    domain::{DeploymentDomain, chain_id},
    key::ConsensusKey,
    types::Member,
};
use k256::ecdsa::SigningKey;
use prost::Message;
use serde_json::Value;
use tendermint::{Hash, Time};
use tendermint_proto::v0_38::types::{CanonicalBlockId, CanonicalPartSetHeader, CanonicalVote};

use super::{text, word};

const BLOCK: &[u8] = include_bytes!("../fixtures/nil-at-unix-epoch.block.pb");
const COMMIT: &[u8] = include_bytes!("../fixtures/nil-at-unix-epoch.commit.pb");

pub(super) fn check(vector: &Value, domain: &DeploymentDomain, chain: &str) -> [B256; 3] {
    let (raw, block) = decode_canonical_block(BLOCK).unwrap();
    let commit = decode_canonical_commit(COMMIT).unwrap();
    let keys = [1, 2].map(|seed| SigningKey::from_slice(&[seed; 32]).unwrap());
    let (first, second) = text(&vector["powers"]).split_once(',').unwrap();
    let validators = validators(&keys, [first.parse().unwrap(), second.parse().unwrap()]);
    let qc = |candidate: &Commit, chain: &str| {
        verify_native_commit(candidate, &block.header, &validators, chain)
    };
    qc(&commit, chain).unwrap();

    let height = vector["height"].as_u64().unwrap();
    let body = |candidate: &Commit| {
        let transaction = verify_body_at_origin(
            CertifiedBlockRef {
                block: &block,
                raw_block: BLOCK,
                commit: candidate,
            },
            height,
            B256::ZERO,
            0,
        )?;
        assert_eq!(transaction, block.data[0].as_slice());
        Ok(())
    };
    body(&commit).unwrap();

    let header = hash_word(header_hash(&block.header));
    let parts = block_parts(&raw).unwrap();
    let parts_hash = hash_word(parts.hash);
    let validator_hash = hash_word(validators.hash());
    assert_eq!(header, word(&vector["header_hash"]));
    assert_eq!(parts_hash, word(&vector["parts_hash"]));
    assert_eq!(
        u64::from(parts.total),
        vector["parts_total"].as_u64().unwrap()
    );
    assert_eq!(validator_hash, word(&vector["validator_hash"]));

    // A 30/40 block vote is already quorate. Corrupt the later nil slot, which also must verify.
    assert_eq!(validators.validators()[0].power(), 30);
    assert_eq!(validators.total_voting_power(), 40);
    assert!(matches!(
        commit.signatures[0],
        CommitSig::BlockIdFlagCommit { .. }
    ));
    let mut nil_only = commit.clone();
    nil_only.signatures[0] = CommitSig::BlockIdFlagAbsent;
    assert!(matches!(
        qc(&nil_only, chain),
        Err(BftError::Certificate(message)) if message.starts_with("insufficient commit power 0/40;")
    ));

    let mut invalid = commit.clone();
    let CommitSig::BlockIdFlagNil { signature, .. } = &mut invalid.signatures[1] else {
        panic!("stock nil signature must follow the quorate commit vote");
    };
    let mut corrupt = signature.as_ref().unwrap().as_bytes().to_vec();
    corrupt[64] ^= 1;
    *signature = Some(Signature::try_from(corrupt).unwrap());
    assert_bad_signature(qc(&invalid, chain));
    let mut short = RawCommit::from(commit.clone());
    short.signatures[1].signature.pop();
    assert!(matches!(
        decode_canonical_commit(&short.encode_to_vec()),
        Err(CodecError::InvalidCommit(_))
    ));

    // Alter only the part set, preserving the block header hash used by the certificate.
    let mut wrong_part_set = commit.clone();
    wrong_part_set.block_id.part_set_header.hash = Hash::Sha256([9; 32]);
    assert_bad_signature(qc(&wrong_part_set, chain));
    assert!(matches!(
        body(&wrong_part_set),
        Err(BftError::Commitment("block part set"))
    ));

    let next_generation = DeploymentDomain {
        generation: domain.generation + 1,
        ..*domain
    };
    let wrong_deployment = DeploymentDomain {
        l2_chain_id: domain.l2_chain_id + U256::from(1),
        ..*domain
    };
    for wrong_domain in [next_generation, wrong_deployment] {
        let wrong_chain = chain_id(&wrong_domain).unwrap();
        assert_ne!(wrong_chain, chain);
        assert!(matches!(
            qc(&commit, &wrong_chain),
            Err(BftError::Certificate(message)) if message == "chain id or commit height mismatch"
        ));
    }

    check_strict_quorum(raw, commit, &keys, chain);
    [header, parts_hash, validator_hash]
}

fn assert_bad_signature(result: Result<(), BftError>) {
    assert!(matches!(
        result,
        Err(BftError::Certificate(message)) if message == "native secp256k1eth signature verification failed"
    ));
}

fn check_strict_quorum(
    mut block: RawBlock,
    mut commit: Commit,
    keys: &[SigningKey; 2],
    chain: &str,
) {
    let quorum = validators(keys, [1, 2]);
    let header = block.header.as_mut().unwrap();
    header.validators_hash = quorum.hash().as_bytes().to_vec();
    header.next_validators_hash = header.validators_hash.clone();
    let raw = block.encode_to_vec();
    let (_, block_view) = decode_canonical_block(&raw).unwrap();
    commit.block_id.hash = header_hash(&block_view.header);
    commit.block_id.part_set_header = block_parts(&block).unwrap();

    // Generate only this synthetic control's signatures using stock protobuf fields.
    // The signed fixture from CometBFT above independently checks the deployed encoding.
    let timestamp = Time::from_unix_timestamp(101, 0).unwrap();
    let vote = CanonicalVote {
        r#type: 2,
        height: i64::try_from(commit.height.value()).unwrap(),
        round: i64::from(commit.round.value()),
        block_id: Some(CanonicalBlockId {
            hash: commit.block_id.hash.as_bytes().to_vec(),
            part_set_header: Some(CanonicalPartSetHeader {
                total: commit.block_id.part_set_header.total,
                hash: commit.block_id.part_set_header.hash.as_bytes().to_vec(),
            }),
        }),
        timestamp: Some(timestamp.into()),
        chain_id: chain.into(),
    };
    let digest = keccak256(vote.encode_length_delimited_to_vec());
    commit.signatures = quorum
        .validators()
        .iter()
        .map(|validator| {
            let key = keys
                .iter()
                .find(|key| public_key(key) == validator.pub_key)
                .unwrap();
            let (signature, recovery) = key.sign_prehash_recoverable(digest.as_slice()).unwrap();
            let mut bytes = signature.to_bytes().to_vec();
            bytes.push(recovery.to_byte());
            CommitSig::BlockIdFlagCommit {
                validator_address: validator.address,
                timestamp,
                signature: Some(Signature::try_from(bytes).unwrap()),
            }
        })
        .collect();

    verify_native_commit(&commit, &block_view.header, &quorum, chain).unwrap();
    assert_eq!(quorum.validators()[0].power(), 2);
    assert_eq!(quorum.validators()[1].power(), 1);
    commit.signatures[1] = CommitSig::BlockIdFlagAbsent;
    assert!(matches!(
        verify_native_commit(&commit, &block_view.header, &quorum, chain),
        Err(BftError::Certificate(message)) if message.starts_with("insufficient commit power 2/3;")
    ));
}

fn validators(keys: &[SigningKey; 2], powers: [u64; 2]) -> Validators {
    let [first, second] = keys;
    Validators::from_members(&[
        Member {
            pubkey: public_key(first),
            power: powers[0],
            eff_stake: U256::ZERO,
        },
        Member {
            pubkey: public_key(second),
            power: powers[1],
            eff_stake: U256::ZERO,
        },
    ])
    .unwrap()
}

fn public_key(key: &SigningKey) -> ConsensusKey {
    ConsensusKey::from_slice(key.verifying_key().to_encoded_point(true).as_bytes()).unwrap()
}

fn hash_word(hash: Hash) -> B256 {
    B256::from_slice(hash.as_bytes())
}
