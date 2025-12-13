use crate::{Priority, TaskId, TaskKind, TaskState, TaskStore};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub lease_duration: Duration,
    pub retry: RetryPolicy,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum RetryPolicy {
    None,
    Fixed {
        max_attempts: u32,
        delay: Duration,
    },
    Exponential {
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
    },
}

impl RetryPolicy {
    fn retry_delay(&self, attempt: u32) -> Option<Duration> {
        match self {
            RetryPolicy::None => None,
            RetryPolicy::Fixed {
                max_attempts,
                delay,
            } => {
                if attempt >= *max_attempts {
                    None
                } else {
                    Some(*delay)
                }
            }
            RetryPolicy::Exponential {
                max_attempts,
                base_delay,
                max_delay,
            } => {
                if attempt >= *max_attempts {
                    return None;
                }

                let exponent = attempt.saturating_sub(1).min(31);
                let base_ms = base_delay.as_millis().min(u64::MAX as u128) as u64;
                let max_ms = max_delay.as_millis().min(u64::MAX as u128) as u64;
                let factor = 1u64 << exponent;
                let delay_ms = base_ms.saturating_mul(factor).min(max_ms);
                Some(Duration::from_millis(delay_ms))
            }
        }
    }
}

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
    pub attempt: u32,
    pub worker: String,
}

#[derive(Clone)]
pub struct TaskView<O> {
    pub id: TaskId,
    pub state: TaskState<O>,
    pub kind: TaskKind,
    pub priority: Priority,
}

pub struct Scheduler<P, O: Clone> {
    store: Arc<dyn TaskStore<P, O>>,
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

impl<P, O: Clone> Scheduler<P, O> {
    pub fn new<S>(store: S) -> Self
    where
        S: TaskStore<P, O> + 'static,
    {
        Self::with_config(store, SchedulerConfig::default())
    }

    pub fn with_config<S>(store: S, config: SchedulerConfig) -> Self
    where
        S: TaskStore<P, O> + 'static,
    {
        Self::from_arc_with_config(Arc::new(store), config)
    }

    pub fn from_arc(store: Arc<dyn TaskStore<P, O>>) -> Self {
        Self::from_arc_with_config(store, SchedulerConfig::default())
    }

    pub fn from_arc_with_config(store: Arc<dyn TaskStore<P, O>>, config: SchedulerConfig) -> Self {
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

impl<P: Send + 'static, O: Clone + Send + 'static> Scheduler<P, O> {
    pub async fn submit(&self, task: NewTask<P>, deps: Vec<TaskId>) -> TaskId {
        let id = TaskId::new();
        self.store
            .insert_task(id, task.kind, task.payload, task.priority, deps)
            .await;
        if let Some(priority) = self.store.try_mark_ready(&id).await {
            self.store.push_ready(priority, id).await;
            self.notify.notify_one();
        }
        id
    }

    pub async fn next_ready(&self, worker: &str) -> Option<TaskLease<P>> {
        for prio in [Priority::High, Priority::Medium, Priority::Low] {
            if let Some((id, payload, kind, priority, attempt)) =
                self.store.pop_ready_and_take(prio, worker).await
            {
                return Some(TaskLease {
                    id,
                    payload,
                    kind,
                    priority,
                    attempt,
                    worker: worker.to_string(),
                });
            }
        }

        None
    }

    pub async fn complete(&self, lease: TaskLease<P>, result: Result<O, String>) {
        let TaskLease {
            id,
            payload,
            kind: _,
            priority,
            attempt,
            worker,
        } = lease;

        let Some(current) = self.store.get_state(&id).await else {
            self.notify.notify_one();
            return;
        };

        if matches!(
            current,
            TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. }
        ) {
            self.notify.notify_one();
            return;
        }

        let TaskState::Running {
            worker: current_worker,
        } = current
        else {
            self.notify.notify_one();
            return;
        };

        if current_worker != worker {
            self.notify.notify_one();
            return;
        }

        match result {
            Ok(output) => {
                self.store
                    .set_state(&id, TaskState::Succeeded { output })
                    .await;
            }
            Err(error) => {
                if let Some(delay) = self.config.retry.retry_delay(attempt) {
                    self.store.put_payload(&id, payload).await;

                    if delay == Duration::ZERO {
                        self.store.set_state(&id, TaskState::Ready).await;
                        self.store.push_ready(priority, id).await;
                        self.notify.notify_one();
                        return;
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
                        .await;
                    self.store.schedule(id, next_ready_at_ms).await;
                    self.notify.notify_one();
                    return;
                }

                self.store
                    .set_state(
                        &id,
                        TaskState::Failed {
                            error: error.clone(),
                            caused_by_dep: None,
                        },
                    )
                    .await;
                self.fail_dependents(id, "dependency failed".to_string())
                    .await;
                self.notify.notify_one();
                return;
            }
        }

        for dependent in self.store.dependents_of(&id).await {
            let remaining = self.store.dec_remaining_deps(&dependent).await;
            if remaining == 0
                && let Some(priority) = self.store.try_mark_ready(&dependent).await
            {
                self.store.push_ready(priority, dependent).await;
            }
        }

        self.notify.notify_one();
    }

    pub async fn cancel(&self, id: TaskId) {
        let Some(current) = self.store.get_state(&id).await else {
            self.notify.notify_one();
            return;
        };

        if matches!(
            current,
            TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. }
        ) {
            self.notify.notify_one();
            return;
        }

        self.store.set_state(&id, TaskState::Cancelled).await;
        self.fail_dependents(id, "dependency cancelled".to_string())
            .await;
        self.notify.notify_one();
    }

    pub async fn get(&self, id: TaskId) -> Option<TaskView<O>> {
        let (state, kind, priority) = self.store.get_view(&id).await?;
        Some(TaskView {
            id,
            state,
            kind,
            priority,
        })
    }

    pub async fn maintenance_tick(&self) -> usize {
        self.maintenance_tick_at(now_millis()).await
    }

    pub async fn maintenance_tick_at(&self, now_ms: u64) -> usize {
        let moved_scheduled = self
            .store
            .promote_scheduled(now_ms, MAINTENANCE_TICK_LIMIT)
            .await;
        let moved_leases = self
            .store
            .requeue_expired_leases(now_ms, MAINTENANCE_TICK_LIMIT)
            .await;
        let moved = moved_scheduled + moved_leases;
        if moved > 0 {
            self.notify.notify_one();
        }
        moved
    }

    async fn fail_dependents(&self, root: TaskId, error: String) {
        let mut queue: VecDeque<TaskId> = self.store.dependents_of(&root).await.into();
        let mut visited = HashSet::new();

        while let Some(id) = queue.pop_front() {
            if !visited.insert(id) {
                continue;
            }

            let state = self.store.get_state(&id).await;
            if !matches!(
                state,
                Some(TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. })
            ) {
                self.store
                    .set_state(
                        &id,
                        TaskState::Failed {
                            error: error.clone(),
                            caused_by_dep: Some(root),
                        },
                    )
                    .await;
            }

            for dependent in self.store.dependents_of(&id).await {
                queue.push_back(dependent);
            }
        }
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
    use std::time::Duration;

    struct BuggyTakeStore<P, O> {
        inner: MemoryStore<P, O>,
    }

    #[async_trait::async_trait]
    impl<P: Clone + Send + 'static, O: Clone + Send + 'static> TaskStore<P, O>
        for BuggyTakeStore<P, O>
    {
        async fn insert_task(
            &self,
            id: TaskId,
            kind: TaskKind,
            payload: P,
            prio: Priority,
            deps: Vec<TaskId>,
        ) {
            self.inner.insert_task(id, kind, payload, prio, deps).await;
        }

        async fn get_state(&self, id: &TaskId) -> Option<TaskState<O>> {
            self.inner.get_state(id).await
        }

        async fn set_state(&self, id: &TaskId, state: TaskState<O>) {
            self.inner.set_state(id, state).await;
        }

        async fn get_view(&self, id: &TaskId) -> Option<(TaskState<O>, TaskKind, Priority)> {
            self.inner.get_view(id).await
        }

        async fn dependents_of(&self, dep: &TaskId) -> Vec<TaskId> {
            self.inner.dependents_of(dep).await
        }

        async fn dec_remaining_deps(&self, id: &TaskId) -> usize {
            self.inner.dec_remaining_deps(id).await
        }

        async fn try_mark_ready(&self, id: &TaskId) -> Option<Priority> {
            self.inner.try_mark_ready(id).await
        }

        async fn push_ready(&self, prio: Priority, id: TaskId) {
            self.inner.push_ready(prio, id).await;
        }

        async fn pop_ready(&self, prio: Priority) -> Option<TaskId> {
            self.inner.pop_ready(prio).await
        }

        async fn take_ready(
            &self,
            _id: &TaskId,
            _worker: &str,
        ) -> Option<(P, TaskKind, Priority, u32)> {
            None
        }

        async fn pop_ready_and_take(
            &self,
            prio: Priority,
            worker: &str,
        ) -> Option<(TaskId, P, TaskKind, Priority, u32)> {
            self.inner.pop_ready_and_take(prio, worker).await
        }

        async fn put_payload(&self, id: &TaskId, payload: P) {
            self.inner.put_payload(id, payload).await;
        }

        async fn schedule(&self, id: TaskId, not_before_ms: u64) {
            self.inner.schedule(id, not_before_ms).await;
        }

        async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> usize {
            self.inner.promote_scheduled(now_ms, limit).await
        }

        async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> usize {
            self.inner.requeue_expired_leases(now_ms, limit).await
        }
    }

    #[tokio::test]
    async fn next_ready_picks_high_before_low() {
        let sched: Scheduler<&'static str, ()> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await;
        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::Aggregation,
                    priority: Priority::High,
                    payload: "b",
                },
                vec![],
            )
            .await;

        let first = sched.next_ready("w").await.unwrap();
        assert_eq!(first.id, b);
        let second = sched.next_ready("w").await.unwrap();
        assert_eq!(second.id, a);
    }

    #[tokio::test]
    async fn next_ready_uses_atomic_store_take_when_available() {
        let sched: Scheduler<&'static str, ()> = Scheduler::new(BuggyTakeStore {
            inner: MemoryStore::new(),
        });

        let _ = sched
            .submit(
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await;

        assert!(sched.next_ready("w").await.is_some());
    }

    #[tokio::test]
    async fn dependent_enters_ready_after_all_deps_complete() {
        let sched: Scheduler<&'static str, &'static str> = Scheduler::new(MemoryStore::new());

        let _a1 = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "a1",
                },
                vec![],
            )
            .await;
        let _a2 = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "a2",
                },
                vec![],
            )
            .await;

        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::Aggregation,
                    priority: Priority::High,
                    payload: "b",
                },
                vec![_a1, _a2],
            )
            .await;

        let t1 = sched.next_ready("w").await.unwrap();
        assert_ne!(t1.id, b);
        let t2 = sched.next_ready("w").await.unwrap();
        assert_ne!(t2.id, b);
        assert!(sched.next_ready("w").await.is_none());

        sched.complete(t1, Ok("ok")).await;
        assert!(sched.next_ready("w").await.is_none());

        sched.complete(t2, Ok("ok")).await;
        assert_eq!(sched.next_ready("w").await.unwrap().id, b);
    }

    #[tokio::test]
    async fn failure_propagates_to_dependents() {
        let sched: Scheduler<&'static str, ()> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await;
        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::Aggregation,
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a],
            )
            .await;

        let lease = sched.next_ready("w").await.unwrap();
        assert_eq!(lease.id, a);
        sched.complete(lease, Err("boom".to_string())).await;

        let b_state = sched.get(b).await.unwrap().state;
        assert!(matches!(b_state, TaskState::Failed { .. }));
    }

    #[tokio::test]
    async fn failure_propagates_transitively() {
        let sched: Scheduler<&'static str, ()> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await;
        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "b",
                },
                vec![a],
            )
            .await;
        let c = sched
            .submit(
                NewTask {
                    kind: TaskKind::Aggregation,
                    priority: Priority::High,
                    payload: "c",
                },
                vec![b],
            )
            .await;

        let lease = sched.next_ready("w").await.unwrap();
        assert_eq!(lease.id, a);
        sched.complete(lease, Err("boom".to_string())).await;

        assert!(matches!(
            sched.get(b).await.unwrap().state,
            TaskState::Failed { .. }
        ));
        assert!(matches!(
            sched.get(c).await.unwrap().state,
            TaskState::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn cancel_propagates_to_dependents() {
        let sched: Scheduler<&'static str, ()> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await;
        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "b",
                },
                vec![a],
            )
            .await;

        sched.cancel(a).await;
        assert!(matches!(
            sched.get(b).await.unwrap().state,
            TaskState::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn cancel_is_terminal_even_if_worker_completes_late() {
        let sched: Scheduler<&'static str, &'static str> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await;

        let lease = sched.next_ready("w").await.unwrap();
        assert_eq!(lease.id, a);

        sched.cancel(a).await;
        sched.complete(lease, Ok("late-ok")).await;

        assert!(matches!(
            sched.get(a).await.unwrap().state,
            TaskState::Cancelled
        ));
    }

    #[tokio::test]
    async fn cancel_after_success_is_noop() {
        let sched: Scheduler<&'static str, &'static str> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await;

        let lease = sched.next_ready("w").await.unwrap();
        assert_eq!(lease.id, a);

        sched.complete(lease, Ok("ok")).await;
        sched.cancel(a).await;

        assert!(matches!(
            sched.get(a).await.unwrap().state,
            TaskState::Succeeded { .. }
        ));
    }

    #[tokio::test]
    async fn memory_store_requeues_task_after_lease_expires() {
        let store = MemoryStore::with_lease(Duration::from_millis(1));
        let sched: Scheduler<&'static str, ()> = Scheduler::new(store);

        let id = sched
            .submit(
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await;

        let lease1 = sched.next_ready("w1").await.unwrap();
        assert_eq!(lease1.id, id);
        assert_eq!(lease1.attempt, 1);

        sched
            .maintenance_tick_at(super::now_millis().saturating_add(10_000))
            .await;

        let lease2 = sched.next_ready("w2").await.unwrap();
        assert_eq!(lease2.id, id);
        assert_eq!(lease2.attempt, 2);
    }

    #[tokio::test]
    async fn retry_defers_dependent_failure_until_exhausted() {
        let sched: Scheduler<&'static str, ()> = Scheduler::with_config(
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
                NewTask {
                    kind: TaskKind::Preflight,
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await;
        let b = sched
            .submit(
                NewTask {
                    kind: TaskKind::BatchProof,
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a],
            )
            .await;

        for attempt in 1..=3 {
            let lease = sched.next_ready("w").await.unwrap();
            assert_eq!(lease.id, a);
            assert_eq!(lease.attempt, attempt);
            sched.complete(lease, Err("boom".to_string())).await;
        }

        assert!(matches!(
            sched.get(a).await.unwrap().state,
            TaskState::Failed { .. }
        ));
        assert!(matches!(
            sched.get(b).await.unwrap().state,
            TaskState::Failed { .. }
        ));
    }
}
