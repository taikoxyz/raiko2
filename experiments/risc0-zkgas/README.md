# RISC0 zkGas to Cycles Offline Experiment

This directory contains a finite, resumable collector and a standard-library-only model fitter for
the post-Unzen RISC0 proposal experiment. It executes the proposal guest locally; it never proves,
submits to Boundless, or changes production quoting.

## Prepare the pinned cohort

Build the two existing CLIs and select one proposal ELF. Record the source revision, RISC0 SDK
version, `execution_po2`, and the full proposal image ID before collecting either network:

```sh
cargo build --release -p preflight -p guest-launcher
cargo run --release -p xtask --features guest-tools -- guest-digests \
  --output /tmp/risc0-guest-digests.json
```

In that JSON's `digests` array, use the `digest` value from the entry whose `proof_system` is
`risc0`, `stage` is `proposal`, and `digest_source` is `image_id`. The collector also requires the
image ID computed by `guest-launcher` from the actual `--proposal-elf` to equal `--image-id`; a
mismatch is a terminal cohort failure.

Candidate selection happens before expensive execution. Supply one finite JSON manifest per
network. Hoodi entries use `fit` or `calibration`; Mainnet entries use `holdout`. The intended initial
cohort has 80 Hoodi fit, 40 Hoodi calibration, and 60 Mainnet holdout successes. Generate the lists
by scanning eligible spans and stratifying jointly by proposal zkGas and block count, as described in
the experiment design; do not use consecutive-tip-only lists.

```json
{
  "schema_version": 1,
  "network": "taiko_hoodi",
  "split_targets": {"fit": 80, "calibration": 40},
  "candidates": [
    {"proposal_id": 12345, "split": "fit", "stratum": "zkgas-q1-blocks-q3"},
    {"proposal_id": 12399, "split": "calibration", "stratum": "zkgas-q4-blocks-q2"}
  ]
}
```

For Mainnet, use `"split_targets": {"holdout": 60}`. Every split target is mandatory and positive,
their sum must equal `--target-count`, and the finite candidate list must contain at least that many
candidates for each split after `--max-candidates` is applied. The manifest may contain extra
candidates so terminal exclusions do not force a short cohort. Proposal IDs must be unique.

## Collect

Run Hoodi and Mainnet separately. Invoke the script with the same virtual-environment interpreter
that should be used by discovery; `/path/to/venv/bin/python` below is deliberately a placeholder.
RPC flags are optional when the repository defaults are suitable.

```sh
/path/to/venv/bin/python experiments/risc0-zkgas/risc0_zkgas.py collect \
  --network taiko_hoodi \
  --candidate-manifest /path/to/hoodi-candidates.json \
  --target-count 120 \
  --max-candidates 180 \
  --output-dir /tmp/raiko2-risc0-zkgas-cycles/hoodi \
  --source-revision "$(git rev-parse HEAD)" \
  --image-id 0xFULL_PROPOSAL_IMAGE_ID \
  --risc0-version 3.0.5 \
  --execution-po2 20 \
  --proposal-elf crates/guests/elf/risc0_shasta_proposal.elf \
  --preflight-bin target/release/preflight \
  --guest-launcher-bin target/release/guest-launcher

/path/to/venv/bin/python experiments/risc0-zkgas/risc0_zkgas.py collect \
  --network taiko_mainnet \
  --candidate-manifest /path/to/mainnet-candidates.json \
  --target-count 60 \
  --max-candidates 100 \
  --output-dir /tmp/raiko2-risc0-zkgas-cycles/mainnet \
  --source-revision "$(git rev-parse HEAD)" \
  --image-id 0xFULL_PROPOSAL_IMAGE_ID \
  --risc0-version 3.0.5 \
  --execution-po2 20 \
  --proposal-elf crates/guests/elf/risc0_shasta_proposal.elf \
  --preflight-bin target/release/preflight \
  --guest-launcher-bin target/release/guest-launcher
```

Add `--l1-rpc`, `--l2-rpc`, or `--chain-spec-list` when needed. One resolved chain-spec file is
passed to both discovery and preflight. Before creating output, the collector requires that JSON
array to contain exactly one entry named for the selected L2 network and exactly one entry named for
its pinned L1 network (`hoodi` for `taiko_hoodi`, or `ethereum` for `taiko_mainnet`); missing or
duplicate relevant entries are fatal. A run is pinned by `run-manifest.json`; changing any cohort
setting requires a new output directory. The collector verifies `--source-revision` against the
repository HEAD and `--risc0-version` against `Cargo.lock`, then hashes the collector, proposal ELF,
both binaries, discovery script, lockfile, and resolved chain spec into the manifest and every result
row. Rebuilding an artifact therefore cannot silently reuse a cohort under unchanged labels.

`samples.jsonl` is append-only and fsynced, while `progress.json` is replaced atomically. Discovery
and preflight publish validated JSON caches through a same-directory atomic rename, so timeout
partials are discarded. Successful and terminal rows are skipped on resume; retryable
infrastructure or malformed-cache failures are attempted again. Reaching every split quota returns
0. Exhausting the finite manifest returns 3 and records total and per-split shortfalls.

If a crash tears only the final `samples.jsonl` record before its newline, collector resume truncates
that malformed tail to the previous complete newline, fsyncs the repair, and reports it on stderr.
If the final JSON object is complete but only its trailing newline is missing, resume preserves the
row and durably appends the newline before any later result append.
Malformed interior records and malformed newline-terminated records remain fatal. The `fit` command
always reads JSONL strictly and never repairs input.

Eligibility and features are derived from the exact persisted `GuestInput`. Its compact manifest
chain spec must identify the expected Taiko network by `name`, `chain_id`, and `is_taiko`; every
included block must be at or after the pinned network Unzen timestamp and have nonzero `difficulty`.
For built-in/default chain specs the experiment design timestamps are pinned directly. If a custom
chain-spec list explicitly provides `hard_forks.UNZEN.Timestamp`, it must equal the pinned design
timestamp. The resolved timestamp is recorded in the run manifest and every sample. The authoritative
M3 size feature is `risc0_input_bytes`, the RISC0 bincode frame length reported by
`guest-launcher`. `guest_input_json_bytes` is retained only as a cheap diagnostic and is never used
as M3's input-size feature.

## Fit and report

Pass both independently collected JSONL files. The fitter fixes M1/M2/M3 coefficients and all
one-sided margins from Hoodi before evaluating the untouched Mainnet holdout.

```sh
/path/to/venv/bin/python experiments/risc0-zkgas/risc0_zkgas.py fit \
  --samples /tmp/raiko2-risc0-zkgas-cycles/hoodi/samples.jsonl \
  --samples /tmp/raiko2-risc0-zkgas-cycles/mainnet/samples.jsonl \
  --model-out /tmp/raiko2-risc0-zkgas-cycles/model.json \
  --report-out /tmp/raiko2-risc0-zkgas-cycles/report.md
```

The JSON artifact records all candidate coefficients, Hoodi largest-positive-residual margins, the
selected model, feature envelope, 1,000-mcycle bucket policy with a 2,000-mcycle minimum, Mainnet
holdout gates, and an explicit shadow-mode recommendation. Exit 0 means every selected-model gate
passed; exit 4 means retain local pre-execution. Passing is evidence for a later shadow experiment,
not permission to copy a coefficient directly into production.

## Unit tests

Tests use fixtures and mocked subprocesses; they do not contact RPC endpoints or run RISC0:

```sh
PYTHONDONTWRITEBYTECODE=1 /path/to/venv/bin/python -m unittest discover \
  -s experiments/risc0-zkgas/tests -v
```
