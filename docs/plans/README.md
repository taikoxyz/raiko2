# Plan Tracking

`docs/plans` is intentionally tracked. Do not add it to `.gitignore`.

Use this directory for design notes, runbooks, and implementation plans that should survive handoff.
Keep transient command output, logs, generated calldata, quote dumps, and large experiment artifacts
outside this directory.

## Status Values

Every plan should include a `## Status` section near the top with one of these values:

- `Draft for discussion`: proposal is not accepted yet.
- `Accepted`: direction is approved, but implementation is not complete.
- `In progress`: implementation or validation is actively happening.
- `Implemented`: the plan is complete and points to the landed code or release.
- `Superseded`: another plan or implementation replaced this plan.
- `Archived`: retained for history only.

## Hygiene Rules

- Use repository-relative links, not absolute local paths.
- Do not commit machine-local paths such as `/home/...`, `/Users/...`, or ad hoc `/tmp/...` paths.
- Do not commit private keys, API tokens, bearer tokens, RPC credentials, or cloud credentials.
- Do not commit raw run logs; use concise evidence snippets in the plan or link to tracked test output.

## Current Drafts Added For Open Prover Work

| File | Status | Notes |
| --- | --- | --- |
| `2026-05-13-tdx-prover-intake-design.md` | Draft for discussion | External prover acceptance criteria. |
| `2026-05-24-raiko2-open-prover-platform-design.md` | Draft for discussion | Program overview and workstream map. |
| `2026-05-25-raiko2-benchmark-framework-design.md` | Draft for discussion | Benchmark taxonomy and reporting shape. |
| `2026-05-25-raiko2-provider-registry-and-proof-envelope-design.md` | Draft for discussion | Provider identity and proof envelope direction. |
| `2026-05-25-raiko2-security-invariants-and-mutation-suite-design.md` | Draft for discussion | Invariant and mutation-test direction. |
| `2026-05-26-open-prover-platform-landscape-to-investigate.md` | Draft research note | External prover-market landscape notes. |
| `2026-05-26-prediction-market-friendly-chain-to-investigate.md` | Draft research note | Early feasibility notes for prediction-market chain needs. |
| `2026-05-27-tdx-remote-provider-image-identity-design.md` | Draft for discussion | TDX provider identity and image-attestation model. |
| `2026-06-09-tdx-gce-smoke-runbook.md` | Draft runbook | First GCE TDX validation runbook. |

## Current ZKGas Calibration Documents

| File | Status | Responsibility |
| --- | --- | --- |
| `2026-09-26-zkgas-calibration-design.md` | Accepted | Authoritative architecture, ownership boundaries, units, and experiment invariants. |
| `2026-09-26-zkgas-calibration-execution-plan.md` | In progress | Current Osaka operation-supplement tasks and gates. |
| `2026-09-26-zkgas-calibration-progress.md` | In progress | Verified evidence, active milestone, next gate, and explicit non-goals. |
| `2026-09-27-zkgas-composite-block-estimator-implementation-plan.md` | In progress | Composite estimator, typed traces, sealing, and real block validation. |
| `2026-10-02-zkgas-anchor-operation-correction-implementation-plan.md` | In progress | Correct Anchor operation ownership, calibrate BLOCKHASH, and refit block costs. |

The earlier `2026-09-06-zkgas-multiplier-recalibration-experiment-design.md` and
`2026-06-08-sp1-opcode-prover-gas-experiment-implementation-plan.md` are superseded historical
documents, not current execution instructions. The September 22 relative-opcode and September 25
hierarchical spec/plan files under `docs/superpowers/` carry the same explicit historical status.
