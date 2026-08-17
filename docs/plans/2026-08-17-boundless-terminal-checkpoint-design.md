# Boundless Terminal Checkpoint Reset

## Problem

Boundless no-lock rebids reuse one market request ID and persist every attempt as the task's
resumable provider checkpoint. After the final attempt reaches its payable deadline, the proof task
fails, but the attempt-5 checkpoint remains. A later client POST re-enqueues the same deterministic
Raiko task, restores the exhausted request, and fails again instead of starting a new market cycle.

## Decision

Treat the configured no-lock rebid budget as one market cycle. When the final no-lock attempt is
terminal:

1. Atomically clear only the matching Boundless provider checkpoint.
2. Preserve the root task, proposal inputs, dependency proofs, preflight artifacts, and task history.
3. Return failure for the current task execution.
4. Let the next client POST re-enqueue the same Raiko task and create a fresh Boundless request ID at
   attempt 1.

The clear operation must compare the backend, provider request ID, and attempt before mutation. A
missing or mismatched checkpoint is an error, because starting a new payable request after clearing a
different or newer checkpoint could create duplicate payment exposure.

Final-attempt terminalization must also be confirmed by the latest chain timestamp. Host wall-clock
time may trigger ordinary rebids, but it cannot prove that a request's payable window has closed.
The final no-lock checkpoint is therefore cleared only when the latest block timestamp is strictly
greater than the request's lock deadline, matching the Boundless market pricing boundary. At the
exact deadline the request remains payable. If the status RPC cannot provide a block timestamp, the
final attempt remains pending and retains its checkpoint.

## API And Persistence

Extend `ProverProgressObserver` with a terminal-checkpoint operation carrying a compact expected
identity. The engine adapter forwards it to the runtime observer under the existing submission
checkpoint and execution lifecycle permits. The runtime observer updates every active root owner
atomically and clears the selected proposal or aggregate `TaskRuntimeMetadata` only when its durable
identity matches.

No HTTP response schema changes. The current execution still becomes `failed`; the next POST returns
the normal registered/work-in-progress response for a fresh market cycle.

## Legacy Recovery

An exhausted checkpoint written by an older binary is still resumed once. If its final market poll
confirms no-lock terminal status, the new code clears it before returning failure. The client's next
retry then starts attempt 1. Fulfilled or still-payable submissions are never cleared.

## Failure Handling

- A durable clear is required before reporting the terminal cycle as safely reset.
- A final no-lock clear requires chain-derived confirmation that the payable deadline has passed.
- Retryable persistence errors use the existing checkpoint retry policy.
- Permanent lifecycle, identity, or ownership errors stop the proof task without creating a new
  market request.
- Other Boundless failures keep their checkpoint and existing resume behavior.

## Tests

- Final no-lock exhaustion requests one matching checkpoint clear.
- Non-terminal rebids do not clear progress.
- Matching aggregate and proposal checkpoints clear all remote-submission fields.
- Mismatched provider request ID or attempt is rejected without mutation.
- A failed task with a cleared checkpoint is re-enqueueable and the next submission begins at attempt
  1 with a new provider request ID.
- A legacy exhausted checkpoint is cleared after its terminal status is confirmed.
