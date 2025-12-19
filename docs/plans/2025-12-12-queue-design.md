# Raiko2 Queue (DAG‑aware) Design

Date: 2025‑12‑12  
Status: validated with maintainer (goal B, breaking OK, priority 1>2>3)

## Context

Raiko V1 (repo `taikoxyz/raiko`) uses a simple in‑process queue in `reqactor/src/queue.rs`:

- three FIFO queues by priority (Aggregation > BatchProof > Preflight),
- `queued_keys` for de‑dup and capacity,
- `working_in_progress` for in‑flight,
- backend loop with `Semaphore` for concurrency and `Notify` for wakeups.

It is predictable and small, but expresses only linear tasks and has no abstraction for persistence or dependency graphs.

Raiko2’s philosophy is Shasta‑only, zkVM‑only, modular crates, minimal cross‑deps, thin binary, and future replaceability. We also need tasks with explicit dependencies (e.g., an aggregation task depends on multiple batch proofs).

## Goals

1. Add a queue mechanism to raiko2 that supports **task DAGs** (B depends on many As).
2. Preserve raiko‑style priority semantics for “ready” work.
3. Keep raiko2 boundaries clean: queue logic **outside engine/bin**, payload opaque to queue.
4. **First milestone:** single‑instance in‑memory backend with bounded concurrency.
5. **Second milestone:** pluggable persistence / distributed backend (Redis KV, TaskDB, etc.) without changing scheduling logic.

## Non‑goals (first milestone)

- Cross‑process strong consistency.
- Exotic scheduling policies (fair‑share, SLA windows, etc.).
- Partial‑success dependency policies (all‑of/any‑of) beyond “all deps must succeed”.

## Crate layout

New crate:

```
crates/queue/           # package = raiko2-queue
  src/
    lib.rs              # public API
    scheduler.rs        # DAG + priority scheduling
    store.rs            # TaskStore trait + MemoryStore
    types.rs            # TaskId, TaskKind, Priority, TaskState, NewTask, TaskLease
```

The crate depends only on std + tokio (for Mutex/Notify), no reth/alethia/protocol deps.

## Core types

- `TaskId`: ULID/UUID (opaque).
- `TaskKey`: optional, for business de‑dup (e.g., `batch_id+proof_type`); queue crate treats as bytes/hashable.
- `TaskKind`: semantic class used for priority/metrics. First milestone aligns with raiko2 needs:
  - `Preflight`
  - `BuildGuestInput`
  - `BatchProof`
  - `Aggregation`
- `Priority`: `High | Medium | Low`. Derived from `TaskKind` by engine.
- `TaskPayload<P>`: generic payload, opaque to queue.
- `TaskOutput<O>`: generic output, opaque to queue.
- `TaskState`:
  - `Pending { remaining_deps: usize }`
  - `Ready`
  - `Running { worker: String }`
  - `Succeeded { output: O }`
  - `Failed { error: String, caused_by_dep: Option<TaskId> }`
  - `Cancelled`

## Scheduler data model

Scheduler maintains:

- `tasks: HashMap<TaskId, TaskMeta<P,O>>`
- `dependents: HashMap<TaskId, Vec<TaskId>>` (reverse edges)
- `deps: HashMap<TaskId, Vec<TaskId>>` (forward edges)
- `remaining_deps: HashMap<TaskId, usize>`
- `ready_queues: {High,Medium,Low} -> VecDeque<TaskId>`

When a dependency completes:

1. mark dep Succeeded/Failed/Cancelled;
2. for each dependent:
   - if dep failed/cancelled → dependent `Failed{caused_by_dep=dep}` (no enqueue);
   - else decrement `remaining_deps`;
   - if reaches 0 and state still Pending → push into ready queue by its priority.

Priority only affects **Ready** tasks. Pending tasks with unsatisfied deps are not schedulable.

## Public API (first milestone)

```rust
pub struct Scheduler<P, O, S: TaskStore<P,O>> { /* ... */ }

impl<P, O, S: TaskStore<P,O>> Scheduler<P,O,S> {
    pub async fn submit(&self, task: NewTask<P>, deps: Vec<TaskId>) -> TaskId;
    pub async fn submit_group(&self, tasks: Vec<NewTask<P>>) -> Vec<TaskId>; // convenience

    pub async fn next_ready(&self, worker: &str) -> Option<TaskLease<P>>;
    pub async fn complete(&self, id: TaskId, result: Result<O, String>);
    pub async fn cancel(&self, id: TaskId);

    pub async fn get(&self, id: TaskId) -> Option<TaskView<O>>;
    pub async fn list(&self, filter: ListFilter) -> Vec<TaskView<O>>;
}
```

`TaskLease<P>` guarantees that a task taken from ready is marked Running atomically and must be completed/cancelled.

## TaskStore abstraction (future backend)

```rust
#[async_trait]
pub trait TaskStore<P,O>: Send + Sync {
    async fn insert_task(&self, meta: TaskMeta<P,O>);
    async fn get_task(&self, id: &TaskId) -> Option<TaskMeta<P,O>>;
    async fn set_state(&self, id: &TaskId, state: TaskState<O>);

    async fn add_deps(&self, id: &TaskId, deps: Vec<TaskId>);
    async fn add_dependent(&self, dep: &TaskId, id: TaskId);
    async fn dependents_of(&self, dep: &TaskId) -> Vec<TaskId>;

    async fn dec_remaining_deps(&self, id: &TaskId) -> usize;

    async fn push_ready(&self, prio: Priority, id: TaskId);
    async fn pop_ready(&self, prio: Priority) -> Option<TaskId>;
}
```

First milestone implements `MemoryStore` with a single `tokio::Mutex` over HashMaps/VecDeques.

Second milestone adds `RedisStore` behind feature `redis-store`:
atomic `pop_ready + set_running` via Lua/txn; dependents update via pipeline; ready queues as lists/zsets.

## Integration with raiko2

### Engine as the only executor

`raiko2-engine` owns:

- `Scheduler<EngineTask, EngineOutput, MemoryStore<…>>`
- a background worker loop with bounded concurrency (Semaphore).

Engine submits tasks representing the proving pipeline:

1. `BuildGuestInput` (Low)  
2. `BatchProof` (Medium) depends on (1)  
3. `Aggregation` (High) depends on N batch proofs  

Handlers live in engine and call existing crates:

- `provider/driver/stateless` to build inputs
- `prover` to generate proofs

Binary (`bin/raiko2`) becomes a thin shell:

- `POST /v2/proof/batch` → engine submits tasks and returns TaskId
- `POST /v2/proof/aggregate` → engine submits aggregation task with deps
- `GET /v2/proof/:id` → scheduler.get
- `DELETE /v2/proof/:id` → scheduler.cancel

### Failure / cancellation semantics

Milestone 1 policy:

- Any dependency failure/cancel ⇒ dependent fails automatically (`caused_by_dep`).
- Cancelling a running task sets Cancelled and relies on handler to check a cancel flag periodically (engine can implement cooperative cancellation).

Policy hooks are left in scheduler for future per‑task policies.

## Migration plan

Milestone 1 (single instance, memory):

1. Add `raiko2-queue` crate with Scheduler + MemoryStore + tests.
2. Add Engine task enum + worker loop using Scheduler.
3. Switch HTTP handlers to engine‑backed task submission/status.
4. Update docs/API to describe task ids + DAG behaviour.

Milestone 2 (distributed backend):

1. Implement `RedisStore` in `raiko2-queue` behind feature.
2. Add config in bin to choose store backend.
3. Multi‑worker safety tests (atomics, crash recovery).

## Open questions (deferred)

- De‑dup semantics: should TaskKey be mandatory for some kinds?
- Retry policy per TaskKind (backoff, max tries).
- Partial dependency policies (any‑of vs all‑of).

