use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use raiko2_primitives_shasta::GuestInput;
use raiko2_protocol_shasta::shasta::{ProofCarryData, TransitionInputData};
use serde::de::DeserializeOwned;

pub fn read_json_file<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&contents).context("parse input JSON")
}

pub fn proof_carry_data_for_guest_input(input: &GuestInput) -> ProofCarryData {
    // Chain id must match what the guest expects for the witness' chain spec.
    // Prefer the witness' chain id (it is what the pipeline executes against), and fall back to
    // the manifest chain id if the witness list is empty.
    let chain_id = input
        .witnesses
        .first()
        .map(|w| w.chain_spec.chain_id)
        .filter(|&id| id != 0)
        .unwrap_or(input.taiko.chain_spec.chain_id);

    ProofCarryData {
        chain_id,
        verifier: Default::default(),
        transition_input: TransitionInputData {
            proposal_id: input.taiko.proposal_id,
            ..Default::default()
        },
    }
}

