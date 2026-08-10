# Boundless Local Funding Design

Status: Approved for implementation after local review.

## Context

PR #218 added account-level serialization, explicit nonce recovery, confirmed receipt handling, and
funding protection for concurrent Boundless on-chain requests. Those controls address real races and
must remain.

It also made every on-chain submission fetch the requestor's complete history from a hard-coded
CloudFront indexer endpoint before querying the market balance. That request currently fails with
HTTP 403 and makes proving depend on an external historical scan whose cost grows with the lifetime
of the requestor account.

Task recovery and funding are separate concerns. Runtime checkpoints already carry the Boundless
request ID, exact max price, deadlines, attempt, deployment, and image reference. Funding only needs
the current market balance plus requests that this process submitted and that may still consume that
balance.

Mainnet uses one active Raiko2 writer per Boundless signer and three queue workers. If process state
is lost, repeating at most those three proof tasks is acceptable. Exact cross-restart reconstruction
of funding reservations is not required.

## Goals

1. Remove requestor-history indexer access from Boundless funding and submission.
2. Preserve in-process protection against underfunding submitted-but-not-yet-locked requests.
3. Keep PR #218's account semaphore, checkpoint-before-broadcast rule, nonce recovery, uncertain-send
   handling, and confirmed receipt behavior.
4. Query `BoundlessMarket.balanceOf(requestor)` once per serialized submission.
5. Prevent a GCS-only Boundless client from initializing AWS/S3 credential discovery.

## Non-Goals

- No distributed signer coordination, new runtime schema, or persistent account ledger.
- No historical request reconstruction after restart.
- No HTTP API, proof format, GuestInput, public input, or on-chain verifier changes.
- No removal of the deployment indexer used by Boundless SDK market-pricing mode.

## Funding Model

`BoundlessFundingState` remains shared by every configured pair using the same signer. Reservations
are keyed by market `request_id`; rebid digests under the same ID contribute only their maximum active
`max_price` because one request ID cannot produce multiple payable fulfillments.

For each serialized submission:

```text
required_total = sum(max_price_by_active_request_id)
market_balance = BoundlessMarket.balanceOf(requestor)
attached_value = required_total.saturating_sub(market_balance)
```

The balance is read through the configured Boundless Ethereum RPC. It does not use the CloudFront
requestor-history endpoint.

### Required Ordering Invariant

The on-chain path must execute in this order:

1. Persist the request identity checkpoint.
2. Acquire the account submission permit and recover any uncertain predecessor.
3. Query market balance and account nonces.
4. Under the short funding-state lock, prune expired reservations, calculate the top-up including the
   current request, allocate the explicit nonce, and atomically record both the current reservation
   and uncertain submission.
5. Release the state lock.
6. Invoke `call.send()`.

The current request must exist in the local ledger before `call.send()`. Once step 4 completes,
cancellation, send timeout, or an ambiguous RPC error must not remove its reservation.

### Reservation Lifecycle

- A confirmed submit transaction revert removes only the matching request digest.
- Successful, timed-out, and ambiguous submissions remain reserved.
- Other rebid digests under the same request ID remain active.
- Reservations are pruned only after `lock_expires_at` plus a 60-second local-clock/chain-time
  safety margin.

Keeping a reservation after an earlier lock or fulfillment can temporarily over-deposit, but cannot
underfund the account. Network I/O must never occur while holding the funding-state mutex.

## Restart Behavior

The local funding ledger starts empty after restart and is not reconstructed from the external
indexer.

- A complete runtime checkpoint resumes its existing market request ID.
- A missing checkpoint is handled by the existing client/runtime retry path rebuilding the task.
- An unknown old request may consume balance before a rebuilt request locks. The rebuilt request may
  remain unlocked temporarily; existing no-lock timeout, rebid, and client retry behavior eventually
  observes the lower balance and tops it up.
- This is an accepted liveness and proving-cost tradeoff. Three workers is an operational bound, not
  a value to hard-code in funding logic.

This failure mode can delay market locking or repeat proving expense, but it cannot produce an
invalid proof.

## Indexer Boundary

Remove the custom funding-only requestor-history client, response types, pagination, snapshot,
`indexer_caught_up`, terminal-block alignment, and their tests.

Keep `Deployment.indexer_url` and the Taiko deployment URL. Boundless SDK market pricing may consume
that endpoint. The removed dependency is specifically the custom funding request to
`v1/market/requestors/{requestor}/requests`.

## GCS And Optional S3 Support

Keep the S3 change in the same PR as a separate commit:

- Declare `boundless-market` with `default-features = false` and GCS enabled.
- Add a non-default `boundless-s3` feature enabling `boundless-market/s3`.
- Compile S3 imports, fields, configuration, and tests only with `boundless-s3`.
- A default build receiving an explicit `BOUNDLESS_STORAGE_UPLOADER=s3` must fail during startup
  with a clear configuration error explaining that the S3 feature is not compiled. Implicit
  selection prefers GCS, Pinata, and File over `S3_BUCKET`; an S3-only implicit configuration must
  also fail startup in a default build.
- A Boundless network route must fail startup if storage resolves to `none`, because both program
  and guest-input publication require an uploader.
- GCS, Pinata, file, and none retain their current behavior.

SP1 may still carry its own AWS signer dependency. Acceptance here is narrower: constructing and
using the Boundless GCS client must not initialize Boundless S3 code or emit AWS IMDS warnings from
that path.

## Observability

Emit one structured `info` event per funding decision containing no secrets:

- market request ID;
- locally reserved request-ID count;
- market balance;
- required total;
- attached top-up value.

Existing request ID, nonce recovery, receipt, timeout, and rebid logs remain unchanged.

## Verification

Funding and submission tests must cover:

- multiple request IDs sum their maximum prices;
- rebid digests under one ID use only the maximum price;
- confirmed revert removes only its matching digest;
- ambiguous submission remains reserved;
- expired reservations are pruned;
- top-up is zero when balance covers required total and equals the shortfall otherwise;
- the current request is recorded before `call.send()`;
- cancellation or timeout after reservation does not release it;
- funding makes no requestor-history HTTP call;
- existing nonce recovery, serialization, and three-confirmation tests still pass.

Feature verification must show that the default dependency graph does not enable Boundless S3, the
default GCS path does not construct an S3 downloader, a default S3 configuration fails clearly, and a
`boundless-s3` build compiles.

Run:

```bash
cargo fmt --all -- --check
cargo test -p raiko2-prover boundless_ --lib
cargo test -p raiko2-prover
cargo clippy -p raiko2-prover --all-targets -- -D warnings
cargo check -p raiko2-prover --no-default-features \
  --features "chain-spec-json,boundless,boundless-s3"
cargo clippy --workspace -- -D warnings
```

Canary acceptance requires one GCS-backed Boundless proposal to produce a provider request ID and
enter polling without a requestor-history 403 or Boundless S3 AWS IMDS warning.

## Rollout

The current GCS deployment requires no config migration. Roll out to canary first. Rollback uses the
previous host image; no stored artifact or on-chain format changes are introduced.
