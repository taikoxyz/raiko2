use crate::{Priority, TaskId, TaskState};
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
    O: Clone,
    Id: Send + Sync,
{
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
    ) -> StoreResult<bool>;
    async fn get_state(&self, id: &TaskId<Id>) -> StoreResult<Option<TaskState<O, Id>>>;
    async fn set_state(&self, id: &TaskId<Id>, state: TaskState<O, Id>) -> StoreResult<()>;
    async fn get_view(&self, id: &TaskId<Id>) -> StoreResult<Option<(TaskState<O, Id>, Priority)>>;
    async fn dependents_of(&self, dep: &TaskId<Id>) -> StoreResult<Vec<TaskId<Id>>>;
    async fn dec_remaining_deps(&self, id: &TaskId<Id>) -> StoreResult<usize>;
    async fn try_mark_ready(&self, id: &TaskId<Id>) -> StoreResult<Option<Priority>>;
    async fn push_ready(&self, prio: Priority, id: TaskId<Id>) -> StoreResult<()>;
    async fn pop_ready(&self, prio: Priority) -> StoreResult<Option<TaskId<Id>>>;
    async fn take_ready(
        &self,
        id: &TaskId<Id>,
        worker: &str,
    ) -> StoreResult<Option<(P, Priority, u32)>>;

    async fn pop_ready_and_take(
        &self,
        prio: Priority,
        worker: &str,
    ) -> StoreResult<Option<(TaskId<Id>, P, Priority, u32)>> {
        loop {
            let Some(id) = self.pop_ready(prio).await? else {
                return Ok(None);
            };
            if let Some((payload, priority, attempt)) = self.take_ready(&id, worker).await? {
                return Ok(Some((id, payload, priority, attempt)));
            }
        }
    }
    async fn put_payload(&self, id: &TaskId<Id>, payload: P) -> StoreResult<()>;
    async fn schedule(&self, id: TaskId<Id>, not_before_ms: u64) -> StoreResult<()>;
    async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> StoreResult<usize>;
    async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> StoreResult<usize>;
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
}

struct TaskRecord<P, O, Id> {
    payload: Option<P>,
    state: TaskState<O, Id>,
    priority: Priority,
    attempt: u32,
    lease_until_ms: Option<u64>,
}

impl<P, O, Id> MemoryStore<P, O, Id> {
    pub fn new() -> Self {
        Self::with_lease(Duration::from_secs(60))
    }

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

#[async_trait]
impl<P, O, Id> TaskStore<P, O, Id> for MemoryStore<P, O, Id>
where
    P: Clone + Send + 'static,
    O: Clone + Send + 'static,
    Id: Clone + Eq + Hash + Send + Sync + 'static,
{
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        if g.tasks.contains_key(&id) {
            return Ok(false);
        }
        let remaining = deps.len();
        g.remaining.insert(id.clone(), remaining);
        for dep in deps {
            g.dependents.entry(dep).or_default().push(id.clone());
        }
        g.tasks.insert(
            id,
            TaskRecord {
                payload: Some(payload),
                state: TaskState::pending(remaining),
                priority: prio,
                attempt: 0,
                lease_until_ms: None,
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
        if let Some(r) = g.tasks.get_mut(id) {
            r.state = state;
            if !matches!(r.state, TaskState::Running { .. }) {
                r.lease_until_ms = None;
            }
        }

        Ok(())
    }

    async fn get_view(&self, id: &TaskId<Id>) -> StoreResult<Option<(TaskState<O, Id>, Priority)>> {
        let g = self.inner.lock().await;
        let record = g.tasks.get(id);
        Ok(record.map(|r| (r.state.clone(), r.priority)))
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
        match prio {
            Priority::High => g.ready_high.push_back(id),
            Priority::Medium => g.ready_med.push_back(id),
            Priority::Low => g.ready_low.push_back(id),
        }

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
    ) -> StoreResult<Option<(P, Priority, u32)>> {
        let mut g = self.inner.lock().await;
        let Some(record) = g.tasks.get_mut(id) else {
            return Ok(None);
        };
        if !matches!(record.state, TaskState::Ready) {
            return Ok(None);
        }
        let Some(payload) = record.payload.as_ref() else {
            return Ok(None);
        };
        let payload = payload.clone();
        record.attempt = record.attempt.saturating_add(1);
        let attempt = record.attempt;
        record.state = TaskState::Running {
            worker: worker.to_string(),
            attempt,
        };
        let lease_ms = self.lease.as_millis().min(u64::MAX as u128) as u64;
        record.lease_until_ms = Some(now_millis().saturating_add(lease_ms));
        Ok(Some((payload, record.priority, attempt)))
    }

    async fn put_payload(&self, id: &TaskId<Id>, payload: P) -> StoreResult<()> {
        let mut g = self.inner.lock().await;
        if let Some(record) = g.tasks.get_mut(id) {
            record.payload = Some(payload);
        }

        Ok(())
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
                    match record.priority {
                        Priority::High => g.ready_high.push_back(id),
                        Priority::Medium => g.ready_med.push_back(id),
                        Priority::Low => g.ready_low.push_back(id),
                    }
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

        for (id, record) in g.tasks.iter_mut() {
            if expired.len() >= limit {
                break;
            }

            if !matches!(record.state, TaskState::Running { .. }) {
                continue;
            }
            let Some(lease_until_ms) = record.lease_until_ms else {
                continue;
            };
            if lease_until_ms > now_ms {
                continue;
            }

            record.state = TaskState::Ready;
            record.lease_until_ms = None;
            expired.push(id.clone());
        }

        for id in expired.iter().cloned() {
            let Some(record) = g.tasks.get(&id) else {
                continue;
            };
            match record.priority {
                Priority::High => g.ready_high.push_back(id),
                Priority::Medium => g.ready_med.push_back(id),
                Priority::Low => g.ready_low.push_back(id),
            }
        }

        Ok(expired.len())
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

    #[tokio::test]
    async fn ready_push_pop_respects_priority() {
        let store: MemoryStore<(), (), u64> = MemoryStore::new();
        let a = TaskId::new(1);
        let b = TaskId::new(2);
        store.push_ready(Priority::Low, a.clone()).await.unwrap();
        store.push_ready(Priority::High, b.clone()).await.unwrap();
        assert_eq!(store.pop_ready(Priority::High).await.unwrap(), Some(b));
        assert_eq!(store.pop_ready(Priority::Low).await.unwrap(), Some(a));
    }
}
