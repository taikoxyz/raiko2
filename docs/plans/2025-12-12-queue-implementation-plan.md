# Raiko2 Queue Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task‑by‑task.  
> **Also required:** superpowers:test-driven-development for each behavior change.

**Goal:** Add a DAG‑aware, priority‑based in‑process queue to raiko2, integrated into `raiko2-engine` and used by `bin/raiko2` for proof job lifecycle. First milestone uses a single‑instance in‑memory backend.

**Architecture:** Create new `raiko2-queue` crate with `Scheduler` + `TaskStore` trait + `MemoryStore`. Engine owns a `Scheduler<EngineTask, EngineOutput, MemoryStore>` plus worker loop (Semaphore + Notify). Bin handlers submit tasks and query/cancel via engine only.

**Tech Stack:** Rust 2024, tokio, async-trait, axum.

---

### Task 1: Scaffold `raiko2-queue` crate and core types

**Files:**
- Create: `crates/queue/Cargo.toml`
- Create: `crates/queue/src/lib.rs`
- Create: `crates/queue/src/types.rs`

**Step 1: Write failing tests**

Create `crates/queue/src/types.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(uuid::Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    High,
    Medium,
    Low,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_is_unique() {
        let a = TaskId::new();
        let b = TaskId::new();
        assert_ne!(a, b);
    }
}
```

Initially this will fail to compile because crate/files don’t exist yet.

**Step 2: Run test to verify failure**

Run: `cargo test -p raiko2-queue task_id_is_unique`  
Expected: compile error / missing crate.

**Step 3: Add minimal crate skeleton**

Create `crates/queue/Cargo.toml`:

```toml
[package]
name = "raiko2-queue"
version.workspace = true
edition.workspace = true

[dependencies]
tokio = { workspace = true, features = ["sync"] }
async-trait.workspace = true
serde = { workspace = true, features = ["derive"] }
uuid = { version = "1.8", features = ["v4", "serde"] }
```

Create `crates/queue/src/lib.rs`:

```rust
mod scheduler;
mod store;
mod types;

pub use scheduler::{NewTask, Scheduler, TaskLease, TaskView};
pub use store::{MemoryStore, TaskStore};
pub use types::{Priority, TaskId, TaskKind, TaskState};
```

Add `TaskKind` and `TaskState` to `types.rs` (minimal enums; behavior later).

**Step 4: Run tests to verify green**

Run: `cargo test -p raiko2-queue task_id_is_unique`  
Expected: PASS.

**Step 5: Commit**

`git add crates/queue docs/plans/2025-12-12-queue-implementation-plan.md`  
`git commit -m "feat: scaffold raiko2-queue crate and core types"`

---

### Task 2: Implement `TaskStore` trait + `MemoryStore`

**Files:**
- Create: `crates/queue/src/store.rs`
- Modify: `crates/queue/src/lib.rs` (re-export)
- Test: `crates/queue/src/store.rs`

**Step 1: Write failing tests**

In `crates/queue/src/store.rs`:

```rust
use crate::{Priority, TaskId, TaskState};
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use tokio::sync::Mutex;

#[async_trait]
pub trait TaskStore<P, O>: Send + Sync {
    async fn insert_task(&self, id: TaskId, payload: P, prio: Priority, deps: Vec<TaskId>);
    async fn get_state(&self, id: &TaskId) -> Option<TaskState<O>>;
    async fn set_state(&self, id: &TaskId, state: TaskState<O>);
    async fn dependents_of(&self, dep: &TaskId) -> Vec<TaskId>;
    async fn dec_remaining_deps(&self, id: &TaskId) -> usize;
    async fn push_ready(&self, prio: Priority, id: TaskId);
    async fn pop_ready(&self, prio: Priority) -> Option<TaskId>;
}

pub struct MemoryStore<P, O> {
    inner: Mutex<Inner<P, O>>,
}

struct Inner<P, O> {
    tasks: HashMap<TaskId, (P, TaskState<O>, Priority)>,
    dependents: HashMap<TaskId, Vec<TaskId>>,
    remaining: HashMap<TaskId, usize>,
    ready_high: VecDeque<TaskId>,
    ready_med: VecDeque<TaskId>,
    ready_low: VecDeque<TaskId>,
}

impl<P, O> MemoryStore<P, O> {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                tasks: HashMap::new(),
                dependents: HashMap::new(),
                remaining: HashMap::new(),
                ready_high: VecDeque::new(),
                ready_med: VecDeque::new(),
                ready_low: VecDeque::new(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TaskState;

    #[tokio::test]
    async fn ready_push_pop_respects_priority() {
        let store: MemoryStore<(), ()> = MemoryStore::new();
        let a = TaskId::new();
        let b = TaskId::new();
        store.push_ready(Priority::Low, a).await;
        store.push_ready(Priority::High, b).await;
        assert_eq!(store.pop_ready(Priority::High).await, Some(b));
        assert_eq!(store.pop_ready(Priority::Low).await, Some(a));
    }
}
```

**Step 2: Run test to verify failure**

Run: `cargo test -p raiko2-queue ready_push_pop_respects_priority`  
Expected: FAIL (trait methods unimplemented).

**Step 3: Minimal implementation**

Implement `TaskStore` for `MemoryStore`:

```rust
#[async_trait]
impl<P: Send + 'static, O: Send + 'static> TaskStore<P, O> for MemoryStore<P, O> {
    async fn insert_task(&self, id: TaskId, payload: P, prio: Priority, deps: Vec<TaskId>) {
        let mut g = self.inner.lock().await;
        g.remaining.insert(id, deps.len());
        for dep in deps {
            g.dependents.entry(dep).or_default().push(id);
        }
        g.tasks.insert(id, (payload, TaskState::pending(g.remaining[&id]), prio));
    }

    async fn get_state(&self, id: &TaskId) -> Option<TaskState<O>> {
        let g = self.inner.lock().await;
        g.tasks.get(id).map(|(_, s, _)| s.clone())
    }

    async fn set_state(&self, id: &TaskId, state: TaskState<O>) {
        let mut g = self.inner.lock().await;
        if let Some((_, s, _)) = g.tasks.get_mut(id) {
            *s = state;
        }
    }

    async fn dependents_of(&self, dep: &TaskId) -> Vec<TaskId> {
        let g = self.inner.lock().await;
        g.dependents.get(dep).cloned().unwrap_or_default()
    }

    async fn dec_remaining_deps(&self, id: &TaskId) -> usize {
        let mut g = self.inner.lock().await;
        let entry = g.remaining.entry(*id).or_insert(0);
        if *entry > 0 { *entry -= 1; }
        *entry
    }

    async fn push_ready(&self, prio: Priority, id: TaskId) {
        let mut g = self.inner.lock().await;
        match prio {
            Priority::High => g.ready_high.push_back(id),
            Priority::Medium => g.ready_med.push_back(id),
            Priority::Low => g.ready_low.push_back(id),
        }
    }

    async fn pop_ready(&self, prio: Priority) -> Option<TaskId> {
        let mut g = self.inner.lock().await;
        match prio {
            Priority::High => g.ready_high.pop_front(),
            Priority::Medium => g.ready_med.pop_front(),
            Priority::Low => g.ready_low.pop_front(),
        }
    }
}
```

Add helper ctor to `TaskState` in `types.rs`:

```rust
impl<O> TaskState<O> {
    pub fn pending(remaining: usize) -> Self {
        TaskState::Pending { remaining_deps: remaining }
    }
}
```

**Step 4: Run tests**

`cargo test -p raiko2-queue ready_push_pop_respects_priority`

**Step 5: Commit**

`git commit -m "feat: add TaskStore and MemoryStore"`

---

### Task 3: Implement `Scheduler::submit` and priority ready ordering

**Files:**
- Create: `crates/queue/src/scheduler.rs`
- Modify: `crates/queue/src/lib.rs`
- Test: `crates/queue/src/scheduler.rs`

**Step 1: Write failing tests**

In `scheduler.rs`:

```rust
use crate::{MemoryStore, Priority, TaskId, TaskKind, TaskState, TaskStore};
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct NewTask<P> {
    pub kind: TaskKind,
    pub priority: Priority,
    pub payload: P,
}

pub struct TaskLease<P> {
    pub id: TaskId,
    pub payload: P,
    pub kind: TaskKind,
    pub priority: Priority,
}

pub struct Scheduler<P, O, S: TaskStore<P,O>> {
    store: Arc<S>,
    notify: Arc<Notify>,
}

impl<P, O, S: TaskStore<P,O>> Scheduler<P,O,S> {
    pub fn new(store: S) -> Self {
        Self { store: Arc::new(store), notify: Arc::new(Notify::new()) }
    }

    pub fn notifier(&self) -> Arc<Notify> { self.notify.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn next_ready_picks_high_before_low() {
        let sched: Scheduler<&'static str, (), MemoryStore<&'static str, ()>> =
            Scheduler::new(MemoryStore::new());

        let a = sched.submit(NewTask { kind: TaskKind::Preflight, priority: Priority::Low, payload: "a" }, vec![]).await;
        let b = sched.submit(NewTask { kind: TaskKind::Aggregation, priority: Priority::High, payload: "b" }, vec![]).await;

        let first = sched.next_ready("w").await.unwrap();
        assert_eq!(first.id, b);
        let second = sched.next_ready("w").await.unwrap();
        assert_eq!(second.id, a);
    }
}
```

**Step 2: Run test to verify failure**

`cargo test -p raiko2-queue next_ready_picks_high_before_low`  
Expected: FAIL (`submit/next_ready` missing).

**Step 3: Minimal implementation**

Implement:

```rust
impl<P: Send + 'static, O: Send + 'static, S: TaskStore<P,O>> Scheduler<P,O,S> {
    pub async fn submit(&self, task: NewTask<P>, deps: Vec<TaskId>) -> TaskId {
        let id = TaskId::new();
        self.store.insert_task(id, task.payload, task.priority, deps).await;
        if self.store.dec_remaining_deps(&id).await == 0 {
            // deps empty case (remaining==0)
            self.store.push_ready(task.priority, id).await;
            self.notify.notify_one();
        }
        id
    }

    pub async fn next_ready(&self, worker: &str) -> Option<TaskLease<P>> {
        let id = self.store
            .pop_ready(Priority::High).await
            .or_else(|| futures::executor::block_on(self.store.pop_ready(Priority::Medium)))
            .or_else(|| futures::executor::block_on(self.store.pop_ready(Priority::Low)))?;

        // NOTE: MemoryStore stores payload; add getter in store to extract payload+kind+prio.
        unimplemented!()
    }
}
```

Adjust `TaskStore` to allow extracting payload + kind + priority for a lease.

**Step 4: Run tests**

`cargo test -p raiko2-queue next_ready_picks_high_before_low`

**Step 5: Commit**

`git commit -m "feat: add Scheduler submit/next_ready ordering"`

---

### Task 4: Add DAG dependency release + failure propagation

**Files:**
- Modify: `crates/queue/src/scheduler.rs`
- Test: `crates/queue/src/scheduler.rs`

**Step 1: Write failing tests**

Add tests:

```rust
#[tokio::test]
async fn dependent_enters_ready_after_all_deps_complete() {
    let sched: Scheduler<&'static str, &'static str, MemoryStore<&'static str, &'static str>> =
        Scheduler::new(MemoryStore::new());

    let a1 = sched.submit(NewTask { kind: TaskKind::BatchProof, priority: Priority::Medium, payload: "a1" }, vec![]).await;
    let a2 = sched.submit(NewTask { kind: TaskKind::BatchProof, priority: Priority::Medium, payload: "a2" }, vec![]).await;
    let b  = sched.submit(NewTask { kind: TaskKind::Aggregation, priority: Priority::High, payload: "b" }, vec![a1, a2]).await;

    assert!(sched.next_ready("w").await.map(|l| l.id) != Some(b));

    sched.complete(a1, Ok("ok")).await;
    assert!(sched.next_ready("w").await.map(|l| l.id) != Some(b));

    sched.complete(a2, Ok("ok")).await;
    assert_eq!(sched.next_ready("w").await.unwrap().id, b);
}

#[tokio::test]
async fn failure_propagates_to_dependents() {
    let sched: Scheduler<&'static str, (), MemoryStore<&'static str, ()>> =
        Scheduler::new(MemoryStore::new());
    let a = sched.submit(NewTask { kind: TaskKind::BatchProof, priority: Priority::Medium, payload: "a" }, vec![]).await;
    let b = sched.submit(NewTask { kind: TaskKind::Aggregation, priority: Priority::High, payload: "b" }, vec![a]).await;

    sched.complete(a, Err("boom".into())).await;
    let b_state = sched.get(b).await.unwrap().state;
    assert!(matches!(b_state, TaskState::Failed{..}));
}
```

**Step 2: Run tests (expect fail)**  
`cargo test -p raiko2-queue dependent_enters_ready_after_all_deps_complete`

**Step 3: Minimal implementation**

Add `Scheduler::complete`:

```rust
pub async fn complete(&self, id: TaskId, result: Result<O, String>) {
    match result {
        Ok(out) => self.store.set_state(&id, TaskState::Succeeded { output: out }).await,
        Err(err) => self.store.set_state(&id, TaskState::Failed { error: err, caused_by_dep: None }).await,
    }

    let deps_ok = self.store.get_state(&id).await.map(|s| matches!(s, TaskState::Succeeded{..})).unwrap_or(false);
    for dep in self.store.dependents_of(&id).await {
        if !deps_ok {
            self.store.set_state(&dep, TaskState::Failed { error: "dependency failed".into(), caused_by_dep: Some(id) }).await;
            continue;
        }
        let remaining = self.store.dec_remaining_deps(&dep).await;
        if remaining == 0 {
            let prio = self.store.get_priority(&dep).await.unwrap();
            self.store.push_ready(prio, dep).await;
        }
    }
    self.notify.notify_one();
}
```

**Step 4: Run tests (expect green)**  
`cargo test -p raiko2-queue dependent_enters_ready_after_all_deps_complete failure_propagates_to_dependents`

**Step 5: Commit**  
`git commit -m "feat: add DAG completion propagation"`

---

### Task 5: Integrate Scheduler into `raiko2-engine`

**Files:**
- Modify: `crates/engine/Cargo.toml` (add `raiko2-queue`, `tokio`)
- Modify: `crates/engine/src/lib.rs`
- Create: `crates/engine/src/tasks.rs`
- Test: `crates/engine/src/tasks.rs`

**Step 1: Write failing tests**

Create `crates/engine/src/tasks.rs`:

```rust
use raiko2_queue::{NewTask, Priority, Scheduler, TaskKind, MemoryStore};

#[derive(Clone, Debug)]
pub enum EngineTask {
    BuildGuestInput { batch_id: u64 },
    ProveBatch { batch_id: u64 },
    Aggregate { batch_ids: Vec<u64> },
}

pub type EngineOutput = raiko2_primitives::Proof;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aggregation_depends_on_batches() {
        let sched: Scheduler<EngineTask, EngineOutput, MemoryStore<EngineTask, EngineOutput>> =
            Scheduler::new(MemoryStore::new());

        let a1 = sched.submit(NewTask { kind: TaskKind::BatchProof, priority: Priority::Medium, payload: EngineTask::ProveBatch{batch_id:1}}, vec![]).await;
        let a2 = sched.submit(NewTask { kind: TaskKind::BatchProof, priority: Priority::Medium, payload: EngineTask::ProveBatch{batch_id:2}}, vec![]).await;
        let b  = sched.submit(NewTask { kind: TaskKind::Aggregation, priority: Priority::High, payload: EngineTask::Aggregate{batch_ids:vec![1,2]}}, vec![a1,a2]).await;

        sched.complete(a1, Ok(EngineOutput::default())).await;
        sched.complete(a2, Ok(EngineOutput::default())).await;
        assert_eq!(sched.next_ready(\"w\").await.unwrap().id, b);
    }
}
```

**Step 2: Run tests (expect fail)**  
`cargo test -p raiko2-engine aggregation_depends_on_batches`

**Step 3: Minimal implementation**

- Add deps in `crates/engine/Cargo.toml`:

```toml
raiko2-queue = { path = "../queue" }
tokio = { workspace = true, features = ["sync", "rt", "macros"] }
```

- In `crates/engine/src/lib.rs`, add:
  - a `Scheduler` field to `Engine<P>` or a new `EngineService<P>` wrapper.
  - a `run_workers()` async fn that loops `next_ready()` and dispatches to existing engine/driver/prover calls.

**Step 4: Run tests (green)**  
`cargo test -p raiko2-engine aggregation_depends_on_batches`

**Step 5: Commit**  
`git commit -m "feat: integrate queue into engine"`

---

### Task 6: Switch `bin/raiko2` handlers to engine‑backed queue

**Files:**
- Modify: `bin/raiko2/src/server/state.rs`
- Modify: `bin/raiko2/src/server/handlers.rs`
- Modify: `bin/raiko2/src/server/routes.rs`

**Step 1: Write failing tests**

Add an integration‑style test in `bin/raiko2/src/server/handlers.rs` (or `tests/` if preferred) asserting:

- `request_batch_proof` returns task id,
- status endpoint reflects queue state.

**Step 2: Run test (expect fail)**  
`cargo test -p raiko2`

**Step 3: Minimal implementation**

- `AppState` holds `EngineService<NetworkProvider>` (or trait object) instead of `Arc<dyn Prover>` + jobs map.
- `request_batch_proof` calls engine `.submit_batch(batch_id, …)` and returns id.
- `get_proof_status` calls engine `.get_task(id)`.
- `cancel_proof` calls engine `.cancel_task(id)`.

**Step 4: Run tests**

`cargo test -p raiko2`

**Step 5: Commit**

`git commit -m "feat: route HTTP proof jobs through engine queue"`

