# Explicit-input SP1 gas estimator

This independent SDK6.3.0 host tool executes one caller-supplied opaque bincode buffer
against an explicit ELF. Build and ordinary measurement workflows are documented in
[the workload toolkit](../bench/README.md). The input has exactly one SP1 stdin buffer;
no guest, SDK, pricing or native-runner change is required.

## Optional historical register observations

Add `--observation-plan plan.json` for bounded host diagnostics. Without this option,
arguments, canonical options/input framing and the report shape remain unchanged.
An observed invocation adds only an `observation` object to the ordinary report.

Supply a strict JSON plan, replacing the digest and PC with values derived from the
actual ELF and its disassembly. This schema example is not an executable watch profile:

```json
{
  "schema_version": 1,
  "elf_sha256": "REPLACE_WITH_ACTUAL_ELF_SHA256",
  "instruction_limit": 1000000000,
  "watches": [{
    "label": "selected_load",
    "pc": "0x0000000000001000",
    "registers": [10, 11],
    "sample_limit": 64,
    "skip_hits": 0
  }]
}
```

Plans are regular files of at most64KiB. Fields are exact; unknown and duplicate fields
are rejected. `skip_hits` alone is optional and defaults to0. Labels are unique ASCII
identifiers matching `[A-Za-z_][A-Za-z0-9_]{0,63}`. PCs must be unique lowercase `0x`
plus16 hex digits, aligned executable instructions in the supplied Program. The ELF
SHA256 must match the actual file. Each watch selects distinct register indices0..31.
There must be1..64 watches; each sample limit is0..1024, with at most2048 samples across
the plan. Zero requests counts only. Instruction limit is1..1,000,000,000. Validation
and a conservative4MiB observation-output bound precede runner/resource creation.

Each watch counts every matching PC reached by diagnostic replay, including skipped
hits and hits beyond its sample cap. It retains the first requested matches after
`skip_hits`. Samples carry1-based hit and processed-instruction ordinals, historical
PC, before/after clocks, next PC, and before/after values in the declared register
order. All integer values use JSON numbers; consumers must preserve64-bit precision.
The plan hash and ELF identity bind the diagnostic configuration.

`complete`/`status` describe replay: `complete` requires an observed halt, empty trace
read cursors, and equal PC/clock/register values across successive chunks. An instruction
limit produces `complete:false,status:"instruction_limit"`; a replay failure produces
`status:"error"` and a bounded error. Both stop diagnostics while canonical estimation
continues where possible. Counts then cover only `processed_instructions`.
`samples_complete` additionally requires that all eligible post-skip hits encountered
were retained; a sample cap sets it false and marks that watch's `samples_truncated`.
Replay can complete while sample retention is truncated. An error or instruction limit
also sets `samples_complete:false`. Neither completion field establishes table coverage,
seed identity, map ownership or a pricing result.

The observer uses public typed `SplicingVM::execute_instruction()` after canonical
GasVM consumes each already-held immutable native chunk. It does not copy the raw trace,
retain full execution records, read arbitrary RAM, pause the native child, or rebuild a
guest. SDK touched-address/page and shape accounting lasts only for that chunk; no CoreVM
state is reset. External guards remain necessary for SDK allocation, replay time and total
process-tree PSS. Keep the canonical two-slot/134217728-memory-entry cadence and the existing
resource receipt/cleanup path. Diagnostic wall time includes extra host replay work and
is not prover/GPU time or the original canonical measurement. Record the new executable
identity and independently compare gas, instructions, public values, options and input/ELF
identities against frozen references before interpreting observations.
