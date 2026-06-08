# SP1 Prover Gas Research Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend the existing SP1 execute-only benchmark path so local prover-gas research can run
one or more cached `GuestInput` files and produce a stable aggregate JSON report.

**Architecture:** Keep `guest-launcher` as the process that executes SP1 guests and writes per-run
JSON. Extend `xtask bench-guest` to preserve the full execute metadata, summarize prover gas and
execution counters, and optionally drive multiple existing `GuestInput` files from a JSON suite
manifest. Avoid hosted API and network prover changes.

**Tech Stack:** Rust 2024, Clap, Serde JSON, SP1 SDK `ProverClient::execute()`, existing
`guest-launcher`, existing `xtask bench-guest`

---

### Task 1: Preserve SP1 execute metadata in `bench-guest` reports

**Files:**
- Modify: `xtask/src/bench_guest.rs`

**Step 1: Write the failing test**

Add a `#[cfg(test)] mod tests` to `xtask/src/bench_guest.rs` if one does not exist. Add a test that
parses a representative `guest-launcher` JSON payload containing `gas`, `exit_code`,
`total_instruction_count`, `total_syscall_count`, `touched_memory_addresses`,
`invocation_tracker`, `opcode_counts`, `syscall_counts`, and `memory_snapshots`.

The test should assert the parsed `LauncherReport` retains all fields:

```rust
#[test]
fn launcher_report_preserves_sp1_execute_metadata() {
    let raw = r#"{
      "stage": "proposal",
      "mode": "execute",
      "proof_mode": "compressed",
      "input": "input.json",
      "public_values": "0x1234",
      "wall_time_ms": 42,
      "exit_code": 0,
      "gas": 123456,
      "total_instruction_count": 200,
      "total_syscall_count": 3,
      "touched_memory_addresses": 99,
      "cycle_tracker": [{"label": "block", "cycles": 10}],
      "invocation_tracker": [{"label": "block", "count": 1}],
      "opcode_counts": [{"label": "ADD", "count": 2}],
      "syscall_counts": [{"label": "COMMIT", "count": 1}],
      "memory_snapshots": [{"label": "proposal:start", "rss_kb": 10, "hwm_kb": 12}]
    }"#;

    let report: LauncherReport = serde_json::from_str(raw).unwrap();

    assert_eq!(report.gas, Some(123456));
    assert_eq!(report.exit_code, Some(0));
    assert_eq!(report.total_instruction_count, Some(200));
    assert_eq!(report.total_syscall_count, Some(3));
    assert_eq!(report.touched_memory_addresses, Some(99));
    assert_eq!(report.invocation_tracker[0].label, "block");
    assert_eq!(report.opcode_counts[0].label, "ADD");
    assert_eq!(report.syscall_counts[0].label, "COMMIT");
    assert_eq!(report.memory_snapshots[0].label, "proposal:start");
}
```

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p xtask launcher_report_preserves_sp1_execute_metadata
```

Expected: FAIL because `LauncherReport` does not yet define the execution metadata fields.

**Step 3: Implement the metadata fields**

Extend `LauncherReport` with:

```rust
exit_code: Option<u64>,
gas: Option<u64>,
total_instruction_count: Option<u64>,
total_syscall_count: Option<u64>,
touched_memory_addresses: Option<u64>,
invocation_tracker: Vec<LauncherCountEntry>,
opcode_counts: Vec<LauncherCountEntry>,
syscall_counts: Vec<LauncherCountEntry>,
memory_snapshots: Vec<LauncherMemoryEntry>,
```

Add `LauncherCountEntry` and `LauncherMemoryEntry` structs that mirror `guest-launcher` JSON.

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p xtask launcher_report_preserves_sp1_execute_metadata
```

Expected: PASS.

### Task 2: Summarize prover gas and execution counters

**Files:**
- Modify: `xtask/src/bench_guest.rs`

**Step 1: Write the failing test**

Add a test that builds two `LauncherReport` values and calls `summarize(&reports)`. Assert summary
stats include `prover_gas`, `total_instruction_count`, `total_syscall_count`, and
`touched_memory_addresses`.

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p xtask bench_summary_includes_prover_gas_and_execution_counters
```

Expected: FAIL because `BenchSummary` currently has no fields for these stats.

**Step 3: Implement summary fields**

Extend `BenchSummary` with:

```rust
prover_gas: Option<Stats>,
total_instruction_count: Option<Stats>,
total_syscall_count: Option<Stats>,
touched_memory_addresses: Option<Stats>,
```

Populate them from `LauncherReport` optional fields. Update `print_summary` to print
`prover_gas` when present.

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p xtask bench_summary_includes_prover_gas_and_execution_counters
```

Expected: PASS.

### Task 3: Add suite manifest support for existing GuestInput files

**Files:**
- Modify: `xtask/src/bench_guest.rs`

**Step 1: Write the failing manifest-parse test**

Add a test for this JSON shape:

```json
{
  "cases": [
    {
      "name": "mainnet-proposal-2222",
      "input": "./tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_2222_l2_5412225_5412416.json",
      "proof_type": "sp1"
    }
  ]
}
```

Assert it parses into a suite with one case, preserving `name`, `input`, and `proof_type`.

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p xtask bench_suite_manifest_parses_guest_input_cases
```

Expected: FAIL because suite types do not exist.

**Step 3: Implement manifest types and CLI flag**

Add:

```rust
#[arg(long)]
pub(crate) suite: Option<PathBuf>,

#[derive(Debug, Deserialize)]
struct BenchSuiteManifest {
    cases: Vec<BenchSuiteCase>,
}

#[derive(Debug, Deserialize)]
struct BenchSuiteCase {
    name: String,
    input: PathBuf,
    #[serde(default)]
    proof_type: Option<String>,
}
```

Add `read_suite_manifest(path: &Path) -> Result<BenchSuiteManifest>` with validation:

- suite must contain at least one case
- case name must not be empty
- case input must exist
- case proof type defaults to global `--proof-type`

Reject `--suite` together with preflight-only arguments that imply generating one input:
`--network`, `--l1-network`, `--rpc-url`, `--l1-rpc-url`, `--proposal-id`,
`--l1-inclusion-block-number`, `--last-anchor-block-number`, `--l2-start`, `--l2-end`,
`--l2-chain-id`, `--output`, and `--force-preflight`.

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p xtask bench_suite_manifest_parses_guest_input_cases
```

Expected: PASS.

### Task 4: Run suite cases and emit grouped reports

**Files:**
- Modify: `xtask/src/bench_guest.rs`

**Step 1: Write the failing grouping test**

Add a test around a pure helper, for example:

```rust
fn build_suite_report(
    backend: String,
    mode: String,
    proof_mode: String,
    built_guest: bool,
    sp1_docker_tag: Option<String>,
    warmup: usize,
    cases: Vec<BenchCaseReport>,
) -> BenchGuestReport
```

The test should assert JSON serialization contains `cases[0].name`, `cases[0].runs[0].gas`, and
`cases[0].summary.prover_gas`.

**Step 2: Run test to verify it fails**

Run:

```bash
cargo test -p xtask bench_suite_report_groups_case_results
```

Expected: FAIL because grouped suite reports do not exist.

**Step 3: Implement grouped report structs and run flow**

Add:

```rust
struct BenchCaseReport {
    name: String,
    proof_type: String,
    input: String,
    repeat: usize,
    runs: Vec<LauncherReport>,
    summary: BenchSummary,
}
```

Change top-level `BenchGuestReport` to include:

```rust
cases: Vec<BenchCaseReport>,
```

For backward compatibility, single-input mode can emit one case named from the input file stem.

Update `run(...)`:

- if `--suite` is present, load cases and skip `prepare_input`
- otherwise keep the existing single-input behavior
- build guest once
- build `guest-launcher` once
- run each case with the global `--warmup` and `--repeat`
- write one JSON file containing all cases

**Step 4: Run test to verify it passes**

Run:

```bash
cargo test -p xtask bench_suite_report_groups_case_results
```

Expected: PASS.

### Task 5: Document the operator workflow

**Files:**
- Modify: `docs/development.md`
- Modify: `docs/API.md`
- Modify: `docs/plans/2026-06-08-sp1-prover-gas-research-design.md`

**Step 1: Update `docs/development.md`**

Document:

- single `GuestInput` prover-gas run
- suite manifest shape
- `--skip-build-guest` after a prior `build-guest sp1 --bench`
- report location and key fields

**Step 2: Update `docs/API.md`**

Tighten the runtime-semantics note so it explicitly says `proposals[].extra_data.sp1.gas` is SP1
prover gas from `ExecutionReport::gas()` when available.

**Step 3: Update the design doc status**

Add a short implementation note listing the final CLI shape.

### Task 6: Final verification

**Files:**
- Test: `xtask/src/bench_guest.rs`
- Test: `docs/development.md`
- Test: `docs/API.md`

**Step 1: Format**

Run:

```bash
cargo fmt --all
```

Expected: exit code `0`.

**Step 2: Focused tests**

Run:

```bash
cargo test -p xtask bench_guest
```

Expected: exit code `0`.

**Step 3: API execute metadata regression**

Run:

```bash
cargo test -p raiko2 sp1_execute_batch_returns_report_without_proof
```

Expected: exit code `0`.

**Step 4: Diff hygiene**

Run:

```bash
git diff --check
```

Expected: no output and exit code `0`.
