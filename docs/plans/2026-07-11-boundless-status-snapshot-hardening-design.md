# Boundless Status and Snapshot Hardening Design

## Context

PR #164 makes request-ID rotation depend on a successful market read proving that the previous
request is no longer payable, and makes the previous exact bid part of the persisted resume state.
Three review gaps weaken those guarantees: batched `latest` reads are not an atomic chain snapshot,
the poller queries the full request deadline where it needs the lock deadline, and malformed
present exact-price strings are treated like legacy missing data.

## Design

### Block-pinned market polling

Fetch the latest block before constructing the market-status batch. Use that block's number as the
explicit block tag for every `eth_call` in the batch, and use the same block timestamp for status
classification. If the block lookup fails, preserve the existing transient-error path. This makes
the fulfilled, locked, lock-deadline, and timestamp observations one coherent chain snapshot.

### Correct lock-expiry source

Replace `requestDeadline(uint256)` with `requestLockDeadline(uint256)`. A locked request observed
after that deadline but before the full request expiry is `LockExpired`, allowing the bidding
session to rotate the request ID. The full request expiry remains sourced from the persisted exact
request and is checked independently.

### Fail-closed exact-price parsing

Keep `max_price_wei = None` compatible with records written before the field existed. When the
field is present, parse it as `U256` and return a stored-submission error on malformed input instead
of converting it to the legacy zero sentinel. This preserves the previous-price floor and cap
conflict checks for every current snapshot.

## Testing

- Assert the generated status batch pins all `eth_call` entries to the supplied block number and
  uses the `requestLockDeadline` selector.
- Assert a timestamp between lock expiry and request expiry produces `LockExpired`.
- Assert a present malformed exact price fails snapshot conversion while a missing price remains
  accepted for legacy compatibility.
- Run the focused Boundless tests, native check/clippy/formatting, and the repository's relevant
  prover lane.

## Non-goals

- Change final-rung payable-window semantics.
- Change retry budgets, price ladders, receipt confirmation, or persistence schema version.
- Remove legacy flat-record compatibility.
