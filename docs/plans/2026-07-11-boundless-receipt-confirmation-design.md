# Boundless Onchain Delivery Confirmation Design

## Context

PR #164 treats an acknowledged Boundless submission at the configured price ceiling as the final
bid rung. On the onchain path, that acknowledgement must prove that the exact request was published
while a prover could still obtain a payable lock. The current implementation only checks selected
event topics and compares the lock deadline with host wall time, so malformed event data or a host
clock trailing the market chain can incorrectly finalize an undeliverable rung.

## Goals

- Fail closed unless a receipt contains a fully ABI-decodable `RequestSubmitted` event emitted by
  the configured market for the exact request.
- Decide receipt timeliness using the receipt block's chain timestamp, not host wall time.
- Preserve the existing pre-dispatch snapshot acknowledgement, transaction-hash persistence,
  balance reservation, bounded receipt wait, and same-request-ID retry behavior.
- Keep all receipt, log-decoding, and block-lookup uncertainty on the unconfirmed path.

## Design

### Typed event validation

Filter receipt logs by the configured market address and the `RequestSubmitted` signature, then use
Alloy's generated event decoder rather than manually comparing topics. Confirmation requires one
valid decoded event whose indexed `requestId` and decoded request body's `id` both equal the
finalized submission ID. Missing, duplicate, malformed, wrong-emitter, or mismatched events return
an unconfirmed verdict.

### Chain-derived inclusion time

After obtaining a successful receipt, load the receipt block through the same market-chain provider
using its block hash or number. Pass that block timestamp into the pure delivery-verdict helper and
require it to be strictly earlier than `lock_expires_at`. Missing receipt block metadata, block
lookup errors, or inclusion at/after the lock deadline are unconfirmed.

The existing host-time calculation may continue to cap how long the process waits for a receipt; it
is only an operational timeout. It must not authorize delivery. The chain timestamp is the sole
authority for the payable-window check.

### Error and recovery behavior

The transaction hash remains durably published before receipt waiting. Any unconfirmed verdict is
logged and returns the submission with `delivery_confirmed = false`, retaining ceiling-pinned
same-ID rebids. Crashes and cancellations remain conservative because delivery confirmation stays
in memory and the balance claim is released through RAII.

## Testing

Use test-driven development with two regression tests written and observed failing before production
changes:

1. A receipt log with the correct emitter and topics but malformed event data must not confirm.
   Positive fixtures must use a fully ABI-encoded `RequestSubmitted` event.
2. A receipt included after the chain lock deadline must remain unconfirmed even when a simulated
   host clock is still before that deadline.

Retain the existing negative cases for revert, missing event, wrong emitter, wrong request ID,
receipt timeout, and receipt lookup failure. Run focused prover tests where the local toolchain
allows, `cargo check -p raiko2-prover --tests`, formatting, diff checks, and the repository's
relevant CI lanes.

## Non-goals

- Changing price escalation, request-ID rotation, or persistence schemas.
- Replacing the custom balance gate with the Boundless SDK submission helper.
- Adding configurable confirmation depth; reorg hardening remains a separate follow-up.
