use crate::ready_sort::insert_ready_sorted;
use crate::{Priority, ReadyQueueSort, TaskExecutionPolicy, TaskId, TaskState, TaskStateKind};
use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::error::Error;
use std::hash::Hash;
use std::time::Duration;
use tokio::sync::Mutex;

pub type StoreResult<T> = Result<T, TaskStoreError>;

#[derive(Debug)]
pub enum TaskStoreError {
    /// Underlying storage/backend failure (e.g. Redis unavailable, network errors, timeouts).
    Backend(Box<dyn Error + Send + Sync>),
    /// Data is missing or cannot be decoded (e.g. schema mismatch, corrupt payload).
    CorruptData(Box<dyn Error + Send + Sync>),
}

impl TaskStoreError {
    pub fn backend<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Backend(Box::new(err))
    }

    pub fn corrupt_data<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::CorruptData(Box::new(err))
    }

    pub fn corrupt_msg(msg: impl Into<String>) -> Self {
        Self::CorruptData(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            msg.into(),
        )))
    }
}

impl std::fmt::Display for TaskStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStoreError::Backend(err) => write!(f, "task store backend error: {err}"),
            TaskStoreError::CorruptData(err) => write!(f, "task store corrupt data: {err}"),
        }
    }
}

impl Error for TaskStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            TaskStoreError::Backend(err) | TaskStoreError::CorruptData(err) => Some(err.as_ref()),
        }
    }
}

#[async_trait]
pub trait TaskStore<P, O, Id>: Send + Sync
where
    P: Send + 'static,
    O: Clone,
    Id: Send + Sync + 'static,
{
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool>;
    async fn get_state(&self, id: &TaskId<Id>) -> StoreResult<Option<TaskState<O, Id>>>;
    async fn set_state(&self, id: &TaskId<Id>, state: TaskState<O, Id>) -> StoreResult<()>;
    async fn set_state_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        state: TaskState<O, Id>,
        payload: Option<P>,
    ) -> StoreResult<bool>;
    async fn retry_now_if_running(
        &self,
        id: TaskId<Id>,
        worker: &str,
        attempt: u32,
        priority: Priority,
        payload: P,
    ) -> StoreResult<bool> {
        let updated = self
            .set_state_if_running(&id, worker, attempt, TaskState::Ready, Some(payload))
            .await?;
        if updated {
            self.push_ready(priority, id).await?;
        }
        Ok(updated)
    }
    async fn retry_later_if_running(
        &self,
        id: TaskId<Id>,
        worker: &str,
        attempt: u32,
        error: String,
        payload: P,
        next_ready_at_ms: u64,
    ) -> StoreResult<bool> {
        let updated = self
            .set_state_if_running(
                &id,
                worker,
                attempt,
                TaskState::Retrying {
                    error,
                    attempt,
                    next_ready_at_ms,
                },
                Some(payload),
            )
            .await?;
        if updated {
            self.schedule(id, next_ready_at_ms).await?;
        }
        Ok(updated)
    }
    async fn get_view(&self, id: &TaskId<Id>) -> StoreResult<Option<(TaskState<O, Id>, Priority)>>;
    async fn list_view_states(&self) -> StoreResult<Vec<(TaskId<Id>, TaskStateKind, Priority)>>;
    async fn dependents_of(&self, dep: &TaskId<Id>) -> StoreResult<Vec<TaskId<Id>>>;
    async fn dec_remaining_deps(&self, id: &TaskId<Id>) -> StoreResult<usize>;
    async fn try_mark_ready(&self, id: &TaskId<Id>) -> StoreResult<Option<Priority>>;
    async fn push_ready(&self, prio: Priority, id: TaskId<Id>) -> StoreResult<()>;
    async fn pop_ready(&self, prio: Priority) -> StoreResult<Option<TaskId<Id>>>;
    async fn take_ready(
        &self,
        id: &TaskId<Id>,
        worker: &str,
    ) -> StoreResult<Option<(P, Priority, u32, TaskExecutionPolicy)>>;
    async fn renew_lease(&self, id: &TaskId<Id>, worker: &str, attempt: u32) -> StoreResult<bool>;

    async fn pop_ready_and_take(
        &self,
        prio: Priority,
        worker: &str,
    ) -> StoreResult<Option<(TaskId<Id>, P, Priority, u32, TaskExecutionPolicy)>> {
        loop {
            let Some(id) = self.pop_ready(prio).await? else {
                return Ok(None);
            };
            if let Some((payload, priority, attempt, execution_policy)) =
                self.take_ready(&id, worker).await?
            {
                return Ok(Some((id, payload, priority, attempt, execution_policy)));
            }
        }
    }
    async fn put_payload(&self, id: &TaskId<Id>, payload: P) -> StoreResult<()>;
    async fn schedule(&self, id: TaskId<Id>, not_before_ms: u64) -> StoreResult<()>;
    async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> StoreResult<usize>;
    async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> StoreResult<usize>;
    async fn remove_task(&self, id: &TaskId<Id>) -> StoreResult<bool>;
}

pub struct MemoryStore<P, O, Id> {
    inner: Mutex<Inner<P, O, Id>>,
    lease: Duration,
}

struct Inner<P, O, Id> {
    tasks: HashMap<TaskId<Id>, TaskRecord<P, O, Id>>,
    dependents: HashMap<TaskId<Id>, Vec<TaskId<Id>>>,
    remaining: HashMap<TaskId<Id>, usize>,
    ready_high: VecDeque<TaskId<Id>>,
    ready_med: VecDeque<TaskId<Id>>,
    ready_low: VecDeque<TaskId<Id>>,
    scheduled: BTreeMap<u64, VecDeque<TaskId<Id>>>,
    next_sequence: u64,
}

struct TaskRecord<P, O, Id> {
    payload: Option<P>,
    state: TaskState<O, Id>,
    priority: Priority,
    attempt: u32,
    lease_until_ms: Option<u64>,
    lease_sequence: u64,
    execution_policy: TaskExecutionPolicy,
}

type TakenReadyTask<P> = (P, Priority, u32, TaskExecutionPolicy);

enum DependencyInsertState<Id> {
    Pending {
        remaining: usize,
        unresolved_deps: Vec<TaskId<Id>>,
    },
    Failed,
    Cancelled,
}

fn resolve_dependency_insert_state<P, O, Id>(
    tasks: &HashMap<TaskId<Id>, TaskRecord<P, O, Id>>,
    deps: Vec<TaskId<Id>>,
) -> DependencyInsertState<Id>
where
    Id: Clone + Eq + Hash,
{
    let mut unresolved_deps = Vec::with_capacity(deps.len());

    for dep in deps {
        match tasks.get(&dep).map(|record| &record.state) {
            Some(TaskState::Succeeded { .. }) => {}
            Some(TaskState::Failed { .. }) => return DependencyInsertState::Failed,
            Some(TaskState::Cancelled) => return DependencyInsertState::Cancelled,
            _ => unresolved_deps.push(dep),
        }
    }

    DependencyInsertState::Pending {
        remaining: unresolved_deps.len(),
        unresolved_deps,
    }
}

impl<P, O, Id> MemoryStore<P, O, Id> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_lease(Duration::from_secs(60))
    }

    #[must_use]
    pub fn with_lease(lease: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                tasks: HashMap::new(),
                dependents: HashMap::new(),
                remaining: HashMap::new(),
                ready_high: VecDeque::new(),
                ready_med: VecDeque::new(),
                ready_low: VecDeque::new(),
                scheduled: BTreeMap::new(),
                next_sequence: 0,
            }),
            lease,
        }
    }
}

impl<P, O, Id> Default for MemoryStore<P, O, Id> {
    fn default() -> Self {
        Self::new()
    }
}

fn retain_task_id<Id>(queue: &mut VecDeque<TaskId<Id>>, target: &TaskId<Id>)
where
    Id: Clone + Eq + Hash,
{
    queue.retain(|queued| queued != target);
}

const fn next_sequence<P, O, Id>(inner: &mut Inner<P, O, Id>) -> u64 {
    let sequence = inner.next_sequence;
    inner.next_sequence = inner.next_sequence.saturating_add(1);
    sequence
}

fn remove_queue_memberships<P, O, Id>(inner: &mut Inner<P, O, Id>, id: &TaskId<Id>)
where
    Id: Clone + Eq + Hash,
{
    retain_task_id(&mut inner.ready_high, id);
    retain_task_id(&mut inner.ready_med, id);
    retain_task_id(&mut inner.ready_low, id);

    let scheduled_keys = inner.scheduled.keys().copied().collect::<Vec<_>>();
    for key in scheduled_keys {
        if let Some(queue) = inner.scheduled.get_mut(&key) {
            retain_task_id(queue, id);
            if queue.is_empty() {
                inner.scheduled.remove(&key);
            }
        }
    }
}

fn push_ready_locked<P, O, Id>(inner: &mut Inner<P, O, Id>, priority: Priority, id: TaskId<Id>)
where
    Id: ReadyQueueSort,
{
    match priority {
        Priority::High => insert_ready_sorted(&mut inner.ready_high, id),
        Priority::Medium => insert_ready_sorted(&mut inner.ready_med, id),
        Priority::Low => insert_ready_sorted(&mut inner.ready_low, id),
    }
}

fn take_ready_locked<P, O, Id>(
    inner: &mut Inner<P, O, Id>,
    lease: Duration,
    id: &TaskId<Id>,
    worker: &str,
) -> StoreResult<Option<TakenReadyTask<P>>>
where
    P: Clone,
    Id: Clone + Eq + Hash,
{
    let (payload, now_ms) = {
        let Some(record) = inner.tasks.get_mut(id) else {
            return Ok(None);
        };
        if !matches!(record.state, TaskState::Ready) {
            return Ok(None);
        }
        let Some(payload) = record.payload.as_ref() else {
            return Ok(None);
        };
        let payload = payload.clone();
        let now_ms = now_millis();
        (payload, now_ms)
    };

    let lease_sequence = next_sequence(inner);
    let record = inner
        .tasks
        .get_mut(id)
        .ok_or_else(|| TaskStoreError::corrupt_msg("ready task disappeared"))?;
    record.attempt = record.attempt.saturating_add(1);
    let attempt = record.attempt;
    record.state = TaskState::Running {
        worker: worker.to_string(),
        attempt,
    };
    record.lease_sequence = lease_sequence;
    let lease_duration = record.execution_policy.lease_duration.max(lease);
    let lease_ms =
        u64::try_from(lease_duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX);
    record.lease_until_ms = Some(now_ms.saturating_add(lease_ms));
    Ok(Some((
        payload,
        record.priority,
        attempt,
        record.execution_policy.clone(),
    )))
}

fn remove_dependent_edges<Id>(
    dependents: &mut HashMap<TaskId<Id>, Vec<TaskId<Id>>>,
    target: &TaskId<Id>,
) where
    Id: Clone + Eq + Hash,
{
    dependents.retain(|_, ids| {
        ids.retain(|dependent| dependent != target);
        !ids.is_empty()
    });
}

#[async_trait]
impl<P, O, Id> TaskStore<P, O, Id> for MemoryStore<P, O, Id>
where
    P: Clone + Send + 'static,
    O: Clone + Send + 'static,
    Id: ReadyQueueSort,
{
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let dependency_state = resolve_dependency_insert_state(&g.tasks, deps);
        let should_reset = matches!(
            g.tasks.get(&id).map(|record| &record.state),
            Some(TaskState::Failed { .. } | TaskState::Cancelled)
        );
        if should_reset {
            let (remaining, next_state) = match &dependency_state {
                DependencyInsertState::Pending { remaining, .. } => {
                    (*remaining, TaskState::pending(*remaining))
                }
                DependencyInsertState::Failed => (
                    0,
                    TaskState::Failed {
                        error: "dependency failed".to_string(),
                        caused_by_dep: None,
                    },
                ),
                DependencyInsertState::Cancelled => (0, TaskState::Cancelled),
            };
            remove_dependent_edges(&mut g.dependents, &id);
            if let DependencyInsertState::Pending {
                unresolved_deps, ..
            } = dependency_state
            {
                for dep in unresolved_deps {
                    g.dependents.entry(dep).or_default().push(id.clone());
                }
            }
            g.remaining.insert(id.clone(), remaining);
            remove_queue_memberships(&mut g, &id);
            if let Some(existing) = g.tasks.get_mut(&id) {
                existing.payload = Some(payload);
                existing.priority = prio;
                existing.attempt = 0;
                existing.lease_until_ms = None;
                existing.lease_sequence = 0;
                existing.state = next_state;
                existing.execution_policy = execution_policy;
            }
            return Ok(true);
        }
        if g.tasks.contains_key(&id) {
            return Ok(false);
        }
        let (remaining, next_state) = match dependency_state {
            DependencyInsertState::Pending {
                remaining,
                unresolved_deps,
            } => {
                for dep in unresolved_deps {
                    g.dependents.entry(dep).or_default().push(id.clone());
                }
                (remaining, TaskState::pending(remaining))
            }
            DependencyInsertState::Failed => (
                0,
                TaskState::Failed {
                    error: "dependency failed".to_string(),
                    caused_by_dep: None,
                },
            ),
            DependencyInsertState::Cancelled => (0, TaskState::Cancelled),
        };
        g.remaining.insert(id.clone(), remaining);
        g.tasks.insert(
            id,
            TaskRecord {
                payload: Some(payload),
                state: next_state,
                priority: prio,
                attempt: 0,
                lease_until_ms: None,
                lease_sequence: 0,
                execution_policy,
            },
        );

        Ok(true)
    }

    async fn get_state(&self, id: &TaskId<Id>) -> StoreResult<Option<TaskState<O, Id>>> {
        let g = self.inner.lock().await;
        Ok(g.tasks.get(id).map(|r| &r.state).cloned())
    }

    async fn set_state(&self, id: &TaskId<Id>, state: TaskState<O, Id>) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        if !matches!(state, TaskState::Running { .. }) {
            remove_queue_memberships(&mut g, id);
        }
        if let Some(r) = g.tasks.get_mut(id) {
            r.state = state;
            if !matches!(r.state, TaskState::Running { .. }) {
                r.lease_until_ms = None;
            }
        }

        Ok(())
    }

    async fn set_state_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        state: TaskState<O, Id>,
        payload: Option<P>,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let Some(record) = g.tasks.get_mut(id) else {
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
        if !matches!(record.state, TaskState::Running { .. }) {
            record.lease_until_ms = None;
        }
        Ok(true)
    }

    async fn retry_now_if_running(
        &self,
        id: TaskId<Id>,
        worker: &str,
        attempt: u32,
        priority: Priority,
        payload: P,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let Some(record) = g.tasks.get_mut(&id) else {
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

        debug_assert_eq!(record.priority, priority);
        record.payload = Some(payload);
        record.state = TaskState::Ready;
        record.lease_until_ms = None;
        push_ready_locked(&mut g, priority, id);
        Ok(true)
    }

    async fn retry_later_if_running(
        &self,
        id: TaskId<Id>,
        worker: &str,
        attempt: u32,
        error: String,
        payload: P,
        next_ready_at_ms: u64,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let Some(record) = g.tasks.get_mut(&id) else {
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

        record.payload = Some(payload);
        record.state = TaskState::Retrying {
            error,
            attempt,
            next_ready_at_ms,
        };
        record.lease_until_ms = None;
        g.scheduled
            .entry(next_ready_at_ms)
            .or_default()
            .push_back(id);
        Ok(true)
    }

    async fn get_view(&self, id: &TaskId<Id>) -> StoreResult<Option<(TaskState<O, Id>, Priority)>> {
        let g = self.inner.lock().await;
        let record = g.tasks.get(id);
        Ok(record.map(|r| (r.state.clone(), r.priority)))
    }

    async fn list_view_states(&self) -> StoreResult<Vec<(TaskId<Id>, TaskStateKind, Priority)>> {
        let g = self.inner.lock().await;
        Ok(g.tasks
            .iter()
            .map(|(id, record)| {
                (
                    id.clone(),
                    TaskStateKind::from(&record.state),
                    record.priority,
                )
            })
            .collect())
    }

    async fn dependents_of(&self, dep: &TaskId<Id>) -> StoreResult<Vec<TaskId<Id>>> {
        let g = self.inner.lock().await;
        Ok(g.dependents.get(dep).cloned().unwrap_or_default())
    }

    async fn dec_remaining_deps(&self, id: &TaskId<Id>) -> StoreResult<usize> {
        let mut g = self.inner.lock().await;
        let remaining = {
            let entry = g.remaining.entry(id.clone()).or_insert(0);
            if *entry > 0 {
                *entry -= 1;
            }
            *entry
        };

        if let Some(record) = g.tasks.get_mut(id)
            && matches!(record.state, TaskState::Pending { .. })
        {
            record.state = TaskState::pending(remaining);
        }

        Ok(remaining)
    }

    async fn try_mark_ready(&self, id: &TaskId<Id>) -> StoreResult<Option<Priority>> {
        let mut g = self.inner.lock().await;
        let remaining = g.remaining.get(id).copied().unwrap_or(0);
        if remaining != 0 {
            return Ok(None);
        }
        let Some(record) = g.tasks.get_mut(id) else {
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

    async fn push_ready(&self, prio: Priority, id: TaskId<Id>) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        push_ready_locked(&mut g, prio, id);

        Ok(())
    }

    async fn pop_ready(&self, prio: Priority) -> StoreResult<Option<TaskId<Id>>> {
        let mut g = self.inner.lock().await;
        let id = match prio {
            Priority::High => g.ready_high.pop_front(),
            Priority::Medium => g.ready_med.pop_front(),
            Priority::Low => g.ready_low.pop_front(),
        };
        Ok(id)
    }

    async fn take_ready(
        &self,
        id: &TaskId<Id>,
        worker: &str,
    ) -> StoreResult<Option<(P, Priority, u32, TaskExecutionPolicy)>> {
        let mut g = self.inner.lock().await;
        take_ready_locked(&mut g, self.lease, id, worker)
    }

    async fn put_payload(&self, id: &TaskId<Id>, payload: P) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        if let Some(record) = g.tasks.get_mut(id) {
            record.payload = Some(payload);
        }

        Ok(())
    }

    async fn renew_lease(&self, id: &TaskId<Id>, worker: &str, attempt: u32) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let Some(record) = g.tasks.get_mut(id) else {
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

        let lease_duration = record.execution_policy.lease_duration.max(self.lease);
        let lease_ms =
            u64::try_from(lease_duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX);
        record.lease_until_ms = Some(now_millis().saturating_add(lease_ms));
        Ok(true)
    }

    async fn schedule(&self, id: TaskId<Id>, not_before_ms: u64) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        g.scheduled.entry(not_before_ms).or_default().push_back(id);
        Ok(())
    }

    async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> StoreResult<usize> {
        let mut g = self.inner.lock().await;
        let mut moved = 0usize;

        while moved < limit {
            let Some((&ts, _)) = g.scheduled.iter().next() else {
                break;
            };
            if ts > now_ms {
                break;
            }

            let mut ids = g.scheduled.remove(&ts).unwrap_or_default();
            while moved < limit {
                let Some(id) = ids.pop_front() else {
                    break;
                };
                let Some(record) = g.tasks.get_mut(&id) else {
                    continue;
                };

                if let TaskState::Retrying {
                    next_ready_at_ms, ..
                } = record.state
                {
                    if next_ready_at_ms > now_ms {
                        g.scheduled
                            .entry(next_ready_at_ms)
                            .or_default()
                            .push_back(id);
                        continue;
                    }

                    record.state = TaskState::Ready;
                    record.lease_until_ms = None;
                    let priority = record.priority;
                    push_ready_locked(&mut g, priority, id);
                    moved += 1;
                }
            }

            if !ids.is_empty() {
                g.scheduled.insert(ts, ids);
            }
        }

        Ok(moved)
    }

    async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> StoreResult<usize> {
        let mut g = self.inner.lock().await;
        let mut expired = Vec::new();

        for (id, record) in &g.tasks {
            if !matches!(record.state, TaskState::Running { .. }) {
                continue;
            }
            let Some(lease_until_ms) = record.lease_until_ms else {
                continue;
            };
            if lease_until_ms > now_ms {
                continue;
            }

            expired.push((lease_until_ms, record.lease_sequence, id.clone()));
        }
        expired.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

        let moved = expired.len().min(limit);
        for (_, _, id) in expired.into_iter().take(limit) {
            let priority = {
                let Some(record) = g.tasks.get_mut(&id) else {
                    continue;
                };
                record.state = TaskState::Ready;
                record.lease_until_ms = None;
                record.priority
            };
            push_ready_locked(&mut g, priority, id);
        }

        Ok(moved)
    }

    async fn remove_task(&self, id: &TaskId<Id>) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        if g.tasks.remove(id).is_none() {
            return Ok(false);
        }

        g.remaining.remove(id);
        g.dependents.remove(id);
        remove_dependent_edges(&mut g.dependents, id);

        remove_queue_memberships(&mut g, id);

        Ok(true)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ready_push_pop_respects_priority() -> StoreResult<()> {
        let store: MemoryStore<(), (), u64> = MemoryStore::new();
        let a = TaskId::new(1);
        let b = TaskId::new(2);
        store.push_ready(Priority::Low, a.clone()).await?;
        store.push_ready(Priority::High, b.clone()).await?;
        assert_eq!(store.pop_ready(Priority::High).await?, Some(b));
        assert_eq!(store.pop_ready(Priority::Low).await?, Some(a));
        Ok(())
    }

    #[tokio::test]
    async fn medium_ready_sorted_by_sort_prefix() -> StoreResult<()> {
        let store: MemoryStore<(), (), u64> = MemoryStore::new();
        store.push_ready(Priority::Medium, TaskId::new(100)).await?;
        store.push_ready(Priority::Medium, TaskId::new(5)).await?;
        store.push_ready(Priority::Medium, TaskId::new(42)).await?;
        assert_eq!(
            store.pop_ready(Priority::Medium).await?,
            Some(TaskId::new(5))
        );
        assert_eq!(
            store.pop_ready(Priority::Medium).await?,
            Some(TaskId::new(42))
        );
        assert_eq!(
            store.pop_ready(Priority::Medium).await?,
            Some(TaskId::new(100))
        );
        Ok(())
    }

    #[tokio::test]
    async fn low_ready_sorted_by_sort_prefix() -> StoreResult<()> {
        let store: MemoryStore<(), (), u64> = MemoryStore::new();
        store.push_ready(Priority::Low, TaskId::new(100)).await?;
        store.push_ready(Priority::Low, TaskId::new(5)).await?;
        store.push_ready(Priority::Low, TaskId::new(42)).await?;
        assert_eq!(store.pop_ready(Priority::Low).await?, Some(TaskId::new(5)));
        assert_eq!(store.pop_ready(Priority::Low).await?, Some(TaskId::new(42)));
        assert_eq!(
            store.pop_ready(Priority::Low).await?,
            Some(TaskId::new(100))
        );
        Ok(())
    }
}
