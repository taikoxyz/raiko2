use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use raiko2_pipeline::proposal::validate_proposal_guest_input;
use raiko2_primitives::ProofType;
use raiko2_primitives_shasta::{
    DEFAULT_GUEST_INPUT_ROOT, GuestInput, build_proof_carry_data_from_witness_spec,
    guest_input_proposal_path, guest_input_proposals_dir, guest_input_suite_path,
    parse_guest_input_proposal_id,
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde::{Deserialize, Serialize};

#[derive(Args)]
pub(crate) struct ReplayGuestInputArgs {
    /// Network key under the guest input fixture root.
    #[arg(long)]
    pub(crate) network: String,

    /// Replay one proposal id.
    #[arg(long)]
    pub(crate) proposal: Option<u64>,

    /// Replay proposal ids listed in a suite manifest.
    #[arg(long)]
    pub(crate) suite: Option<String>,

    /// Replay every proposal fixture for the network.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    pub(crate) all: bool,

    /// Root directory for repo-managed guest input fixtures.
    #[arg(long, default_value = DEFAULT_GUEST_INPUT_ROOT)]
    pub(crate) guest_input_root: PathBuf,

    /// Stop after the first failed proposal.
    #[arg(long, value_parser = clap::builder::BoolishValueParser::new(), default_value = "false")]
    pub(crate) fail_fast: bool,

    /// Optional path to write a JSON replay report.
    #[arg(long)]
    pub(crate) json_out: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct SuiteManifest {
    network: String,
    name: String,
    proposals: Vec<u64>,
}

#[derive(Debug, Serialize)]
struct ReplayReport {
    network: String,
    selection: String,
    root: String,
    success_count: usize,
    failure_count: usize,
    elapsed_ms: u64,
    results: Vec<ProposalReplayResult>,
}

#[derive(Debug, Serialize)]
struct ProposalReplayResult {
    proposal_id: u64,
    path: String,
    passed: bool,
    block_start: Option<u64>,
    block_end: Option<u64>,
    block_count: usize,
    input_hash: Option<String>,
    validation_ms: Option<u64>,
    error: Option<String>,
}

pub(crate) fn run(root: &Path, args: ReplayGuestInputArgs) -> Result<()> {
    let guest_input_root = resolve_guest_input_root(root, &args.guest_input_root);
    let selection = selection_label(&args);
    let proposal_ids = selected_proposals(&guest_input_root, &args)?;
    ensure!(
        !proposal_ids.is_empty(),
        "no proposals selected for network {}",
        args.network
    );

    let started_at = Instant::now();
    let mut results = Vec::with_capacity(proposal_ids.len());
    for proposal_id in proposal_ids {
        let path = guest_input_proposal_path(&guest_input_root, &args.network, proposal_id)?;
        let result = replay_proposal(&path, proposal_id);
        print_result(&result);
        let failed = !result.passed;
        results.push(result);
        if failed && args.fail_fast {
            break;
        }
    }

    let success_count = results.iter().filter(|result| result.passed).count();
    let failure_count = results.len() - success_count;
    let report = ReplayReport {
        network: args.network,
        selection,
        root: guest_input_root.display().to_string(),
        success_count,
        failure_count,
        elapsed_ms: elapsed_ms(started_at),
        results,
    };

    if let Some(path) = &args.json_out {
        write_report(path, &report)?;
    }

    println!(
        "replay_guest_input: successes={} failures={} elapsed_ms={}",
        report.success_count, report.failure_count, report.elapsed_ms
    );

    if report.failure_count > 0 {
        bail!("{} guest input replay(s) failed", report.failure_count);
    }
    Ok(())
}

fn selected_proposals(root: &Path, args: &ReplayGuestInputArgs) -> Result<Vec<u64>> {
    let selected_count = usize::from(args.proposal.is_some())
        + usize::from(args.suite.is_some())
        + usize::from(args.all);
    ensure!(
        selected_count == 1,
        "select exactly one of --proposal, --suite, or --all"
    );

    if let Some(proposal_id) = args.proposal {
        return Ok(vec![proposal_id]);
    }

    if let Some(suite) = &args.suite {
        let path = guest_input_suite_path(root, &args.network, suite)?;
        let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let manifest: SuiteManifest =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        ensure!(
            manifest.network == args.network,
            "suite network mismatch: expected {}, got {}",
            args.network,
            manifest.network
        );
        ensure!(
            manifest.name == *suite,
            "suite name mismatch: expected {}, got {}",
            suite,
            manifest.name
        );
        validate_proposal_list(&manifest.proposals)?;
        return Ok(manifest.proposals);
    }

    let proposals_dir = guest_input_proposals_dir(root, &args.network)?;
    let mut proposals = Vec::new();
    for entry in
        fs::read_dir(&proposals_dir).with_context(|| format!("read {}", proposals_dir.display()))?
    {
        let path = entry
            .with_context(|| format!("read entry under {}", proposals_dir.display()))?
            .path();
        if let Some(proposal_id) = parse_guest_input_proposal_id(&path) {
            proposals.push(proposal_id);
        }
    }
    proposals.sort_unstable();
    validate_proposal_list(&proposals)?;
    Ok(proposals)
}

fn validate_proposal_list(proposals: &[u64]) -> Result<()> {
    ensure!(!proposals.is_empty(), "proposal list must not be empty");
    let unique = proposals.iter().copied().collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == proposals.len(),
        "proposal list must not contain duplicates"
    );
    Ok(())
}

fn replay_proposal(path: &Path, proposal_id: u64) -> ProposalReplayResult {
    let mut result = ProposalReplayResult {
        proposal_id,
        path: path.display().to_string(),
        passed: false,
        block_start: None,
        block_end: None,
        block_count: 0,
        input_hash: None,
        validation_ms: None,
        error: None,
    };

    let outcome = (|| -> Result<()> {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let mut input: GuestInput =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        ensure!(
            input.taiko.proposal_id == proposal_id,
            "proposal id mismatch: expected {}, got {}",
            proposal_id,
            input.taiko.proposal_id
        );

        result.block_count = input.witnesses.len();
        result.block_start = input
            .witnesses
            .first()
            .map(|witness| witness.block.header.number);
        result.block_end = input
            .witnesses
            .last()
            .map(|witness| witness.block.header.number);

        let native_carry = build_proof_carry_data_from_witness_spec(&input, ProofType::Native)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        if input.proof_carry_data != ProofCarryData::default()
            && input.proof_carry_data != native_carry
        {
            bail!("proof_carry_data does not match native proof carry data");
        }
        input.proof_carry_data = native_carry.clone();
        let expected_hash = hash_shasta_subproof_input(&native_carry);
        result.input_hash = Some(format!("{expected_hash:#x}"));

        let validation_started_at = Instant::now();
        validate_proposal_guest_input(&input).map_err(|err| anyhow::anyhow!("{err}"))?;
        result.validation_ms = Some(elapsed_ms(validation_started_at));

        Ok(())
    })();

    match outcome {
        Ok(()) => result.passed = true,
        Err(err) => result.error = Some(format!("{err:#}")),
    }

    result
}

fn print_result(result: &ProposalReplayResult) {
    if result.passed {
        println!(
            "PASS proposal={} blocks={} range={:?}..{:?} hash={} validation_ms={}",
            result.proposal_id,
            result.block_count,
            result.block_start,
            result.block_end,
            result.input_hash.as_deref().unwrap_or(""),
            result.validation_ms.unwrap_or_default()
        );
        return;
    }

    println!(
        "FAIL proposal={} path={} error={}",
        result.proposal_id,
        result.path,
        result.error.as_deref().unwrap_or("unknown error")
    );
}

fn write_report(path: &Path, report: &ReplayReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let contents = serde_json::to_string_pretty(report).context("serialize replay report")?;
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn resolve_guest_input_root(repo_root: &Path, root: &Path) -> PathBuf {
    if root.is_absolute() {
        root.to_path_buf()
    } else {
        repo_root.join(root)
    }
}

fn selection_label(args: &ReplayGuestInputArgs) -> String {
    if let Some(proposal_id) = args.proposal {
        return format!("proposal:{proposal_id}");
    }
    if let Some(suite) = &args.suite {
        return format!("suite:{suite}");
    }
    "all".to_string()
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::validate_proposal_list;

    #[test]
    fn proposal_lists_must_be_non_empty_and_unique() {
        assert!(validate_proposal_list(&[3, 5]).is_ok());
        assert!(validate_proposal_list(&[]).is_err());
        assert!(validate_proposal_list(&[3, 3]).is_err());
    }
}
