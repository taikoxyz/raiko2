# Startup Summary Logging Design

## Goal

Add a safe, route-aware startup summary for `raiko2` hosts so operators can tell what a pod intends
to run before the first request arrives.

## Scope

This design covers host startup logging only.

It includes:

- a generic startup summary log emitted after config load
- a `startup readiness passed` log emitted after startup checks succeed
- route-specific summary fields for `sgx/remote`

It does not include:

- request-time logging changes
- metrics changes
- full config dumping
- any change to proving behavior

## Problem

The current startup logs only show:

- that the process started
- the listening host/port

That is too weak for GKE operations. Operators need to know:

- which network pairs are configured
- which prover route the host is serving
- whether the host is wired to remote SGX providers
- when startup readiness has actually passed

At the same time, startup logs must not leak secrets or dump the full config.

## Decision

Add a small startup-summary builder that extracts safe fields from `Config` and the selected prover
route.

The host startup sequence will log three milestones:

1. `starting raiko2 host` with a structured startup summary
2. `startup readiness passed` with the same host identity fields
3. the existing `server listening on http://...` log

## Safe Summary Fields

The generic summary should include:

- `listen`
- `route`
- `pairs`
- `runtime_root`
- `queue_backend`
- `queue_workers`
- `json_logs`

For `sgx/remote`, it should additionally include:

- `remote_sgx_base_url` when configured
- `remote_sgx_sgxgeth_base_url` when configured

The summary must not include:

- private keys
- admin keys
- tokens
- signer keys
- raw config dumps

## Testing

Add focused tests that verify:

- the generic summary contains `listen`, `route`, and `pairs`
- the remote-SGX summary includes provider URLs
- secret-like fields do not appear in the summary payload

Then run a startup-log smoke test locally to confirm the log order remains:

- startup summary
- startup readiness passed
- server listening
