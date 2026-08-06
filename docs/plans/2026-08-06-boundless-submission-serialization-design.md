# Boundless Submission Serialization Design

## Goal

Serialize every on-chain Boundless submission made by one Raiko2 process so funding calculations
and transaction nonces share one account-level order. Every external wait is bounded, and a timed-out
broadcast preserves enough identity to recover the same request at the same nonce before any later
request is sent.

## Scope

One Raiko2 process is the only writer for its Boundless signer. Cross-process or shared-signer
coordination is out of scope. A task that remains in the engine queue is not a market liability and
does not enter funding state.

## Controller

The submission path uses three independent controls:

- A lifecycle `SubmissionCheckpointPermit` prevents the engine from cancelling or replacing a task
  between remote submission acceptance and durable progress persistence. It is not the account lock.
- An account-level submission permit serializes reconciliation of an uncertain predecessor, snapshot
  reads, balance and nonce reads, funding calculation, local reservation, transaction broadcast, and
  receipt observation across every configured Boundless network pair.
- A funding-state mutex protects the local submitted-but-not-yet-indexed overlay. It is held only for
  short in-memory operations and never across network I/O.

The request is recorded in the local overlay before broadcast so cancellation or an uncertain RPC
result cannot erase a transaction that may still reach the market. The account state also records a
local nonce high-water mark and, while necessary, one uncertain submission containing the original
request, signature, attached value, request digest, and explicit nonce.

## Submission Flow

For request B:

1. Sign B and persist its request identifier before acquiring the account submission permit. The
   lifecycle checkpoint permit is released as soon as this required checkpoint is durable.
2. Acquire the shared account submission permit.
3. If an earlier nonce has an uncertain broadcast outcome, reconcile or retry that exact signed
   request at its original nonce. Do not send B until the predecessor is accepted or its nonce is
   otherwise consumed.
4. Fetch the complete indexer snapshot and aligned on-chain market balance. Query latest and pending
   account nonces and select the maximum of those values and the local high-water mark.
5. Calculate B's attached value using indexed outstanding requests plus the local overlay. Record B's
   funding reservation and explicit nonce before broadcast.
6. Broadcast B with a 30-second timeout. On timeout or an ambiguous RPC error, reconcile chain nonce,
   indexer state, and the request digest; if necessary, retry B with the same request, signature,
   value, and nonce.
7. Once broadcast is accepted, wait for one receipt that has three canonical confirmations. Receipt
   observation has a bounded total timeout and retains the account submission permit.
8. A three-confirmation success keeps B's funding reservation until the indexer observes it. A
   three-confirmation revert removes B's reservation because the attached value did not take effect.
9. Release the account submission permit. An optional transaction-hash checkpoint runs outside this
   critical section with its own total timeout.

A broadcast or receipt result that remains uncertain after bounded recovery keeps B in both the
funding overlay and the uncertain-nonce slot. The current call may return for polling, but the next
submission must resolve that slot before allocating another nonce. This may over-deposit, but cannot
replace B with a different request or create an underfunded request set.

Every indexer, RPC, send, receipt, and optional checkpoint operation has an individual timeout where
useful and an outer total timeout covering all retries. The required request-id checkpoint instead
retries until it is durable or the lifecycle closes, but it never holds the account submission
permit. Timeout paths fail closed for funding and nonce allocation rather than holding either permit
indefinitely.

## Restart Behavior

The shared funding overlay remains process-local and is not reconstructed globally from task
checkpoints. After restart, `indexer_caught_up` is false, so submissions do not reuse the reported
market balance until the indexer observes a request sent by the new process. This may deposit extra
funds, which remain in the market account for later requests, but it preserves funding safety without
a distributed recovery protocol.

After restart, latest and pending chain nonces are the nonce source of truth because the local
high-water mark and uncertain-submission slot are process-local. The recovery window normally lets a
broadcast reach the pending view before the process exits. A cold process remains conservative about
funding and never assumes that an absent in-memory reservation means the corresponding funds are
available.

## Verification

- Concurrent submissions cannot enter the account submission section together.
- Funding-state access does not retain a mutex guard across network waits.
- Required request-id persistence and optional transaction-hash persistence do not hold the account
  submission permit.
- Every receipt outcome is evaluated only after three confirmations.
- A successful receipt retains B until indexer catch-up; a reverted receipt removes B.
- A timed-out broadcast cannot let the next request reuse or skip B's nonce.
- Indexer, balance, nonce, optional checkpoint, send, and receipt waits all have bounded total
  duration.
- Existing cold-start, rebid digest, indexer pagination, and funding calculations continue to pass.
