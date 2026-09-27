# Task 4 Review Package

## Task 4 Commits

- `fa222702` — `feat(zkgas): add proposal estimator smoke plumbing`
- `31121157` — `fix(zkgas): bind proposal smoke provenance`

The intervening `fb882680` commit is the separately owned composite-estimator replay fix and is not
part of Task 4. Review the exact Task 4 changes as:

```bash
git diff f9d4e57e..fa222702
git diff fb882680..31121157
```

## Final Behavior

- Proposal gas estimation is limited to local SP1 execute mode with the canonical threshold/slot
  settings and production proposal ELF. Proof, network, aggregation, explicit ELF, and
  `RAIKO2_GUEST_ELF_DIR` paths fail closed.
- The report records the exact proposal ELF and executing guest-launcher SHA-256. Linux hashes the
  executing inode via `/proc/self/exe`; Python hashes both files before execution, verifies their
  stability afterward, and rejects wrong report digests.
- Only Hoodi `79852` and Mainnet `38261` can be labelled `integration_smoke`. The prepared record and
  raw JSONL bind purpose, network, proposal ID, fixture digest, and workload identity; RISC0 cannot
  carry the smoke label.
- GuestInput parsing and hashing use one byte read. Native trace and SP1 execute consume the same
  read-only staged snapshot, closing original-path mutation races. The standalone launcher report
  retains the real staged path; normalized JSONL preserves the original GuestInput path.
- Standard/lab report schemas do not serialize the estimator-only digest fields when absent.
- Final-corpus acquisition/execution and the two real smoke executions remain outside this
  implementation task and were not run.

## Final Verification

- `python3.11 -m unittest experiments/opcode-gas/tests/test_runner.py experiments/opcode-gas/tests/test_run_manifest.py`:
  71 passed, 0 failed.
- `cargo test -p guest-launcher proposal_gas_estimator -- --nocapture`:
  4 passed, 0 failed.
- `cargo test -p guest-launcher benchmark_report_records_sp1_execution_engine -- --nocapture`:
  1 passed, 0 failed.
- `cargo fmt --all -- --check`: passed.
- Python 3.11 `py_compile` for the implementation and two changed test modules: passed.
- `git diff --check` / staged diff check: passed.

The original Task 4 implementation had a green focused clippy pass. A redundant clippy rerun after
the shared target cache was cleared was stopped while rebuilding dependencies to preserve disk; no
post-review clippy pass is claimed.

## TDD And Review Findings

RED tests first demonstrated the missing frozen identity/workload record, absent ELF/launcher
digest validation, environment override acceptance, and GuestInput path mutation race. Rust RED
also demonstrated the missing override guard/report fields and executing-inode path helper.

Independent review findings addressed:

- froze and persisted smoke identity;
- bound exact guest and host artifacts;
- rejected non-SP1 smoke labels and kept standard schema compatibility;
- made shared gas-estimator diagnostics caller-neutral;
- closed GuestInput and launcher-path TOCTOU races;
- preserved the original normalized JSONL input path while executing the staged snapshot.
