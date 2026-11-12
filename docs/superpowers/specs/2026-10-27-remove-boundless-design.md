# Remove Boundless from raiko2

Status: implemented against main `f1eb6afd8ed849ca90c08c3a1b44407a821bd005`. The user confirmed the release happens after Boundless shutdown; old database handling is external to the new raiko2.

## Goal and boundary

Remove the Boundless proving-market integration and the complexity that serves only it. Retain SP1 local/network, RISC0 local execution/proving and proposal/aggregation guests, native execution, and SGX/SGX-geth remote proving. Vendor-independent zkGas opcode/block calibration remains a separate concern.

The requested approach is full removal of the backend. A disabled feature leaving its implementation and dependencies behind would not meet that request. Removing RISC0 itself would go beyond it.

## SP1 behavior verified in current source

SP1 SDK 6.8.1 submits signed API/gRPC requests, uploads input artifacts, returns a request ID, and exposes status/result/cancellation APIs. The SDK has its own request nonce and signing; raiko2 does not manage an Ethereum transaction nonce, gas replacement ladder, receipt confirmations or chain reorg tracking for these requests.

SP1 is nevertheless asynchronous. Current raiko2 saves its request ID and attempt/deadline/configuration, polls status, resumes the same ID after restart, retries selected terminal failures within a bounded attempt count, and verifies the returned proposal/aggregate proof. Preserve that behavior and the supporting shared persistence/polling code.

## Delete

- Boundless prover and configuration modules, request/transaction broadcasting, nonce and replacement management, receipt/event/confirmation checks, uncertain-submission reconciliation, rebids and price escalation, lock/fulfill tracking, balance admission gates and Boundless storage upload/download.
- The RISC0 `network` route and `ShastaRisc0Network` pipeline, Boundless progress/checkpoint variants, observer branches, status/report fields and fixtures that exist solely for this backend.
- Boundless quote estimation model and exclusively dependent generation/check/calibration entrypoints, generated artifacts and dedicated tests. Preserve unrelated opcode/block calibration experiments and datasets.
- Boundless SDK/feature dependencies, S3 feature wiring, credentials/config/environment keys, per-network overrides, examples, CI/build flags and active operator/agent/skill instructions that would otherwise point to removed commands. Do not rewrite unrelated historical documentation merely to erase every historical word occurrence.
- Helpers that become unused. In particular, verify whether checkpoint-deadline helpers are Boundless-only before deleting or simplifying them; retain non-cancelling persistence guarantees needed by SP1 and runtime state.

## Preserve and verify

- Shared queue, worker task leases/renewal, cancellation/draining fences, GCS generation CAS/recovery, proof publication and generic remote polling/checkpoints. Lease renewal protects a long-running SP1 or other backend task too.
- SP1 request-ID recovery, bounded retry/cancellation behavior and both proposal/aggregation paths.
- Preserve the established SP1 checkpoint storage key for its request attempt. An internal Rust field rename must not change the retained SP1 storage format; this requires no Boundless compatibility reader or migration.
- RISC0 receipt/seal encoding and local proposal/aggregation. The dependency currently called `risc0-ethereum-contracts-boundless` is an alias for ordinary `risc0-ethereum-contracts`; rename the alias appropriately while keeping the required encode-seal functionality/version.
- Other users of Alloy/HTTP/storage libraries. Remove dependencies based on reachability, not their historical association with Boundless.
- No replacement backend, no new price/transaction framework, no changes to guest execution semantics and no live operations.

## Cutover decision

The removal is released only after Boundless is shut down. Old orders and databases are handled externally. The new raiko2 has no Boundless recovery compatibility, migration, cleanup job, startup hook or special database handler. Remove Boundless-specific stored-state types and branches directly. Retained SP1/runtime state still follows its existing persistence contracts.

Removed route/config values fail ordinary typed validation; they must not silently trigger local execution, SP1 submission or persistence deletion. Do not add a retired-Boundless compatibility layer or automatic cleanup.

## Verification

- Fresh complete dependency/feature/reference audit against current main; confirm Boundless market SDK is absent and retained routes still compile.
- Focused config/admission tests for the remaining routes and ordinary rejection of removed route/config values. No old-Boundless database migration tests or implementation.
- Retained SP1 helper, mock and runtime checkpoint tests: request-ID persistence/recovery, attempt monotonicity, retry without resubmission, transient polling errors, cancellation/draining and proposal/aggregate validation. Distinguish these from a full SDK gRPC submit/restart/fulfillment integration; report any untested external-service behavior explicitly.
- RISC0 local mock proposal/aggregation, native and SGX/SGX-geth protocol regression.
- Workspace formatting/clippy and affected CI test lanes. Follow repository guest/prover verification requirements for RISC0/SP1 builds and source/provenance changes; regenerate lockfiles through tooling and never hand-edit ELF assets.
- Independent adversarial review and behavioral verification before a ready PR. No real market submissions or paid proofs for validation.
