# Shasta Driver Derivation Logic Drift

## Status

Open.

## Context

`raiko2` Shasta preflight must derive tx-list witness inputs with the same semantics as the
Taiko client driver:

- decode source manifest payloads
- validate manifest metadata against the rolling parent state
- default invalid or empty manifests before block derivation
- roll parent context across multi-source and multi-block proposals

The current `raiko2-protocol-shasta` helper path copies driver-aligned validation/defaulting logic
instead of directly reusing the Taiko client driver implementation. This works for now, but creates
a protocol drift risk: the host preflight witness path, guest derivation validation, and production
driver can diverge if one side changes.

## Why This Matters

An invalid source manifest can contain transactions that the real driver/guest derivation will
discard by defaulting the manifest to an anchor-only block. If host preflight decodes the raw
manifest and fetches witnesses for those discarded transactions, the witness RPC can fail or produce
inputs for the wrong statement.

## Follow-Up

Move the shared Shasta derivation preparation API into a common crate, preferably the Taiko client
protocol layer or another small dependency that does not pull in the full driver/engine stack.
`raiko2` should call that shared API instead of maintaining its own copy.

The shared API should cover:

- source payload decode from calldata/blob-backed data
- `prepare_segment_manifest` style validation/defaulting
- rolling parent metadata updates across sources
- tests shared with or mirrored from the driver

## Current Mitigation

Keep `raiko2` preflight on the driver-aligned helper path and add regression tests for invalid
manifest defaulting to anchor-only tx-list witness inputs.
