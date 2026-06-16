mod config;
mod retry;
mod types;

pub use config::{SchedulerConfig, TaskExecutionPolicy};
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

impl<P, O: Clone, Id> Clone for Scheduler<P, O, Id> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            notify: Arc::clone(&self.notify),
            config: self.config.clone(),
            _phantom: core::marker::PhantomData,
        }
    }
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
    P: Send + 'static,
    Id: Send + Sync + 'static,
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

    #[must_use]
    pub fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    #[must_use]
    pub const fn config(&self) -> &SchedulerConfig {
        &self.config
    }
}

impl<P, O, Id> Scheduler<P, O, Id>
where
    P: Send + 'static,
    O: Clone + Send + 'static,
    Id: Clone + Eq + std::hash::Hash + Send + Sync + 'static,
{
    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn submit(
        &self,
        id: TaskId<Id>,
        task: NewTask<P>,
        deps: Vec<TaskId<Id>>,
    ) -> Result<TaskId<Id>, TaskStoreError> {
        self.submit_with_execution_policy(id, task, deps, self.config.execution_policy())
            .await
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn submit_with_execution_policy(
        &self,
        id: TaskId<Id>,
        task: NewTask<P>,
        deps: Vec<TaskId<Id>>,
        execution_policy: TaskExecutionPolicy,
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
            .insert_task(
                id.clone(),
                task.payload,
                task.priority,
                deps,
                execution_policy,
            )
            .await?;
        if inserted && let Some(priority) = self.store.try_mark_ready(&id).await? {
            self.store.push_ready(priority, id.clone()).await?;
            self.notify.notify_one();
        }

        Ok(id)
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn next_ready(
        &self,
        worker: &str,
    ) -> Result<Option<TaskLease<P, Id>>, TaskStoreError> {
        for prio in [Priority::High, Priority::Medium, Priority::Low] {
            if let Some((id, payload, priority, attempt, execution_policy)) =
                self.store.pop_ready_and_take(prio, worker).await?
            {
                return Ok(Some(TaskLease {
                    id,
                    payload,
                    priority,
                    attempt,
                    worker: worker.to_string(),
                    execution_policy,
                }));
            }
        }

        Ok(None)
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn complete(
        &self,
        lease: TaskLease<P, Id>,
        result: Result<O, String>,
    ) -> Result<bool, TaskStoreError> {
        match result {
            Ok(output) => self.complete_success(lease, output).await,
            Err(error) => self.complete_failure(lease, error).await,
        }
    }

    async fn complete_success(
        &self,
        lease: TaskLease<P, Id>,
        output: O,
    ) -> Result<bool, TaskStoreError> {
        let TaskLease {
            id,
            attempt,
            worker,
            ..
        } = lease;

        let updated = self
            .store
            .set_state_if_running(&id, &worker, attempt, TaskState::Succeeded { output }, None)
            .await?;
        if !updated {
            self.notify.notify_one();
            return Ok(false);
        }

        self.release_dependents(&id).await?;
        self.notify.notify_one();

        Ok(true)
    }

    async fn complete_failure(
        &self,
        lease: TaskLease<P, Id>,
        error: String,
    ) -> Result<bool, TaskStoreError> {
        let Some(delay) = lease.execution_policy.retry.retry_delay(lease.attempt) else {
            return self.fail_completed_lease(lease, error).await;
        };

        match retry_schedule(delay) {
            RetrySchedule::Now => self.retry_now(lease).await,
            RetrySchedule::At(next_ready_at_ms) => {
                self.retry_later(lease, error, next_ready_at_ms).await
            }
        }
    }

    async fn retry_now(&self, lease: TaskLease<P, Id>) -> Result<bool, TaskStoreError> {
        let updated = self
            .store
            .retry_now_if_running(
                lease.id,
                &lease.worker,
                lease.attempt,
                lease.priority,
                lease.payload,
            )
            .await?;
        if !updated {
            self.notify.notify_one();
            return Ok(false);
        }
        self.notify.notify_one();
        Ok(true)
    }

    async fn retry_later(
        &self,
        lease: TaskLease<P, Id>,
        error: String,
        next_ready_at_ms: u64,
    ) -> Result<bool, TaskStoreError> {
        let updated = self
            .store
            .retry_later_if_running(
                lease.id,
                &lease.worker,
                lease.attempt,
                error,
                lease.payload,
                next_ready_at_ms,
            )
            .await?;
        if !updated {
            self.notify.notify_one();
            return Ok(false);
        }
        self.notify.notify_one();
        Ok(true)
    }

    async fn fail_completed_lease(
        &self,
        lease: TaskLease<P, Id>,
        error: String,
    ) -> Result<bool, TaskStoreError> {
        let updated = self
            .store
            .set_state_if_running(
                &lease.id,
                &lease.worker,
                lease.attempt,
                TaskState::Failed {
                    error,
                    caused_by_dep: None,
                },
                None,
            )
            .await?;
        if !updated {
            self.notify.notify_one();
            return Ok(false);
        }
        self.fail_dependents(lease.id, "dependency failed".to_string())
            .await?;
        self.notify.notify_one();
        Ok(true)
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn renew_lease(&self, lease: &TaskLease<P, Id>) -> Result<bool, TaskStoreError> {
        self.store
            .renew_lease(&lease.id, &lease.worker, lease.attempt)
            .await
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
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

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn remove(&self, id: TaskId<Id>) -> Result<(), TaskStoreError> {
        let _ = self.store.remove_task(&id).await?;
        self.notify.notify_one();
        Ok(())
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
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

    /// # Errors
    ///
    /// Returns `TaskStoreError` if underlying store fails.
    pub async fn list(&self) -> Result<Vec<TaskView<O, Id>>, TaskStoreError> {
        self.store.list_views().await.map(|views| {
            views
                .into_iter()
                .map(|(id, state, priority)| TaskView {
                    id,
                    state,
                    priority,
                })
                .collect()
        })
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
    pub async fn maintenance_tick(&self) -> Result<usize, TaskStoreError> {
        self.maintenance_tick_at(now_millis()).await
    }

    /// # Errors
    ///
    /// Returns `TaskStoreError` if the underlying store fails.
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

    async fn release_dependents(&self, id: &TaskId<Id>) -> Result<(), TaskStoreError> {
        for dependent in self.store.dependents_of(id).await? {
            let remaining = self.store.dec_remaining_deps(&dependent).await?;
            if remaining == 0
                && let Some(priority) = self.store.try_mark_ready(&dependent).await?
            {
                self.store.push_ready(priority, dependent).await?;
            }
        }

        Ok(())
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

    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or_default()
}

enum RetrySchedule {
    Now,
    At(u64),
}

fn retry_schedule(delay: Duration) -> RetrySchedule {
    let now_ms = now_millis();
    if delay == Duration::ZERO {
        return RetrySchedule::Now;
    }

    let delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
    let next_ready_at_ms = now_ms.saturating_add(delay_ms);
    RetrySchedule::At(next_ready_at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use crate::Priority;
    use crate::StoreResult;
    use crate::TaskStoreError;
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;
    use tokio::sync::Mutex;

    type TestId = u64;
    type TestTaskId = TaskId<TestId>;

    fn test_id(value: u64) -> TestTaskId {
        TaskId::new(value)
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct EqualSortId(u64);

    impl crate::ReadyQueueSort for EqualSortId {
        fn ready_queue_sort_prefix(&self) -> [u8; 16] {
            [0u8; 16]
        }
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
        execution_policy: TaskExecutionPolicy,
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
            execution_policy: TaskExecutionPolicy,
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
                    execution_policy,
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

        async fn set_state_if_running(
            &self,
            id: &TestTaskId,
            worker: &str,
            attempt: u32,
            state: TaskState<O, TestId>,
            payload: Option<P>,
        ) -> StoreResult<bool> {
            let mut guard = self.inner.lock().await;
            let Some(record) = guard.tasks.get_mut(id) else {
                return Ok(false);
            };
            let TaskState::Running {
                worker: current_worker,
                attempt: current_attempt,
            } = &record.state
            else {
                return Ok(false);
            };
            if current_worker != worker || *current_attempt != attempt {
                return Ok(false);
            }

            if let Some(payload) = payload {
                record.payload = Some(payload);
            }
            record.state = state;
            Ok(true)
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

        async fn list_views(
            &self,
        ) -> StoreResult<Vec<(TestTaskId, TaskState<O, TestId>, Priority)>> {
            let guard = self.inner.lock().await;
            Ok(guard
                .tasks
                .iter()
                .map(|(id, record)| (id.clone(), record.state.clone(), record.priority))
                .collect())
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
        ) -> StoreResult<Option<(P, Priority, u32, TaskExecutionPolicy)>> {
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
            Ok(Some((
                payload.clone(),
                record.priority,
                attempt,
                record.execution_policy.clone(),
            )))
        }

        async fn renew_lease(
            &self,
            id: &TestTaskId,
            worker: &str,
            attempt: u32,
        ) -> StoreResult<bool> {
            let mut guard = self.inner.lock().await;
            let Some(record) = guard.tasks.get_mut(id) else {
                return Ok(false);
            };

            let TaskState::Running {
                worker: current_worker,
                attempt: current_attempt,
            } = &record.state
            else {
                return Ok(false);
            };

            Ok(current_worker == worker && *current_attempt == attempt)
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

        async fn remove_task(&self, id: &TestTaskId) -> StoreResult<bool> {
            let mut guard = self.inner.lock().await;
            let existed = guard.tasks.remove(id).is_some();
            guard.remaining.remove(id);
            guard.dependents.remove(id);
            for dependents in guard.dependents.values_mut() {
                dependents.remove(id);
            }
            guard.ready_high.retain(|queued| queued != id);
            guard.ready_medium.retain(|queued| queued != id);
            guard.ready_low.retain(|queued| queued != id);
            Ok(existed)
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
            execution_policy: TaskExecutionPolicy,
        ) -> crate::StoreResult<bool> {
            self.inner
                .insert_task(id, payload, prio, deps, execution_policy)
                .await
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

        async fn set_state_if_running(
            &self,
            id: &TestTaskId,
            worker: &str,
            attempt: u32,
            state: TaskState<O, TestId>,
            payload: Option<P>,
        ) -> crate::StoreResult<bool> {
            self.inner
                .set_state_if_running(id, worker, attempt, state, payload)
                .await
        }

        async fn get_view(
            &self,
            id: &TestTaskId,
        ) -> crate::StoreResult<Option<(TaskState<O, TestId>, Priority)>> {
            self.inner.get_view(id).await
        }

        async fn list_views(
            &self,
        ) -> crate::StoreResult<Vec<(TestTaskId, TaskState<O, TestId>, Priority)>> {
            self.inner.list_views().await
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
        ) -> crate::StoreResult<Option<(P, Priority, u32, TaskExecutionPolicy)>> {
            Ok(None)
        }

        async fn pop_ready_and_take(
            &self,
            prio: Priority,
            worker: &str,
        ) -> crate::StoreResult<Option<(TestTaskId, P, Priority, u32, TaskExecutionPolicy)>>
        {
            self.inner.pop_ready_and_take(prio, worker).await
        }

        async fn put_payload(&self, id: &TestTaskId, payload: P) -> crate::StoreResult<()> {
            self.inner.put_payload(id, payload).await
        }

        async fn renew_lease(
            &self,
            id: &TestTaskId,
            worker: &str,
            attempt: u32,
        ) -> crate::StoreResult<bool> {
            self.inner.renew_lease(id, worker, attempt).await
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

        async fn remove_task(&self, id: &TestTaskId) -> crate::StoreResult<bool> {
            self.inner.remove_task(id).await
        }
    }

    #[tokio::test]
    async fn next_ready_orders_by_priority_before_sort_prefix() -> StoreResult<()> {
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
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![],
            )
            .await?;

        let first = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first ready task"))?;
        assert_eq!(first.id, b);
        let second = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second ready task"))?;
        assert_eq!(second.id, a);
        Ok(())
    }

    #[tokio::test]
    async fn next_ready_uses_priority_when_memory_sort_prefix_matches() -> StoreResult<()> {
        let sched: Scheduler<&'static str, (), EqualSortId> = Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                TaskId::new(EqualSortId(1)),
                NewTask {
                    priority: Priority::Low,
                    payload: "a",
                },
                vec![],
            )
            .await?;
        let b = sched
            .submit(
                TaskId::new(EqualSortId(2)),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![],
            )
            .await?;

        let first = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first ready task"))?;
        assert_eq!(first.id, b);
        let second = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second ready task"))?;
        assert_eq!(second.id, a);
        Ok(())
    }

    #[tokio::test]
    async fn next_ready_uses_atomic_store_take_when_available() -> StoreResult<()> {
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
            .await?;

        assert!(sched.next_ready("w").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn duplicate_dependencies_do_not_block_dependents() -> StoreResult<()> {
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
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a.clone(), a.clone()],
            )
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, a);
        sched.complete(lease, Ok("ok")).await?;

        let dependent = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected dependent lease"))?;
        assert_eq!(dependent.id, b);
        Ok(())
    }

    #[tokio::test]
    async fn dependent_enters_ready_after_all_deps_complete() -> StoreResult<()> {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let a1 = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a1",
                },
                vec![],
            )
            .await?;
        let a2 = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a2",
                },
                vec![],
            )
            .await?;

        let b = sched
            .submit(
                test_id(3),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a1, a2],
            )
            .await?;

        let t1 = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first ready lease"))?;
        assert_ne!(t1.id, b);
        let t2 = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second ready lease"))?;
        assert_ne!(t2.id, b);
        assert!(sched.next_ready("w").await?.is_none());

        sched.complete(t1, Ok("ok")).await?;
        assert!(sched.next_ready("w").await?.is_none());

        sched.complete(t2, Ok("ok")).await?;
        let next = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected dependent lease"))?;
        assert_eq!(next.id, b);
        Ok(())
    }

    #[tokio::test]
    async fn dependent_submitted_after_completed_dep_is_ready_immediately() -> StoreResult<()> {
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
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready dependency"))?;
        assert_eq!(lease.id, a);
        sched.complete(lease, Ok("ok")).await?;

        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a],
            )
            .await?;

        let ready = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected dependent to be ready"))?;
        assert_eq!(ready.id, b);
        Ok(())
    }

    #[tokio::test]
    async fn dependent_submitted_after_failed_dep_is_failed_immediately() -> StoreResult<()> {
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
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready dependency"))?;
        assert_eq!(lease.id, a);
        sched.complete(lease, Err("boom".to_string())).await?;

        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a],
            )
            .await?;

        let state = sched
            .get(b)
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected failed dependent"))?
            .state;
        assert!(matches!(state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn failure_propagates_to_dependents() -> StoreResult<()> {
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
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, a);
        sched.complete(lease, Err("boom".to_string())).await?;

        let b_state = sched
            .get(b)
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected dependent task view"))?
            .state;
        assert!(matches!(b_state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn failure_propagates_transitively() -> StoreResult<()> {
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
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::Medium,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await?;
        let c = sched
            .submit(
                test_id(3),
                NewTask {
                    priority: Priority::High,
                    payload: "c",
                },
                vec![b.clone()],
            )
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, a);
        sched.complete(lease, Err("boom".to_string())).await?;

        assert!(matches!(
            sched
                .get(b)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for b"))?
                .state,
            TaskState::Failed { .. }
        ));
        assert!(matches!(
            sched
                .get(c)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for c"))?
                .state,
            TaskState::Failed { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn cancel_propagates_to_dependents() -> StoreResult<()> {
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
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::Medium,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await?;

        sched.cancel(a.clone()).await?;
        assert!(matches!(
            sched
                .get(b)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for b"))?
                .state,
            TaskState::Failed { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn cancel_is_terminal_even_if_worker_completes_late() -> StoreResult<()> {
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
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, a);

        sched.cancel(a.clone()).await?;
        sched.complete(lease, Ok("late-ok")).await?;

        assert!(matches!(
            sched
                .get(a)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for a"))?
                .state,
            TaskState::Cancelled
        ));
        Ok(())
    }

    #[tokio::test]
    async fn cancel_after_success_is_noop() -> StoreResult<()> {
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
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, a);

        sched.complete(lease, Ok("ok")).await?;
        sched.cancel(a.clone()).await?;

        assert!(matches!(
            sched
                .get(a)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for a"))?
                .state,
            TaskState::Succeeded { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn cancel_ready_then_resubmit_same_high_id_removes_stale_entry() -> StoreResult<()> {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::High,
                    payload: "a1",
                },
                vec![],
            )
            .await?;
        sched.cancel(a.clone()).await?;

        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![],
            )
            .await?;
        let resubmitted = sched
            .submit(
                a.clone(),
                NewTask {
                    priority: Priority::High,
                    payload: "a2",
                },
                vec![],
            )
            .await?;
        assert_eq!(resubmitted, a);

        let first = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first high task"))?;
        assert_eq!(first.id, a);
        assert_eq!(first.payload, "a2");

        let second = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected resubmitted high task"))?;
        assert_eq!(second.id, b);
        assert_eq!(second.payload, "b");
        assert!(sched.next_ready("w").await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn memory_store_requeues_task_after_lease_expires() -> StoreResult<()> {
        let store = MemoryStore::with_lease(Duration::from_millis(1));
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::with_config(
            store,
            SchedulerConfig {
                lease_duration: Duration::from_millis(1),
                retry: RetryPolicy::None,
            },
        );

        let id = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await?;

        let lease1 = sched
            .next_ready("w1")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease1.id, id);
        assert_eq!(lease1.attempt, 1);

        sched
            .maintenance_tick_at(super::now_millis().saturating_add(10_000))
            .await?;

        let lease2 = sched
            .next_ready("w2")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected requeued lease"))?;
        assert_eq!(lease2.id, id);
        assert_eq!(lease2.attempt, 2);
        Ok(())
    }

    #[tokio::test]
    async fn expired_high_leases_requeue_in_lease_order() -> StoreResult<()> {
        let store = MemoryStore::with_lease(Duration::from_millis(1));
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::with_config(
            store,
            SchedulerConfig {
                lease_duration: Duration::from_millis(1),
                retry: RetryPolicy::None,
            },
        );

        let a = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::High,
                    payload: "a",
                },
                vec![],
            )
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![],
            )
            .await?;

        let first = sched
            .next_ready("w1")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first lease"))?;
        assert_eq!(first.id, a);
        let second = sched
            .next_ready("w2")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second lease"))?;
        assert_eq!(second.id, b);

        sched
            .maintenance_tick_at(super::now_millis().saturating_add(10_000))
            .await?;

        let requeued_first = sched
            .next_ready("w3")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected first requeued lease"))?;
        assert_eq!(requeued_first.id, a);
        let requeued_second = sched
            .next_ready("w4")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected second requeued lease"))?;
        assert_eq!(requeued_second.id, b);
        Ok(())
    }

    #[tokio::test]
    async fn stale_completion_is_ignored_after_task_is_reacquired() -> StoreResult<()> {
        let store = MemoryStore::with_lease(Duration::from_millis(1));
        let sched: Scheduler<&'static str, &'static str, TestId> = Scheduler::with_config(
            store,
            SchedulerConfig {
                lease_duration: Duration::from_millis(1),
                retry: RetryPolicy::None,
            },
        );

        let id = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "a",
                },
                vec![],
            )
            .await?;
        let dependent = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "dependent",
                },
                vec![id.clone()],
            )
            .await?;

        let lease1 = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease1.id, id);
        assert_eq!(lease1.attempt, 1);

        // Force the lease to expire and requeue the task.
        sched
            .maintenance_tick_at(super::now_millis().saturating_add(10_000))
            .await?;

        let lease2 = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected requeued lease"))?;
        assert_eq!(lease2.id, id);
        assert_eq!(lease2.attempt, 2);

        // Complete in the wrong order: the stale lease completes after the task was
        // already re-leased. The stale completion must not win.
        let stale_completed = sched.complete(lease1, Ok("old")).await?;
        assert!(!stale_completed);
        assert!(sched.next_ready("w").await?.is_none());

        let completed = sched.complete(lease2, Ok("new")).await?;
        assert!(completed);

        let view = sched
            .get(id)
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view"))?;
        match view.state {
            TaskState::Succeeded { output } => assert_eq!(output, "new"),
            other => {
                return Err(TaskStoreError::corrupt_msg(format!(
                    "unexpected task state: {other:?}"
                )));
            }
        }
        let dependent_lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected dependent after valid success"))?;
        assert_eq!(dependent_lease.id, dependent);
        Ok(())
    }

    #[tokio::test]
    async fn retry_defers_dependent_failure_until_exhausted() -> StoreResult<()> {
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
            .await?;
        let b = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "b",
                },
                vec![a.clone()],
            )
            .await?;

        for attempt in 1..=3 {
            let lease = sched
                .next_ready("w")
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
            assert_eq!(lease.id, a);
            assert_eq!(lease.attempt, attempt);
            sched.complete(lease, Err("boom".to_string())).await?;
        }

        assert!(matches!(
            sched
                .get(a)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for a"))?
                .state,
            TaskState::Failed { .. }
        ));
        assert!(matches!(
            sched
                .get(b)
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view for b"))?
                .state,
            TaskState::Failed { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn task_execution_policy_overrides_default_retry_behavior() -> StoreResult<()> {
        let sched: Scheduler<&'static str, (), TestId> = Scheduler::with_config(
            MemoryStore::new(),
            SchedulerConfig {
                lease_duration: Duration::from_secs(60),
                retry: RetryPolicy::Exponential {
                    max_attempts: 5,
                    base_delay: Duration::from_secs(1),
                    max_delay: Duration::from_secs(30),
                },
            },
        );
        let execution_policy = TaskExecutionPolicy {
            lease_duration: Duration::from_secs(600),
            retry: RetryPolicy::None,
        };

        let id = sched
            .submit_with_execution_policy(
                test_id(9),
                NewTask {
                    priority: Priority::Medium,
                    payload: "custom",
                },
                vec![],
                execution_policy.clone(),
            )
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, id);
        assert_eq!(lease.execution_policy, execution_policy);
        sched.complete(lease, Err("boom".to_string())).await?;

        let view = sched
            .get(id)
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view"))?;
        assert!(matches!(view.state, TaskState::Failed { .. }));
        Ok(())
    }

    #[tokio::test]
    async fn resubmit_failed_task_requeues_same_id() -> StoreResult<()> {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let id = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "first",
                },
                vec![],
            )
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready lease"))?;
        assert_eq!(lease.id, id);
        sched.complete(lease, Err("boom".to_string())).await?;

        assert!(matches!(
            sched
                .get(id.clone())
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected task view"))?
                .state,
            TaskState::Failed { .. }
        ));

        let id2 = sched
            .submit(
                id.clone(),
                NewTask {
                    priority: Priority::High,
                    payload: "second",
                },
                vec![],
            )
            .await?;
        assert_eq!(id2, id);

        let lease2 = sched
            .next_ready("w2")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected requeued lease"))?;
        assert_eq!(lease2.id, id);
        assert_eq!(lease2.priority, Priority::High);
        assert_eq!(lease2.payload, "second");
        assert_eq!(lease2.attempt, 1);
        Ok(())
    }

    #[tokio::test]
    async fn resubmit_failed_task_rebuilds_dependency_edges() -> StoreResult<()> {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let failed_dep = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Medium,
                    payload: "failed-dep",
                },
                vec![],
            )
            .await?;
        let task = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "first",
                },
                vec![failed_dep.clone()],
            )
            .await?;

        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected ready dependency"))?;
        assert_eq!(lease.id, failed_dep);
        sched.complete(lease, Err("boom".to_string())).await?;
        assert!(matches!(
            sched
                .get(task.clone())
                .await?
                .ok_or_else(|| TaskStoreError::corrupt_msg("expected failed task"))?
                .state,
            TaskState::Failed { .. }
        ));

        let new_dep = sched
            .submit(
                test_id(3),
                NewTask {
                    priority: Priority::Medium,
                    payload: "new-dep",
                },
                vec![],
            )
            .await?;
        let resubmitted = sched
            .submit(
                task.clone(),
                NewTask {
                    priority: Priority::High,
                    payload: "second",
                },
                vec![new_dep.clone()],
            )
            .await?;
        assert_eq!(resubmitted, task);
        let new_dep_lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected new dependency"))?;
        assert_eq!(new_dep_lease.id, new_dep);
        assert!(sched.next_ready("w").await?.is_none());

        sched.complete(new_dep_lease, Ok("ok")).await?;

        let ready = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected resubmitted task"))?;
        assert_eq!(ready.id, task);
        assert_eq!(ready.payload, "second");
        Ok(())
    }

    #[tokio::test]
    async fn resubmit_cancelled_task_removes_stale_dependency_edges() -> StoreResult<()> {
        let sched: Scheduler<&'static str, &'static str, TestId> =
            Scheduler::new(MemoryStore::new());

        let old_dep = sched
            .submit(
                test_id(1),
                NewTask {
                    priority: Priority::Low,
                    payload: "old-dep",
                },
                vec![],
            )
            .await?;
        let task = sched
            .submit(
                test_id(2),
                NewTask {
                    priority: Priority::High,
                    payload: "first",
                },
                vec![old_dep.clone()],
            )
            .await?;
        sched.cancel(task.clone()).await?;

        let new_dep = sched
            .submit(
                test_id(3),
                NewTask {
                    priority: Priority::Low,
                    payload: "new-dep",
                },
                vec![],
            )
            .await?;
        sched
            .submit(
                task.clone(),
                NewTask {
                    priority: Priority::High,
                    payload: "second",
                },
                vec![new_dep.clone()],
            )
            .await?;

        let old_lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected old dependency"))?;
        assert_eq!(old_lease.id, old_dep);
        sched.complete(old_lease, Ok("old-ok")).await?;
        let new_dep_lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected new dependency"))?;
        assert_eq!(new_dep_lease.id, new_dep);
        assert!(sched.next_ready("w").await?.is_none());

        sched.complete(new_dep_lease, Ok("new-ok")).await?;

        let ready = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| TaskStoreError::corrupt_msg("expected resubmitted task"))?;
        assert_eq!(ready.id, task);
        assert_eq!(ready.payload, "second");
        Ok(())
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
