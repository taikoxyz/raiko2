mod config;
mod retry;
mod types;

pub use config::SchedulerConfig;
pub use retry::RetryPolicy;
pub use types::{NewTask, TaskLease, TaskView};

use crate::{Priority, TaskId, TaskState, TaskStore, TaskStoreError};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

pub struct Scheduler<P, O: Clone, Id> {
    store: Arc<dyn TaskStore<P, O, Id>>,
    notify: Arc<Notify>,
    config: SchedulerConfig,
    _phantom: core::marker::PhantomData<fn(P, O)>,
}

/// Maximum number of tasks to promote/requeue per maintenance tick.
///
/// This bounds the amount of work done in a single tick so that a periodic
/// maintenance loop stays responsive (i.e. doesn't monopolize the executor or
/// hold store-internal locks for too long when there is a large backlog).
///
/// For high-throughput systems where scheduled tasks / expired leases can
/// accumulate faster than they are drained, this limit may become a bottleneck.
/// In that case, consider calling `maintenance_tick` more frequently or in a
/// loop until it returns `0`, or make this value configurable.
const MAINTENANCE_TICK_LIMIT: usize = 128;

impl<P, O: Clone, Id> Scheduler<P, O, Id>
where
    Id: Send + Sync,
{
    pub fn new<S>(store: S) -> Self
    where
        S: TaskStore<P, O, Id> + 'static,
    {
        Self::with_config(store, SchedulerConfig::default())
    }

    pub fn with_config<S>(store: S, config: SchedulerConfig) -> Self
    where
        S: TaskStore<P, O, Id> + 'static,
    {
        Self::from_arc_with_config(Arc::new(store), config)
    }

    pub fn from_arc(store: Arc<dyn TaskStore<P, O, Id>>) -> Self {
        Self::from_arc_with_config(store, SchedulerConfig::default())
    }

    pub fn from_arc_with_config(
        store: Arc<dyn TaskStore<P, O, Id>>,
        config: SchedulerConfig,
    ) -> Self {
        Self {
            store,
            notify: Arc::new(Notify::new()),
            config,
            _phantom: core::marker::PhantomData,
        }
    }

    pub fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }
}

impl<P, O, Id> Scheduler<P, O, Id>
where
    P: Send + 'static,
    O: Clone + Send + 'static,
    Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
{
    pub async fn submit(
        &self,
        id: TaskId<Id>,
        task: NewTask<P>,
        deps: Vec<TaskId<Id>>,
    ) -> Result<TaskId<Id>, TaskStoreError> {
        // Normalize dependency list to avoid backend-specific behavior.
        //
        // Some stores may de-duplicate dependents (e.g. Redis sets) while still
        // counting `remaining_deps` from the raw list length. If callers pass
        // duplicated deps, that mismatch can strand the task in `Pending`.
        let deps = {
            let mut seen = HashSet::with_capacity(deps.len());
            deps.into_iter()
                .filter(|dep| seen.insert(dep.clone()))
                .collect()
        };

        let inserted = self
            .store
            .insert_task(id.clone(), task.payload, task.priority, deps)
            .await?;
        if inserted && let Some(priority) = self.store.try_mark_ready(&id).await? {
            self.store.push_ready(priority, id.clone()).await?;
            self.notify.notify_one();
        }

        Ok(id)
    }

    pub async fn next_ready(
        &self,
        worker: &str,
    ) -> Result<Option<TaskLease<P, Id>>, TaskStoreError> {
        for prio in [Priority::High, Priority::Medium, Priority::Low] {
            if let Some((id, payload, priority, attempt)) =
                self.store.pop_ready_and_take(prio, worker).await?
            {
                return Ok(Some(TaskLease {
                    id,
                    payload,
                    priority,
                    attempt,
                    worker: worker.to_string(),
                }));
            }
        }

        Ok(None)
    }

    pub async fn complete(
        &self,
        lease: TaskLease<P, Id>,
        result: Result<O, String>,
    ) -> Result<(), TaskStoreError> {
        let TaskLease {
            id,
            payload,
            priority,
            attempt,
            worker,
        } = lease;

        let Some(current) = self.store.get_state(&id).await? else {
            self.notify.notify_one();
            return Ok(());
        };

        if matches!(
            current,
            TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. }
        ) {
            self.notify.notify_one();
            return Ok(());
        }

        let TaskState::Running {
            worker: current_worker,
            attempt: current_attempt,
        } = current
        else {
            self.notify.notify_one();
            return Ok(());
        };

        if current_worker != worker || current_attempt != attempt {
            self.notify.notify_one();
            return Ok(());
        }

        match result {
            Ok(output) => {
                self.store
                    .set_state(&id, TaskState::Succeeded { output })
                    .await?;
            }
            Err(error) => {
                if let Some(delay) = self.config.retry.retry_delay(attempt) {
                    self.store.put_payload(&id, payload).await?;

                    if delay == Duration::ZERO {
                        self.store.set_state(&id, TaskState::Ready).await?;
                        self.store.push_ready(priority, id).await?;
                        self.notify.notify_one();
                        return Ok(());
                    }

                    let now_ms = now_millis();
                    let next_ready_at_ms = now_ms.saturating_add(delay.as_millis() as u64);
                    self.store
                        .set_state(
                            &id,
                            TaskState::Retrying {
                                error,
                                attempt,
                                next_ready_at_ms,
                            },
                        )
                        .await?;
                    self.store.schedule(id, next_ready_at_ms).await?;
                    self.notify.notify_one();
                    return Ok(());
                }

                self.store
                    .set_state(
                        &id,
                        TaskState::Failed {
                            error: error.clone(),
                            caused_by_dep: None,
                        },
                    )
                    .await?;
                self.fail_dependents(id, "dependency failed".to_string())
                    .await?;
                self.notify.notify_one();
                return Ok(());
            }
        }

        for dependent in self.store.dependents_of(&id).await? {
            let remaining = self.store.dec_remaining_deps(&dependent).await?;
            if remaining == 0
                && let Some(priority) = self.store.try_mark_ready(&dependent).await?
            {
                self.store.push_ready(priority, dependent).await?;
            }
        }

        self.notify.notify_one();

        Ok(())
    }

    pub async fn cancel(&self, id: TaskId<Id>) -> Result<(), TaskStoreError> {
        let Some(current) = self.store.get_state(&id).await? else {
            self.notify.notify_one();
            return Ok(());
        };

        if matches!(
            current,
            TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. }
        ) {
            self.notify.notify_one();
            return Ok(());
        }

        self.store.set_state(&id, TaskState::Cancelled).await?;
        self.fail_dependents(id, "dependency cancelled".to_string())
            .await?;
        self.notify.notify_one();

        Ok(())
    }

    pub async fn get(&self, id: TaskId<Id>) -> Result<Option<TaskView<O, Id>>, TaskStoreError> {
        let Some((state, priority)) = self.store.get_view(&id).await? else {
            return Ok(None);
        };
        Ok(Some(TaskView {
            id,
            state,
            priority,
        }))
    }

    pub async fn maintenance_tick(&self) -> Result<usize, TaskStoreError> {
        self.maintenance_tick_at(now_millis()).await
    }

    pub async fn maintenance_tick_at(&self, now_ms: u64) -> Result<usize, TaskStoreError> {
        let moved_scheduled = self
            .store
            .promote_scheduled(now_ms, MAINTENANCE_TICK_LIMIT)
            .await?;
        let moved_leases = self
            .store
            .requeue_expired_leases(now_ms, MAINTENANCE_TICK_LIMIT)
            .await?;
        let moved = moved_scheduled + moved_leases;
        if moved > 0 {
            self.notify.notify_one();
        }
        Ok(moved)
    }

    async fn fail_dependents(&self, root: TaskId<Id>, error: String) -> Result<(), TaskStoreError> {
        let mut queue: VecDeque<TaskId<Id>> = self.store.dependents_of(&root).await?.into();
        let mut visited = HashSet::new();

        while let Some(id) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }

            let state = self.store.get_state(&id).await?;
            if !matches!(
                state,
                Some(TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. })
            ) {
                self.store
                    .set_state(
                        &id,
                        TaskState::Failed {
                            error: error.clone(),
                            caused_by_dep: Some(root.clone()),
                        },
                    )
                    .await?;
            }

            for dependent in self.store.dependents_of(&id).await? {
                queue.push_back(dependent);
            }
        }

        Ok(())
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time is before UNIX_EPOCH")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use crate::StoreResult;
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;
    use tokio::sync::Mutex;

    type TestId = u64;
    type TestTaskId = TaskId<TestId>;

    fn test_id(value: u64) -> TestTaskId {
        TaskId::new(value)
    }

    struct BuggyTakeStore<P, O> {
        inner: MemoryStore<P, O, TestId>,
    }

    struct UniqueDependentsStore<P, O> {
        inner: Mutex<UniqueDependentsInner<P, O>>,
    }

    struct UniqueDependentsInner<P, O> {
        tasks: HashMap<TestTaskId, UniqueTaskRecord<P, O>>,
        dependents: HashMap<TestTaskId, HashSet<TestTaskId>>,
        remaining: HashMap<TestTaskId, usize>,
        ready_high: VecDeque<TestTaskId>,
        ready_medium: VecDeque<TestTaskId>,
        ready_low: VecDeque<TestTaskId>,
    }

    struct UniqueTaskRecord<P, O> {
        payload: Option<P>,
        state: TaskState<O, TestId>,
        priority: Priority,
        attempt: u32,
    }

    impl<P, O> UniqueDependentsStore<P, O> {
        fn new() -> Self {
            Self {
                inner: Mutex::new(UniqueDependentsInner {
                    tasks: HashMap::new(),
                    dependents: HashMap::new(),
                    remaining: HashMap::new(),
                    ready_high: VecDeque::new(),
                    ready_medium: VecDeque::new(),
                    ready_low: VecDeque::new(),
                }),
            }
        }
    }

    #[async_trait::async_trait]
    impl<P: Clone + Send + 'static, O: Clone + Send + 'static> TaskStore<P, O, TestId>
        for UniqueDependentsStore<P, O>
    {
        async fn insert_task(
            &self,
            id: TestTaskId,
            payload: P,
            prio: Priority,
            deps: Vec<TestTaskId>,
        ) -> StoreResult<bool> {
            let mut guard = self.inner.lock().await;
            if guard.tasks.contains_key(&id) {
                return Ok(false);
            }
            guard.remaining.insert(id.clone(), deps.len());
            for dep in deps {
                guard.dependents.entry(dep).or_default().insert(id.clone());
            }
            let remaining = guard.remaining.get(&id).copied().unwrap_or(0);
            guard.tasks.insert(
                id,
                UniqueTaskRecord {
                    payload: Some(payload),
                    state: TaskState::pending(remaining),
                    priority: prio,
                    attempt: 0,
                },
            );
            Ok(true)
        }

        async fn get_state(&self, id: &TestTaskId) -> StoreResult<Option<TaskState<O, TestId>>> {
            let guard = self.inner.lock().await;
            Ok(guard.tasks.get(id).map(|record| record.state.clone()))
        }

        async fn set_state(&self, id: &TestTaskId, state: TaskState<O, TestId>) -> StoreResult<()> {
            let mut guard = self.inner.lock().await;
            if let Some(record) = guard.tasks.get_mut(id) {
                record.state = state;
            }
            Ok(())
        }

        async fn get_view(
            &self,
            id: &TestTaskId,
        ) -> StoreResult<Option<(TaskState<O, TestId>, Priority)>> {
            let guard = self.inner.lock().await;
            let Some(record) = guard.tasks.get(id) else {
                return Ok(None);
            };
            Ok(Some((record.state.clone(), record.priority)))
        }

        async fn dependents_of(&self, dep: &TestTaskId) -> StoreResult<Vec<TestTaskId>> {
            let guard = self.inner.lock().await;
            Ok(guard
                .dependents
                .get(dep)
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default())
        }

        async fn dec_remaining_deps(&self, id: &TestTaskId) -> StoreResult<usize> {
            let mut guard = self.inner.lock().await;
            let entry = guard.remaining.entry(id.clone()).or_insert(0);
            if *entry > 0 {
                *entry -= 1;
            }
            let remaining = *entry;

            if let Some(record) = guard.tasks.get_mut(id)
                && matches!(record.state, TaskState::Pending { .. })
            {
                record.state = TaskState::pending(remaining);
            }

            Ok(remaining)
        }

        async fn try_mark_ready(&self, id: &TestTaskId) -> StoreResult<Option<Priority>> {
            let mut guard = self.inner.lock().await;
            let remaining = guard.remaining.get(id).copied().unwrap_or(0);
            if remaining != 0 {
                return Ok(None);
            }
            let Some(record) = guard.tasks.get_mut(id) else {
                return Ok(None);
            };
            match record.state {
                TaskState::Pending { .. } => {
                    record.state = TaskState::Ready;
                    Ok(Some(record.priority))
                }
                _ => Ok(None),
            }
        }

        async fn push_ready(&self, prio: Priority, id: TestTaskId) -> StoreResult<()> {
            let mut guard = self.inner.lock().await;
            match prio {
                Priority::High => guard.ready_high.push_back(id),
                Priority::Medium => guard.ready_medium.push_back(id),
                Priority::Low => guard.ready_low.push_back(id),
            }
            Ok(())
        }

        async fn pop_ready(&self, prio: Priority) -> StoreResult<Option<TestTaskId>> {
            let mut guard = self.inner.lock().await;
            let id = match prio {
                Priority::High => guard.ready_high.pop_front(),
                Priority::Medium => guard.ready_medium.pop_front(),
                Priority::Low => guard.ready_low.pop_front(),
            };
            Ok(id)
        }

        async fn take_ready(
            &self,
            id: &TestTaskId,
            worker: &str,
        ) -> StoreResult<Option<(P, Priority, u32)>> {
            let mut guard = self.inner.lock().await;
            let Some(record) = guard.tasks.get_mut(id) else {
                return Ok(None);
            };
            if !matches!(record.state, TaskState::Ready) {
                return Ok(None);
            }
            let Some(payload) = record.payload.as_ref() else {
                return Ok(None);
            };

            record.attempt = record.attempt.saturating_add(1);
            let attempt = record.attempt;
            record.state = TaskState::Running {
                worker: worker.to_string(),
                attempt,
            };
            Ok(Some((payload.clone(), record.priority, attempt)))
        }

        async fn put_payload(&self, id: &TestTaskId, payload: P) -> StoreResult<()> {
            let mut guard = self.inner.lock().await;
            if let Some(record) = guard.tasks.get_mut(id) {
                record.payload = Some(payload);
            }
            Ok(())
        }

        async fn schedule(&self, _id: TestTaskId, _not_before_ms: u64) -> StoreResult<()> {
            Ok(())
        }

        async fn promote_scheduled(&self, _now_ms: u64, _limit: usize) -> StoreResult<usize> {
            Ok(0)
        }

        async fn requeue_expired_leases(&self, _now_ms: u64, _limit: usize) -> StoreResult<usize> {
            Ok(0)
        }
    }

    #[async_trait::async_trait]
    impl<P: Clone + Send + 'static, O: Clone + Send + 'static> TaskStore<P, O, TestId>
        for BuggyTakeStore<P, O>
    {
        async fn insert_task(
            &self,
            id: TestTaskId,
            payload: P,
            prio: Priority,
            deps: Vec<TestTaskId>,
        ) -> crate::StoreResult<bool> {
            self.inner.insert_task(id, payload, prio, deps).await
        }

        async fn get_state(
            &self,
            id: &TestTaskId,
        ) -> crate::StoreResult<Option<TaskState<O, TestId>>> {
            self.inner.get_state(id).await
        }

        async fn set_state(
            &self,
            id: &TestTaskId,
            state: TaskState<O, TestId>,
        ) -> crate::StoreResult<()> {
            self.inner.set_state(id, state).await
        }

        async fn get_view(
            &self,
            id: &TestTaskId,
        ) -> crate::StoreResult<Option<(TaskState<O, TestId>, Priority)>> {
            self.inner.get_view(id).await
        }

        async fn dependents_of(&self, dep: &TestTaskId) -> crate::StoreResult<Vec<TestTaskId>> {
            self.inner.dependents_of(dep).await
        }

        async fn dec_remaining_deps(&self, id: &TestTaskId) -> crate::StoreResult<usize> {
            self.inner.dec_remaining_deps(id).await
        }

        async fn try_mark_ready(&self, id: &TestTaskId) -> crate::StoreResult<Option<Priority>> {
            self.inner.try_mark_ready(id).await
        }

        async fn push_ready(&self, prio: Priority, id: TestTaskId) -> crate::StoreResult<()> {
            self.inner.push_ready(prio, id).await
        }

        async fn pop_ready(&self, prio: Priority) -> crate::StoreResult<Option<TestTaskId>> {
            self.inner.pop_ready(prio).await
        }

        async fn take_ready(
            &self,
            _id: &TestTaskId,
            _worker: &str,
        ) -> crate::StoreResult<Option<(P, Priority, u32)>> {
            Ok(None)
        }

        async fn pop_ready_and_take(
            &self,
            prio: Priority,
            worker: &str,
        ) -> crate::StoreResult<Option<(TestTaskId, P, Priority, u32)>> {
            self.inner.pop_ready_and_take(prio, worker).await
        }

        async fn put_payload(&self, id: &TestTaskId, payload: P) -> crate::StoreResult<()> {
            self.inner.put_payload(id, payload).await
        }

        async fn schedule(&self, id: TestTaskId, not_before_ms: u64) -> crate::StoreResult<()> {
            self.inner.schedule(id, not_before_ms).await
        }

        async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> crate::StoreResult<usize> {
            self.inner.promote_scheduled(now_ms, limit).await
        }

        async fn requeue_expired_leases(
            &self,
            now_ms: u64,
            limit: usize,
        ) -> crate::StoreResult<usize> {
            self.inner.requeue_expired_leases(now_ms, limit).await
        }
    }

    #[tokio::test]
    async fn next_ready_picks_high_before_low() {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![],
            )
            .await
            .unwrap();

        let first = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(first.id, b);
        let second = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(second.id, a);
    }

    #[tokio::test]
    async fn next_ready_uses_atomic_store_take_when_available() {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::new(BuggyTakeStore {
            inner: MemoryStore::new(),
        });

        let _ = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();

        assert!(sched.next_ready("w").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn duplicate_dependencies_do_not_block_dependents() {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(UniqueDependentsStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a.clone(), a.clone()],
            )
            .await
            .unwrap();

        let lease = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease.id, a);
        sched.complete(lease, Ok("ok")).await.unwrap();

        let dependent = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(dependent.id, b);
    }

    #[tokio::test]
    async fn dependent_enters_ready_after_all_deps_complete() {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let _a1 = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a1",
                },
                vec![],
            )
            .await
            .unwrap();
        let _a2 = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a2",
                },
                vec![],
            )
            .await
            .unwrap();

        let b = sched
            .submit(
                test_id(3),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![_a1, _a2],
            )
            .await
            .unwrap();

        let t1 = sched.next_ready("w").await.unwrap().unwrap();
        assert_ne!(t1.id, b);
        let t2 = sched.next_ready("w").await.unwrap().unwrap();
        assert_ne!(t2.id, b);
        assert!(sched.next_ready("w").await.unwrap().is_none());

        sched.complete(t1, Ok("ok")).await.unwrap();
        assert!(sched.next_ready("w").await.unwrap().is_none());

        sched.complete(t2, Ok("ok")).await.unwrap();
        assert_eq!(sched.next_ready("w").await.unwrap().unwrap().id, b);
    }

    #[tokio::test]
    async fn failure_propagates_to_dependents() {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await
            .unwrap();

        let lease = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease.id, a);
        sched
            .complete(lease, Err("boom".to_string()))
            .await
            .unwrap();

        let b_state = sched.get(b).await.unwrap().unwrap().state;
        assert!(matches!(b_state, TaskState::Failed { .. }));
    }

    #[tokio::test]
    async fn failure_propagates_transitively() {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::Medium,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await
            .unwrap();
        let c = sched
            .submit(
                test_id(3),
                NewTask {
                    priority: Priority::High,
                    payload: "c",
                },
                vec![b.clone()],
            )
            .await
            .unwrap();

        let lease = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease.id, a);
        sched
            .complete(lease, Err("boom".to_string()))
            .await
            .unwrap();

        assert!(matches!(
            sched.get(b).await.unwrap().unwrap().state,
            TaskState::Failed { .. }
        ));
        assert!(matches!(
            sched.get(c).await.unwrap().unwrap().state,
            TaskState::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn cancel_propagates_to_dependents() {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::Medium,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await
            .unwrap();

        sched.cancel(a.clone()).await.unwrap();
        assert!(matches!(
            sched.get(b).await.unwrap().unwrap().state,
            TaskState::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn cancel_is_terminal_even_if_worker_completes_late() {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();

        let lease = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease.id, a);

        sched.cancel(a.clone()).await.unwrap();
        sched.complete(lease, Ok("late-ok")).await.unwrap();

        assert!(matches!(
            sched.get(a).await.unwrap().unwrap().state,
            TaskState::Cancelled
        ));
    }

    #[tokio::test]
    async fn cancel_after_success_is_noop() {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();

        let lease = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease.id, a);

        sched.complete(lease, Ok("ok")).await.unwrap();
        sched.cancel(a.clone()).await.unwrap();

        assert!(matches!(
            sched.get(a).await.unwrap().unwrap().state,
            TaskState::Succeeded { .. }
        ));
    }

    #[tokio::test]
    async fn memory_store_requeues_task_after_lease_expires() {
        let store = MemoryStore::with_lease(Duration::from_millis(1));
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::new(store);

        let id = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();

        let lease1 = sched.next_ready("w1").await.unwrap().unwrap();
        assert_eq!(lease1.id, id);
        assert_eq!(lease1.attempt, 1);

        sched
            .maintenance_tick_at(super::now_millis().saturating_add(10_000))
            .await
            .unwrap();

        let lease2 = sched.next_ready("w2").await.unwrap().unwrap();
        assert_eq!(lease2.id, id);
        assert_eq!(lease2.attempt, 2);
    }

    #[tokio::test]
    async fn stale_completion_is_ignored_after_task_is_reacquired() {
        let store = MemoryStore::with_lease(Duration::from_millis(1));
        let sched: Scheduler<&'static str, &'static str, TestId> = Scheduler::new(store);

        let id = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();

        let lease1 = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease1.id, id);
        assert_eq!(lease1.attempt, 1);

        // Force the lease to expire and requeue the task.
        sched
            .maintenance_tick_at(super::now_millis().saturating_add(10_000))
            .await
            .unwrap();

        let lease2 = sched.next_ready("w").await.unwrap().unwrap();
        assert_eq!(lease2.id, id);
        assert_eq!(lease2.attempt, 2);

        // Complete in the wrong order: the stale lease completes after the task was
        // already re-leased. The stale completion must not win.
        sched.complete(lease1, Ok("old")).await.unwrap();
        sched.complete(lease2, Ok("new")).await.unwrap();

        let view = sched.get(id).await.unwrap().unwrap();
        match view.state {
            TaskState::Succeeded { output } => assert_eq!(output, "new"),
            other => panic!("unexpected task state: {other:?}"),
        }
    }

    #[tokio::test]
    async fn retry_defers_dependent_failure_until_exhausted() {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::with_config(
            MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::Fixed {
                    max_attempts: 3,
                    delay: Duration::ZERO,
                },
            },
        );

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await
            .unwrap();
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await
            .unwrap();

        for attempt in 1..=3 {
            let lease = sched.next_ready("w").await.unwrap().unwrap();
            assert_eq!(lease.id, a);
            assert_eq!(lease.attempt, attempt);
            sched
                .complete(lease, Err("boom".to_string()))
                .await
                .unwrap();
        }

        assert!(matches!(
            sched.get(a).await.unwrap().unwrap().state,
            TaskState::Failed { .. }
        ));
        assert!(matches!(
            sched.get(b).await.unwrap().unwrap().state,
            TaskState::Failed { .. }
        ));
    }

    #[test]
    fn exponential_retry_delay_saturates_without_shift_overflow() {
        let policy = RetryPolicy::Exponential {
            max_attempts: u32::MAX,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(250),
        };

        assert_eq!(policy.retry_delay(1_000), Some(Duration::from_millis(250)));
    }
}
