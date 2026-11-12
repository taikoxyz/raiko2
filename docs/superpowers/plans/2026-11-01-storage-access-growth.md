# Storage-access growth implementation plan

> **For agentic workers:** Use subagent-driven-development, independent source review and native/
> actual verification. Execute the approved state/input-growth work without a new approval pause.

**Goal:** Build and measure26 fixed minimal-multiproof storage-read inputs under frozen modelc893.
**Architecture:** Reuse canonical ProgramStorageState and the existing assembler/native/quote/SP1
interfaces. Only a Python producer/tests are new; this task adds no execution API or builder.
**Tech Stack:** Python standard library with existing independent Ethereum keccak oracle, EVM.
**Spec:** `docs/superpowers/specs/2026-11-01-storage-access-growth-design.md`.

## Constraints and ownership

- Worker owns new `bin/zkgas/bench/cases/storage_access_growth.py` and
  `bin/zkgas/bench/tests/test_storage_access_growth.py` only. Root integrates a supplied usage
  fragment into README after the other composition author finishes its independent section.
- No old producer, Rust, pin, SDK, guest, model coefficient, discount, limit or old asset change.
- Candidate/model/program bindings frozen before new actuals; root samples sequentially under
 6GiB PSS/600seconds/192MiB output/16MiB log/minfree512MiB with unchanged executable identities.
- Every row retains both estimate surfaces, actual proposal PGU and both flags, including N0.

## Review focus

- Raw logical preimages and exactly targeted minimal multiproof, no irrelevant witness padding.
- Warm/cold expected access history and all concrete keys/values, not just result/checksum.
- Fixed64 body and PUSH widths; no count-dependent unrolling or array-building calldata.
- Existing one-block storage fixture and no transaction descriptor; unchanged pre/post storage.
- Wrong key/stride may preserve checksum and receipt but must fail independent operand oracle.

### Task1: producer and semantic tests

- [ ] Observe failing tests before new producer; implement
  `make_case(mode='present', state_size=8192, reads=1024)` and `prepare(output, model_id=MODEL)`.
- [ ] Emit24base+2warm65536 cases, own baselines/references, typed domains, no-overwrite,
  independent decoded slot/value/loop/checksum tests and focused native positive/negative probes.
- [ ] Preserve all old inputs. Self-review and freeze complete source diff for independent review;
  no commit/push before that review. Provide README usage as a task-owned temporary fragment.

### Task2: qualification, actual and retention

- [ ] Independent tester qualifies26 real native inputs/quotes, roots/proof targets and selected
  semantic negatives. Root samples once using frozen artifacts and existing workload CLI.
- [ ] Independent raw actual/resource audit rejoins all hashes, output, gas normalization, resource
  and MPT data, literal ratios and paired diagnostics. No PGU assigned to rejected/failing rows.
- [ ] Root writes research/PR record; retain data in a new immutable Atlas fixture after complete
  review/hash closure and actual remote download. Keep source260/284/296 pending/do-not-merge.
