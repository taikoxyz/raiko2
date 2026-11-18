//! Run a deterministic Etna compatibility diagnostic on the host or in a zkVM guest.
//!
//! Every case is fixed test data. The journal identifies the exercised computations.

#![no_std]

extern crate alloc;

mod comet;

use alloc::vec::Vec;
use alloy_primitives::{Address, B256, U256, hex, keccak256};
use etna_core::{
    committee::{
        CommitteeError, Snapshot, committee_config_hash, derive, entries_root, record_hash,
    },
    domain::{DeploymentDomain, chain_id},
    key::ConsensusKey,
    params::ChainParams,
    schedule::Schedule,
    types::RegistryEntry,
    valstate::ValState,
};
use serde_json::Value;

/// Run all positive and negative controls, then serialize the computed diagnostic journal.
pub fn run() -> Vec<u8> {
    let vectors: Value = serde_json::from_str(include_str!("../fixtures/vectors.json")).unwrap();
    assert_eq!(vectors["schema_version"], "7");
    let domain = &vectors["domain"];
    let domain = DeploymentDomain {
        l1_chain_id: amount(&domain["l1_chain_id"]),
        l2_chain_id: amount(&domain["l2_chain_id"]),
        inbox: text(&domain["inbox"]).parse().unwrap(),
        generation: number(&domain["generation"]),
    };
    let chain = chain_id(&domain).unwrap();
    assert_eq!(chain, text(&vectors["domain"]["chain_id"]));

    let mut journal = b"raiko2-etna-guest-compat-v1\0".to_vec();
    journal.extend_from_slice(chain.as_bytes());
    let words = comet::check(&vectors["comet"], &domain, &chain)
        .into_iter()
        .chain(check_committee(&vectors, &domain))
        .chain(check_valstate(&vectors["valstate"]))
        .chain(check_cut(&vectors));
    for word in words {
        journal.extend_from_slice(word.as_slice());
    }
    journal
}

fn check_committee(vectors: &Value, domain: &DeploymentDomain) -> [B256; 4] {
    let config = &vectors["committee_config"];
    let input = &config["inputs"];
    let params = ChainParams {
        l1_chain_id: domain.l1_chain_id,
        l2_chain_id: number(&input["l2_chain_id"]),
        inbox: domain.inbox,
        registry: text(&input["registry"]).parse().unwrap(),
        unsettled_limit: number(&input["unsettled_limit"]),
        fee_recipient: Address::ZERO,
        vp_unit: amount(&input["vp_unit"]),
        s_min: amount(&input["s_min"]),
        n_max: number(&input["n_max"]).try_into().unwrap(),
        block_gas_limit: number(&vectors["deployment"]["l2_block_gas_limit"]),
        switch: None,
    };
    params.validate().unwrap();
    let config_hash = committee_config_hash(&params);
    assert_eq!(config_hash, word(&config["hash"]));

    let registry = &vectors["registry"];
    let committee = &vectors["committee"];
    let cutoff = number(&committee["cutoff_l1_block"]);
    let epoch = number(&committee["target_epoch"]);
    let mut snapshot = Snapshot {
        checkpoint_index: number(&committee["checkpoint_index"]),
        l1_block: cutoff,
        entries: registry["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| RegistryEntry {
                ck_prefix: number(&entry["ck_prefix"]).try_into().unwrap(),
                ck_x: word(&entry["ck_x"]),
                eff_stake: amount(&entry["eff_stake"]),
                active_from_l1: number(&entry["active_from_l1"]),
                exit_effective_l1: number(&entry["exit_effective_l1"]),
            })
            .collect(),
    };
    let registry_root = entries_root(&snapshot.entries);
    assert_eq!(registry_root, word(&registry["entries_root"]));

    let (record, members) = derive(&snapshot, cutoff, epoch, &params).unwrap();
    assert_eq!(members.len(), 5);
    assert_eq!(record.total_power, number(&committee["total_power"]));
    assert_eq!(record.set_root, word(&committee["set_root"]));
    let record_digest = record_hash(params.l2_chain_id, &record);
    assert_eq!(record_digest, word(&committee["record_hash"]));

    snapshot.entries.push(snapshot.entries[0].clone());
    assert!(matches!(
        derive(&snapshot, cutoff, epoch, &params),
        Err(CommitteeError::DuplicateKey)
    ));
    [config_hash, registry_root, record.set_root, record_digest]
}

fn check_valstate(vector: &Value) -> Vec<B256> {
    let start = number(&vector["start_height"]);
    let schedule = Schedule {
        l1_0: 100,
        epoch_len: number(&vector["epoch_length"]),
        genesis_cutoff: 99,
    };
    schedule.validate().unwrap();

    let switch = &vector["switch"];
    let height = number(&switch["height"]);
    let epoch = number(&switch["epoch"]);
    let activation = number(&switch["activation_height"]);
    assert_eq!(schedule.switch_target(height), Some(epoch));
    assert_eq!(schedule.h_first(epoch), activation);
    assert_eq!(activation, height + 2);
    for pending_height in [height, height + 1] {
        assert_eq!(
            schedule.active_committee_epoch(pending_height, start),
            Some(0)
        );
    }
    assert_eq!(
        schedule.active_committee_epoch(activation, start),
        Some(epoch)
    );
    assert_eq!(
        schedule.active_committee_epoch_with_initial(activation, height, 0),
        Some(0)
    );

    let parse_changes = |rows: &Value| -> Vec<_> {
        rows.as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let key = vector["keys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|key| key["name"] == row[0])
                    .unwrap();
                (
                    ConsensusKey::from_slice(&bytes(&key["public_key"])).unwrap(),
                    number(&row[1]),
                )
            })
            .collect()
    };
    let mut state = ValState::genesis(start, &parse_changes(&vector["genesis"])).unwrap();
    assert_eq!(state.encode(), bytes(&vector["start_preimage"]));
    let changes = parse_changes(&switch["changes"]);
    let old_set = state.current.validators_hash().unwrap();

    let mut digests = Vec::new();
    for (index, expected) in vector["hashes"].as_array().unwrap().iter().enumerate() {
        let block_height = start + u64::try_from(index).unwrap();
        if block_height == height {
            let without_switch = state.advance(&[]).unwrap();
            assert_ne!(without_switch.hash(), word(expected));
            state = state.advance(&changes).unwrap();
        } else if block_height > start {
            state = state.advance(&[]).unwrap();
        }
        assert_eq!(state.height, block_height);
        assert_eq!(ValState::decode(&state.encode()).unwrap(), state);
        let digest = state.hash();
        assert_eq!(digest, word(expected), "ValState at height {block_height}");
        // These are the sets for the next two votes after committing block_height.
        let current = state.current.validators_hash().unwrap();
        let next = state.next.validators_hash().unwrap();
        assert_eq!(current == old_set, block_height + 1 < activation);
        assert_eq!(next == old_set, block_height + 2 < activation);
        digests.push(digest);
    }
    assert_eq!(state.height, activation);
    digests
}

fn check_cut(vectors: &Value) -> [B256; 3] {
    let leaves = vectors["cut_leaves"].as_array().unwrap();
    let [first, second] = <&[Value; 2]>::try_from(leaves.as_slice()).unwrap();
    let [first, second] = [first, second].map(|leaf| {
        let preimage = bytes(&leaf["preimage"]);
        assert_eq!(preimage.len(), 5 * 32);
        let digest = keccak256(preimage);
        assert_eq!(digest, word(&leaf["leaf"]));
        digest
    });

    let cut = &vectors["cut"];
    let leaves_digest = keccak256([first.0, second.0].concat());
    assert_eq!(leaves_digest, word(&cut["leaves_hash"]));
    assert_ne!(leaves_digest, keccak256([second.0, first.0].concat()));
    let preimage = bytes(&cut["preimage"]);
    assert_eq!(preimage.len(), 5 * 32);
    assert_eq!(&preimage[4 * 32..], leaves_digest.as_slice());
    let cut_digest = keccak256(preimage);
    assert_eq!(cut_digest, word(&cut["cut_hash"]));
    [first, second, cut_digest]
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn number(value: &Value) -> u64 {
    text(value).parse().unwrap()
}

fn amount(value: &Value) -> U256 {
    U256::from_str_radix(text(value), 10).unwrap()
}

fn word(value: &Value) -> B256 {
    B256::from_slice(&bytes(value))
}

fn bytes(value: &Value) -> Vec<u8> {
    let input = text(value);
    hex::decode(input.strip_prefix("0x").unwrap_or(input)).unwrap()
}
