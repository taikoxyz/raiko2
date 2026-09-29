# Task 5 Report: Seal and Replay the Production Context Result

Status: **DONE_WITH_CONCERNS**

## Outcome

Implemented create-only canonical sealing and directory-only portable verification for the frozen
production-context campaign. The result directory contains exactly these ten files:

- `result.json`
- `campaign-manifest.json`
- `calibration-identity.json`
- `rows.jsonl`
- `campaign-decisions.json`
- `model-report.json`
- `source-coverage-v5.json`
- `source-higher-layer.json`
- `source-discovery.json`
- `source-code-sha256s.json`

`rows.jsonl` is canonical, uncompressed JSONL with a 2 MiB per-line limit and a 256 MiB total
limit. Every other member has a frozen per-file limit. Reads reject missing or extra members,
symlinks, FIFOs and other non-regular files, oversize members, duplicate JSON fields, binary
floats, non-canonical encodings, path traversal, and changed-during-read files.

The verifier takes every numerical input from the sealed flat directory. It additionally requires
the current Git HEAD to be the recorded clean implementation revision and recomputes every frozen
source-code digest from the actual bounded, no-follow regular file. It reconstructs and validates
every fixture, row, and raw evidence identity; reconstructs the V5 subtotal model from the embedded
pinned coverage/registry and higher-layer bundles (including common event dispatch); refits all
exact matrices, coefficients, predictions, gates, and model choices; preserves accepted, partial,
and rejected family outcomes; and recomputes source, file, result, and directory identities without
SP1 execution.

Publication writes mode-0444 members to a private same-directory temporary directory, fsyncs the
files and directory, and publishes with `renameat2(RENAME_NOREPLACE)`. It fails closed when that
primitive is unavailable, rejects an invalid/symlink publication lock, and removes the temporary
directory after failure. Source and result paths may not overlap.

## Scope Ruling

The Task 5 brief omitted `experiments/opcode-gas/opcode_gas.py` even though the implementation plan
requires live seal/verify CLI dispatch. The root owner explicitly authorized the minimal expansion:
add only the live handlers and parser registration that delegate to the Task 5 implementation.
README and CLI coverage were updated only for commands that work at this commit; no duplicate seal
or replay logic was added to the CLI.

The root owner also resolved the portable-source inventory ambiguity. The exact ten-file inventory
is fixed, while the three source files are strict versioned bundles: V5 coverage embeds the exact
coverage payload and referenced registry artifact, higher-layer embeds the exact model payload and
directory identity, and discovery embeds the exact pinned result. Their internal hashes and content
identities are recomputed and joined to the immutable source pins in the canonical Task 1 manifest.

## TDD Evidence

Initial RED, before production code:

```text
~/.venv/bin/python -m pytest experiments/opcode-gas/tests/test_context_production.py -q -k 'ProductionContextSealTests'
2 failed, 39 deselected in 0.67s
AttributeError: module 'context_production_campaign' has no attribute 'seal_production_context_result'
```

Focused and regression coverage includes create-only duplicate publication, injected atomic-write
failure and cleanup, exact inventory, missing/extra/symlink/FIFO/oversize members, source/result
overlap, caller-resealed source forgery, discovery promotion, row deletion/reorder/duplication,
row/source path traversal, model/gate tampering, and wrong revision. The end-to-end replay test
renames the source run away and makes every subprocess call fail if invoked.

## Verification Evidence

- Non-artifact suite: `38 passed, 5 deselected, 73 subtests passed in 1.08s`.
- Unpatched local source check plus atomic cleanup: `2 passed, 41 deselected in 42.95s`.
- Final narrow real-source seal/replay, with only
  `context_opcode_campaign._validated_context_identity_helper` returning the already pinned sealed
  helper path: `1 passed, 42 deselected in 117.26s (0:01:57)`. The full discovery archive/hash
  graph, coverage/registry, higher-layer, manifest, sealing, and directory-only replay remained real.
- Final complete test file:
  `2 failed, 41 passed, 88 subtests passed in 400.25s (0:06:40)`.
- `~/.venv/bin/python -m py_compile experiments/opcode-gas/context_production_campaign.py experiments/opcode-gas/opcode_gas.py experiments/opcode-gas/tests/test_context_production.py`: passed.
- `git diff --check`: passed.
- No real campaign or proposal run was performed.

The two complete-file failures are the frozen local limitation and are unrelated to Task 5:

- `ProductionContextManifestTests::test_sources_bind_exact_sealed_artifacts_and_execution_surfaces`
- `ProductionContextCliTests::test_generate_is_create_only_and_validate_prints_identity`

Both fail at `context_opcode_campaign.py:4484` with
`ValueError: context native identity helper differs from sealed calibration`. Production validation
remains strict; the sealed launcher identity and evidence were not changed or rebuilt. A dedicated
test asserts that unpatched production sealing fails for this mismatch, while source-content
tampering still fails under the narrow test shim.

## Review Notes

The complete Task 5 diff and all new file/path operations were inspected. A final review found and
fixed two issues before commit: source-run/source-artifact reads inherited unbounded older helpers,
and the publication lock could follow a symlink. Those paths now use bounded `O_NOFOLLOW` regular
file reads/hashing and a checked `O_NOFOLLOW` lock. The post-fix narrow and complete test results are
the final results recorded above.

## Review Round 1

Review found that portable verification accepted the persisted source-code digest strings without
joining them to the actual implementation checkout, and that same-size in-place mutation could
escape the bounded readers' original device/inode/size comparison. The follow-up fix requires the
existing clean exact-HEAD policy, binds the source document revision to the calibration identity,
recomputes the exact frozen source map, and compares every digest. Both bounded helpers now compare
device, inode, full mode/type, link count, size, nanosecond mtime, and nanosecond ctime before and
after reading.

RED evidence:

- Both deterministic first-chunk same-size mutation cases returned without error: `2 failed`.
- A caller-resealed digest-only forgery and jointly resealed `ffff...` calibration/source/result
  forgery both verified: `2 failed, 15 subtests passed in 169.70s`.

Focused GREEN evidence:

- Same-size read/hash mutation, unsafe/unavailable source paths, and exact clean-revision policy:
  `3 passed, 2 subtests passed in 0.23s`.
- End-to-end inventory/tamper suite, including digest-only, missing-key, extra-key, and jointly
  resealed `ffff...` forgeries: `1 passed, 19 subtests passed in 361.48s`.
- Narrow complete-source seal and source-checkout replay: `1 passed, 44 deselected in 202.74s`.
- Non-artifact suite: `40 passed, 5 deselected, 75 subtests passed in 1.09s`.
- Complete test file: `2 failed, 43 passed, 94 subtests passed in 595.43s`. The only failures are
  the same two documented local sealed-helper fingerprint mismatches at
  `context_opcode_campaign.py:4484`; the production check remains strict.
