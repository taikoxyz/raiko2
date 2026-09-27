# Task 4 Implementation Report

## Scope

Implemented only the proposal integration-smoke plumbing. This checkpoint did not acquire proposal
data, execute either frozen smoke proposal, open the final corpus, publish artifacts, or change a
production table or configuration.

## Implementation

- Allowed `guest-launcher` proposal-stage SP1 gas estimation only for local execute mode without
  aggregation or an alternate ELF.
- Loaded the production SP1 proposal ELF and reused the canonical gas-estimator options.
- Preserved the canonical GuestInput identity and emitted public output, exit status, `proverGas`,
  primary metric, execution diagnostics, and exact chunk threshold/slot provenance.
- Made Python `run-proposal --proof-type sp1` explicitly request `gas-estimator` and fail closed on
  noncanonical proposal report provenance or join fields.
- Made `prepare-corpus` discovery pass `--l1-network hoodi` for `taiko_hoodi` and
  `--l1-network ethereum` for `taiko_mainnet`.
- Documented the post-review sequence for frozen integration-smoke proposals Hoodi `79852` and
  Mainnet `38261`, including the boundary that smoke cannot enter or tune the unopened final corpus.
- Review hardening now rejects every other smoke identity and every non-SP1 smoke execution, and
  persists purpose, network, proposal ID, fixture digest, and proposal workload identity in the raw
  JSONL row.
- Proposal gas-estimator reports now bind the actual production proposal ELF and running
  guest-launcher SHA-256 digests. Python independently hashes both supplied files, rejects wrong
  report hashes, and the launcher rejects `RAIKO2_GUEST_ELF_DIR` for this production-only path.
  The VK is not consumed by local execute/gas-estimator and therefore is not claimed as execution
  provenance.
- GuestInput JSON parsing and fixture hashing use one byte snapshot. Native trace and SP1 execute
  both consume a read-only staged copy of those verified bytes, so mutation of the original path
  cannot desynchronize persisted smoke provenance from execution. The launcher hashes the executing
  inode through `/proc/self/exe` on Linux, while Python binds pre-run hashes and rejects an ELF or
  launcher path that changes before report acceptance.

## TDD Evidence

The first focused Rust run failed to compile because the new proposal report finalizer did not yet
exist (`E0425`). The first focused Python run executed 66 tests and produced the four expected
failures: the launcher command lacked `--sp1-execution-engine`, the proposal provenance validator
did not exist, and Hoodi/Mainnet discovery lacked `--l1-network`.

After implementation and review hardening:

- `cargo test -p guest-launcher proposal_gas_estimator -- --nocapture`: 4 passed, 0 failed.
- `cargo test -p guest-launcher benchmark_report_records_sp1_execution_engine -- --nocapture`:
  1 passed, 0 failed; standard reports omit the estimator-only digest fields.
- Python 3.11 focused runner/manifest suite: 71 passed, 0 failed.

## Verification

- `cargo fmt --all`: passed.
- `git diff --check`: passed.
- Python 3.11 `py_compile` for the changed implementation and test modules: passed.
- `cargo test -p guest-launcher`: 34 passed and 2 pre-existing checked-in gas-baseline assertions
  failed. Both observed gas values were exactly `4197` above their frozen expectations:
  `1448066` versus `1443869`, and `1599511` versus `1595314`. The first failure reproduced alone
  and with `--test-threads=1`. It also reproduced with the same `1448066` result from a clean
  archived `f9d4e57e` parent tree using the same toolchain. This change does not modify the
  estimator implementation or either expected baseline.
- `cargo clippy -p guest-launcher -- -D warnings`: passed.
- Complete Python 3.11 opcode-gas unittest discovery: 506 passed, 1 skipped, 0 failed.

The final focused fix reran formatting, Rust, Python, bytecode compilation, and whitespace checks.
A redundant post-cleanup clippy rerun was stopped while rebuilding the dependency graph to preserve
disk; the prior Task 4 clippy pass remains recorded above, but no post-review clippy pass is claimed.

## Operational Boundary

No network acquisition or real proposal execution was performed. The reviewed follow-up checkpoint
must build release `preflight` and `guest-launcher`, seal and replay the estimator from the clean
revision, then execute only the two purpose-labelled integration-smoke rows. The final 60-row corpus
remains unopened and unpublished.
