# Boundless Submission Serialization Design

## Goal

Serialize every on-chain Boundless submission made by one Raiko2 process so funding calculations
and transaction nonces share one account-level order, while bounding how long a slow transaction can
delay later submissions.

## Scope

One Raiko2 process is the only writer for its Boundless signer. Cross-process or shared-signer
coordination is out of scope. A task that remains in the engine queue is not a market liability and
does not enter funding state.

## Controller

`BoundlessBalanceGate` owns two independent controls:

- An account-level submission permit serializes snapshot reads, balance reads, funding calculation,
  local reservation, transaction broadcast, and a short receipt wait across every configured
  Boundless network pair.
- A funding-state mutex protects the local submitted-but-not-yet-indexed overlay. It is held only for
  short in-memory operations and never across network I/O.

The request is recorded in the local overlay before broadcast so cancellation or an uncertain RPC
result cannot erase a transaction that may still reach the market.

## Submission Flow

For request B:

1. Acquire the shared account submission permit.
2. Fetch the complete indexer snapshot and aligned on-chain market balance.
3. Calculate B's attached value using indexed outstanding requests plus the local overlay.
4. Persist B's request identifier so a task cannot broadcast before its recovery identity is durable.
5. Record B as submitted-but-unlocked in the local overlay.
6. Broadcast the transaction with a 30-second timeout and persist its transaction hash when available.
7. After a successful broadcast, wait up to 10 seconds for its receipt while retaining the permit.
8. Release the permit. The next request then observes either B's confirmed balance contribution or
   the conservative local unlocked reservation.

A successful reverted receipt removes B from the local overlay because neither its request nor its
attached value took effect. A broadcast error, broadcast timeout, or receipt timeout keeps B in the
overlay: the result may be an over-deposit, but cannot create an underfunded request set.

## Restart Behavior

The shared funding overlay remains process-local and is not reconstructed globally from task
checkpoints. After restart, `indexer_caught_up` is false, so submissions do not reuse the reported
market balance until the indexer observes a request sent by the new process. This may deposit extra
funds, which remain in the market account for later requests, but it preserves funding safety without
a distributed recovery protocol.

The Boundless SDK queries the maximum of latest and pending account nonces after restart. The local
submission permit additionally guarantees that only one client in the process performs nonce
selection and broadcast at a time.

## Verification

- Concurrent submissions cannot enter the account submission section together.
- Funding-state access does not retain a mutex guard across network waits.
- A successful receipt releases the next submission with B represented by both chain balance and
  local liability.
- A reverted receipt removes B's local liability.
- Broadcast and receipt timeouts release the next submission while retaining B conservatively.
- Existing cold-start, rebid digest, indexer pagination, and funding calculations continue to pass.
