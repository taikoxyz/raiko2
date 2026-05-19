# Raiko2 SGX Provider Logging Design

## Problem

The dedicated `raiko2-sgx` provider currently exposes almost no application logs in normal Docker
usage. `docker logs` shows Gramine and AESM output, but not:

- startup intent
- the bound listening address
- prove or aggregate request activity
- request failures

This leaves the provider materially harder to operate than the main `raiko2` host and the external
`gaiko2-sgxgeth` service.

## Goal

Add minimal structured logs to the `raiko2-sgx-prover` binary so operators can confirm:

1. what configuration the provider started with
2. when it is actually listening
3. whether proposal and aggregate requests succeed or fail

## Non-Goals

This task does not:

- add metrics
- add request IDs or distributed tracing
- change proof semantics or HTTP schemas
- modify `gaiko2`

## Approach

Use the same observability shape already adopted by the main `raiko2` host:

1. initialize `tracing_subscriber` in `bin/raiko2-sgx-prover`
2. build a small startup summary from safe runtime fields
3. emit a startup summary before binding
4. emit an explicit listening log after bind succeeds
5. emit lightweight success/failure logs in the proposal and aggregate handlers

The startup summary should stay intentionally small and safe:

- `mode`
- `listen`
- `fork`
- `instance_id`
- `config_dir`
- `secret_dir`

No bootstrap payloads, keys, quotes, or full config dumps should be logged.

## Request Logging

Proposal requests should log:

- `schema`
- `chain_id`
- `block_count`
- `instance_id`
- success or failure

Aggregate requests should log:

- `schema`
- `proof_count`
- `instance_id`
- success or failure

There should be no separate hot-path "request received" log line. The provider should emit only:

- startup summary
- listening confirmation
- one success line per completed request
- one failure line per failed request

Failures should log the response classification already used by the protocol layer:

- `INVALID_JSON`
- `INVALID_REQUEST`
- `PROVER_ERROR`

## Testing

Add focused tests for the new startup summary helper so the log-safe startup surface stays pinned.

Verification should include:

- `cargo test -p raiko2-sgx-runtime --lib`
- `cargo test -p raiko2-sgx-prover -- --nocapture`
- `cargo fmt --all --check`

The runtime behavior itself should be smoke-checked by running the provider and verifying that
`docker logs` now contains application startup lines in addition to Gramine output.
