use crate::ready_sort::insert_ready_sorted;
use crate::{
    AttachOutcome, DetachMode, DetachOutcome, ExecutionNode, Priority, ProjectionTaskView,
    ProjectionView, ReadyQueueSort, RootOwner, TaskExecutionPolicy, TaskId, TaskState,
    TaskStateKind,
};
use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::error::Error;
use std::hash::Hash;
use std::time::Duration;
use tokio::sync::Mutex;

pub type StoreResult<T> = Result<T, TaskStoreError>;

#[derive(Debug)]
pub enum TaskStoreError {
    /// Underlying storage/backend failure.
    Backend(Box<dyn Error + Send + Sync>),
    /// Data is missing or cannot be decoded.
    CorruptData(Box<dyn Error + Send + Sync>),
    InvalidGraph(String),
    Conflict(String),
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

    pub fn invalid_graph(msg: impl Into<String>) -> Self {
        Self::InvalidGraph(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }
}

impl std::fmt::Display for TaskStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStoreError::Backend(err) => write!(f, "task store backend error: {err}"),
            TaskStoreError::CorruptData(err) => write!(f, "task store corrupt data: {err}"),
            TaskStoreError::InvalidGraph(message) => {
                write!(f, "invalid execution graph: {message}")
            }
            TaskStoreError::Conflict(message) => {
                write!(f, "execution projection conflict: {message}")
            }
        }
    }
}

impl Error for TaskStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            TaskStoreError::Backend(err) | TaskStoreError::CorruptData(err) => Some(err.as_ref()),
            TaskStoreError::InvalidGraph(_) | TaskStoreError::Conflict(_) => None,
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
    async fn attach_graph(
        &self,
        owner: RootOwner,
        nodes: Vec<ExecutionNode<P, Id>>,
    ) -> StoreResult<AttachOutcome>
    where
        P: PartialEq;
    async fn detach_owner(
        &self,
        owner: &RootOwner,
        mode: DetachMode,
    ) -> StoreResult<DetachOutcome<Id>>;
    async fn inspect_owner(&self, owner: &RootOwner) -> StoreResult<Option<ProjectionView<Id>>>;
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool>;
    async fn get_state(&self, id: &TaskId<Id>) -> StoreResult<Option<TaskState<O, Id>>>;
    async fn set_state_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        state: TaskState<O, Id>,
        payload: Option<P>,
    ) -> StoreResult<bool>;
    async fn complete_success_and_release_dependents_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        output: O,
    ) -> StoreResult<bool>;
    async fn complete_failure_and_fail_dependents_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        error: String,
        dependent_error: String,
    ) -> StoreResult<bool>;
    async fn cancel_and_fail_dependents(
        &self,
        id: &TaskId<Id>,
        dependent_error: String,
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
    async fn get_view_state(
        &self,
        id: &TaskId<Id>,
    ) -> StoreResult<Option<(TaskStateKind, Priority)>>;
    async fn list_view_states(&self) -> StoreResult<Vec<(TaskId<Id>, TaskStateKind, Priority)>>;
    async fn dependents_of(&self, dep: &TaskId<Id>) -> StoreResult<Vec<TaskId<Id>>>;
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
    async fn get_payload(&self, _id: &TaskId<Id>) -> StoreResult<Option<P>> {
        Ok(None)
    }
    async fn checkpoint_payload_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        payload: P,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool>;
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
    owner_graphs: HashMap<RootOwner, OwnerGraph<Id>>,
    task_owners: HashMap<TaskId<Id>, HashSet<RootOwner>>,
    next_sequence: u64,
}

struct OwnerGraph<Id> {
    task_ids: Vec<TaskId<Id>>,
    attached: bool,
}

struct TaskRecord<P, O, Id> {
    definition_payload: P,
    payload: Option<P>,
    state: TaskState<O, Id>,
    priority: Priority,
    attempt: u32,
    lease_until_ms: Option<u64>,
    lease_sequence: u64,
    execution_policy: TaskExecutionPolicy,
    dependencies: Vec<TaskId<Id>>,
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

fn normalize_and_validate_graph<P, Id>(
    owner: &RootOwner,
    nodes: &mut [ExecutionNode<P, Id>],
) -> StoreResult<()>
where
    Id: Clone + Eq + Hash,
{
    if owner.task_id.trim().is_empty() {
        return Err(TaskStoreError::invalid_graph(
            "root owner task id must not be empty",
        ));
    }
    if owner.incarnation_id.is_nil() {
        return Err(TaskStoreError::invalid_graph(
            "root owner incarnation must not be nil",
        ));
    }
    if nodes.is_empty() {
        return Err(TaskStoreError::invalid_graph(
            "an execution graph must contain at least one task",
        ));
    }

    let mut graph_ids = HashSet::with_capacity(nodes.len());
    for node in &*nodes {
        if !graph_ids.insert(node.id.clone()) {
            return Err(TaskStoreError::invalid_graph(
                "an execution graph contains duplicate task ids",
            ));
        }
    }

    for node in &mut *nodes {
        let mut seen = HashSet::with_capacity(node.dependencies.len());
        node.dependencies
            .retain(|dependency| seen.insert(dependency.clone()));
        if node
            .dependencies
            .iter()
            .any(|dependency| !graph_ids.contains(dependency))
        {
            return Err(TaskStoreError::invalid_graph(
                "every dependency must be part of the attached execution graph",
            ));
        }
    }

    validate_graph_is_acyclic(nodes)
}

fn validate_graph_is_acyclic<P, Id>(nodes: &[ExecutionNode<P, Id>]) -> StoreResult<()>
where
    Id: Clone + Eq + Hash,
{
    fn visit<P, Id>(
        index: usize,
        nodes: &[ExecutionNode<P, Id>],
        indices: &HashMap<TaskId<Id>, usize>,
        visiting: &mut HashSet<TaskId<Id>>,
        visited: &mut HashSet<TaskId<Id>>,
    ) -> bool
    where
        Id: Clone + Eq + Hash,
    {
        let id = nodes[index].id.clone();
        if visited.contains(&id) {
            return true;
        }
        if !visiting.insert(id.clone()) {
            return false;
        }
        for dependency in &nodes[index].dependencies {
            let Some(dependency_index) = indices.get(dependency).copied() else {
                return false;
            };
            if !visit(dependency_index, nodes, indices, visiting, visited) {
                return false;
            }
        }
        visiting.remove(&id);
        visited.insert(id);
        true
    }

    let indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut visiting = HashSet::with_capacity(nodes.len());
    let mut visited = HashSet::with_capacity(nodes.len());
    for index in 0..nodes.len() {
        if !visit(index, nodes, &indices, &mut visiting, &mut visited) {
            return Err(TaskStoreError::invalid_graph(
                "an execution graph must not contain dependency cycles",
            ));
        }
    }
    Ok(())
}

fn graph_task_sets_match<Id>(left: &[TaskId<Id>], right: &[TaskId<Id>]) -> bool
where
    Id: Clone + Eq + Hash,
{
    left.len() == right.len()
        && left.iter().cloned().collect::<HashSet<_>>()
            == right.iter().cloned().collect::<HashSet<_>>()
}

fn task_definition_matches<P, O, Id>(
    record: &TaskRecord<P, O, Id>,
    node: &ExecutionNode<P, Id>,
) -> bool
where
    P: PartialEq,
    Id: Clone + Eq + Hash,
{
    record.definition_payload == node.task.payload
        && record.priority == node.task.priority
        && record.execution_policy == node.execution_policy
        && graph_task_sets_match(&record.dependencies, &node.dependencies)
}

fn graph_topological_indices<P, Id>(nodes: &[ExecutionNode<P, Id>]) -> Vec<usize>
where
    Id: Clone + Eq + Hash,
{
    fn append<P, Id>(
        index: usize,
        nodes: &[ExecutionNode<P, Id>],
        indices: &HashMap<TaskId<Id>, usize>,
        visited: &mut HashSet<TaskId<Id>>,
        ordered: &mut Vec<usize>,
    ) where
        Id: Clone + Eq + Hash,
    {
        if !visited.insert(nodes[index].id.clone()) {
            return;
        }
        for dependency in &nodes[index].dependencies {
            append(indices[dependency], nodes, indices, visited, ordered);
        }
        ordered.push(index);
    }

    let indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut visited = HashSet::with_capacity(nodes.len());
    let mut ordered = Vec::with_capacity(nodes.len());
    for index in 0..nodes.len() {
        append(index, nodes, &indices, &mut visited, &mut ordered);
    }
    ordered
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
                owner_graphs: HashMap::new(),
                task_owners: HashMap::new(),
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

fn reset_attached_terminal_nodes<P, O, Id>(
    inner: &mut Inner<P, O, Id>,
    nodes: &[ExecutionNode<P, Id>],
    topological_indices: &[usize],
) -> bool
where
    P: Clone,
    Id: ReadyQueueSort,
{
    let reset_ids = topological_indices
        .iter()
        .filter_map(|index| {
            let id = &nodes[*index].id;
            inner
                .tasks
                .get(id)
                .is_some_and(|record| {
                    matches!(
                        record.state,
                        TaskState::Failed { .. } | TaskState::Cancelled
                    )
                })
                .then(|| id.clone())
        })
        .collect::<HashSet<_>>();
    if reset_ids.is_empty() {
        return false;
    }

    for task_id in &reset_ids {
        remove_queue_memberships(inner, task_id);
        remove_dependent_edges(&mut inner.dependents, task_id);
    }

    for index in topological_indices {
        let node = &nodes[*index];
        if !reset_ids.contains(&node.id) {
            continue;
        }
        let dependency_state =
            resolve_dependency_insert_state(&inner.tasks, node.dependencies.clone());
        let (remaining, state, unresolved_dependencies) = match dependency_state {
            DependencyInsertState::Pending {
                remaining,
                unresolved_deps,
            } => {
                let state = if remaining == 0 {
                    TaskState::Ready
                } else {
                    TaskState::pending(remaining)
                };
                (remaining, state, unresolved_deps)
            }
            DependencyInsertState::Failed | DependencyInsertState::Cancelled => {
                unreachable!("all terminal dependencies in the attached graph are reset together")
            }
        };
        for dependency in unresolved_dependencies {
            inner
                .dependents
                .entry(dependency)
                .or_default()
                .push(node.id.clone());
        }
        let ready = matches!(state, TaskState::Ready);
        let priority = {
            let record = inner
                .tasks
                .get_mut(&node.id)
                .expect("attached graph task was checked before recovery reset");
            record.payload = Some(node.task.payload.clone());
            record.state = state;
            record.lease_until_ms = None;
            record.dependencies.clone_from(&node.dependencies);
            record.priority
        };
        inner.remaining.insert(node.id.clone(), remaining);
        if ready {
            push_ready_locked(inner, priority, node.id.clone());
        }
    }
    true
}

fn running_lease_matches<P, O, Id>(
    inner: &Inner<P, O, Id>,
    id: &TaskId<Id>,
    worker: &str,
    attempt: u32,
) -> bool
where
    Id: Clone + Eq + Hash,
{
    matches!(
        inner.tasks.get(id).map(|record| &record.state),
        Some(TaskState::Running {
            worker: current_worker,
            attempt: current_attempt,
        }) if current_worker == worker && *current_attempt == attempt
    )
}

fn release_dependents_locked<P, O, Id>(inner: &mut Inner<P, O, Id>, root: &TaskId<Id>)
where
    Id: ReadyQueueSort,
{
    let dependents = inner.dependents.get(root).cloned().unwrap_or_default();
    for dependent in dependents {
        let is_pending = inner
            .tasks
            .get(&dependent)
            .is_some_and(|record| matches!(record.state, TaskState::Pending { .. }));
        if !is_pending {
            continue;
        }
        let remaining = {
            let entry = inner.remaining.entry(dependent.clone()).or_insert(0);
            *entry = entry.saturating_sub(1);
            *entry
        };
        let mut ready_priority = None;
        if let Some(record) = inner.tasks.get_mut(&dependent) {
            record.state = TaskState::pending(remaining);
            if remaining == 0 {
                record.state = TaskState::Ready;
                ready_priority = Some(record.priority);
            }
        }
        if let Some(priority) = ready_priority {
            push_ready_locked(inner, priority, dependent);
        }
    }
}

fn fail_dependents_locked<P, O, Id>(inner: &mut Inner<P, O, Id>, root: &TaskId<Id>, error: &str)
where
    Id: ReadyQueueSort,
{
    let mut queue: VecDeque<TaskId<Id>> = inner
        .dependents
        .get(root)
        .cloned()
        .unwrap_or_default()
        .into();
    let mut visited = HashSet::new();

    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        let is_terminal = inner.tasks.get(&id).is_some_and(|record| {
            matches!(
                record.state,
                TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. }
            )
        });
        if !is_terminal {
            remove_queue_memberships(inner, &id);
            if let Some(record) = inner.tasks.get_mut(&id) {
                record.state = TaskState::Failed {
                    error: error.to_string(),
                    caused_by_dep: Some(root.clone()),
                };
                record.lease_until_ms = None;
            }
        }
        if let Some(dependents) = inner.dependents.get(&id) {
            queue.extend(dependents.iter().cloned());
        }
    }
}

fn cancel_task_locked<P, O, Id>(inner: &mut Inner<P, O, Id>, id: &TaskId<Id>) -> StoreResult<bool>
where
    Id: ReadyQueueSort,
{
    let Some(record) = inner.tasks.get(id) else {
        return Ok(false);
    };
    if matches!(
        record.state,
        TaskState::Cancelled | TaskState::Succeeded { .. } | TaskState::Failed { .. }
    ) {
        return Ok(false);
    }
    remove_queue_memberships(inner, id);
    let record = inner
        .tasks
        .get_mut(id)
        .ok_or_else(|| TaskStoreError::corrupt_msg("task disappeared during cancellation"))?;
    record.state = TaskState::Cancelled;
    record.lease_until_ms = None;
    fail_dependents_locked(inner, id, "dependency cancelled");
    Ok(true)
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl<P, O, Id> TaskStore<P, O, Id> for MemoryStore<P, O, Id>
where
    P: Clone + Send + 'static,
    O: Clone + Send + 'static,
    Id: ReadyQueueSort,
{
    async fn attach_graph(
        &self,
        owner: RootOwner,
        mut nodes: Vec<ExecutionNode<P, Id>>,
    ) -> StoreResult<AttachOutcome>
    where
        P: PartialEq,
    {
        normalize_and_validate_graph(&owner, &mut nodes)?;
        let topological_indices = graph_topological_indices(&nodes);
        let graph_ids = topological_indices
            .iter()
            .map(|index| nodes[*index].id.clone())
            .collect::<Vec<_>>();
        let mut inner = self.inner.lock().await;

        if let Some(existing_graph) = inner.owner_graphs.get(&owner) {
            if !existing_graph.attached {
                return Err(TaskStoreError::conflict(
                    "a retired root owner cannot be attached again",
                ));
            }
            if !graph_task_sets_match(&existing_graph.task_ids, &graph_ids) {
                return Err(TaskStoreError::conflict(
                    "the same root owner is already attached to a different graph",
                ));
            }
            for node in &nodes {
                let Some(record) = inner.tasks.get(&node.id) else {
                    return Err(TaskStoreError::corrupt_msg(
                        "an attached root owner references a missing task",
                    ));
                };
                if !task_definition_matches(record, node) {
                    return Err(TaskStoreError::conflict(
                        "an attached task id has a different execution definition",
                    ));
                }
                if !inner
                    .task_owners
                    .get(&node.id)
                    .is_some_and(|owners| owners.contains(&owner))
                {
                    return Err(TaskStoreError::corrupt_msg(
                        "owner and task ownership indexes disagree",
                    ));
                }
            }
            return Ok(
                if reset_attached_terminal_nodes(&mut inner, &nodes, &topological_indices) {
                    AttachOutcome::Attached
                } else {
                    AttachOutcome::AlreadyAttached
                },
            );
        }

        for node in &nodes {
            if let Some(record) = inner.tasks.get(&node.id)
                && !task_definition_matches(record, node)
            {
                return Err(TaskStoreError::conflict(
                    "a task id is already attached with a different execution definition",
                ));
            }
        }

        for index in topological_indices {
            let node = &nodes[index];
            let reset_unowned_terminal = inner.tasks.get(&node.id).is_some_and(|record| {
                matches!(
                    record.state,
                    TaskState::Failed { .. } | TaskState::Cancelled
                ) && inner
                    .task_owners
                    .get(&node.id)
                    .is_none_or(HashSet::is_empty)
            });
            if !inner.tasks.contains_key(&node.id) || reset_unowned_terminal {
                let dependency_state =
                    resolve_dependency_insert_state(&inner.tasks, node.dependencies.clone());
                let (remaining, state, unresolved_dependencies) = match dependency_state {
                    DependencyInsertState::Pending {
                        remaining,
                        unresolved_deps,
                    } => {
                        let state = if remaining == 0 {
                            TaskState::Ready
                        } else {
                            TaskState::pending(remaining)
                        };
                        (remaining, state, unresolved_deps)
                    }
                    DependencyInsertState::Failed => (
                        0,
                        TaskState::Failed {
                            error: "dependency failed".to_string(),
                            caused_by_dep: None,
                        },
                        Vec::new(),
                    ),
                    DependencyInsertState::Cancelled => (0, TaskState::Cancelled, Vec::new()),
                };
                if reset_unowned_terminal {
                    remove_dependent_edges(&mut inner.dependents, &node.id);
                    remove_queue_memberships(&mut inner, &node.id);
                }
                for dependency in unresolved_dependencies {
                    inner
                        .dependents
                        .entry(dependency)
                        .or_default()
                        .push(node.id.clone());
                }
                inner.remaining.insert(node.id.clone(), remaining);
                inner.tasks.insert(
                    node.id.clone(),
                    TaskRecord {
                        definition_payload: node.task.payload.clone(),
                        payload: Some(node.task.payload.clone()),
                        state: state.clone(),
                        priority: node.task.priority,
                        attempt: 0,
                        lease_until_ms: None,
                        lease_sequence: 0,
                        execution_policy: node.execution_policy.clone(),
                        dependencies: node.dependencies.clone(),
                    },
                );
                if matches!(state, TaskState::Ready) {
                    push_ready_locked(&mut inner, node.task.priority, node.id.clone());
                }
            }
            inner
                .task_owners
                .entry(node.id.clone())
                .or_default()
                .insert(owner.clone());
        }
        inner.owner_graphs.insert(
            owner,
            OwnerGraph {
                task_ids: graph_ids,
                attached: true,
            },
        );
        Ok(AttachOutcome::Attached)
    }

    async fn detach_owner(
        &self,
        owner: &RootOwner,
        mode: DetachMode,
    ) -> StoreResult<DetachOutcome<Id>> {
        let mut inner = self.inner.lock().await;
        let Some(owner_graph) = inner.owner_graphs.get(owner) else {
            return Ok(DetachOutcome::not_attached(mode));
        };
        let task_ids = owner_graph.task_ids.clone();
        let was_attached = owner_graph.attached;
        if !was_attached && mode == DetachMode::Cancel {
            return Ok(DetachOutcome::not_attached(mode));
        }
        let mut retired = Vec::new();
        let mut retained = Vec::new();
        if was_attached {
            for task_id in &task_ids {
                if !inner.tasks.contains_key(task_id)
                    || !inner
                        .task_owners
                        .get(task_id)
                        .is_some_and(|owners| owners.contains(owner))
                {
                    return Err(TaskStoreError::corrupt_msg(
                        "owner and task ownership indexes disagree",
                    ));
                }
            }
            for task_id in &task_ids {
                if let Some(owners) = inner.task_owners.get_mut(task_id) {
                    owners.remove(owner);
                }
            }
        }
        for task_id in &task_ids {
            if inner
                .task_owners
                .get(task_id)
                .is_some_and(|owners| !owners.is_empty())
            {
                retained.push(task_id.clone());
            } else {
                inner.task_owners.remove(task_id);
                retired.push(task_id.clone());
            }
        }
        match mode {
            DetachMode::Cancel => {
                for task_id in &retired {
                    let _ = cancel_task_locked(&mut inner, task_id)?;
                }
            }
            DetachMode::Remove => {
                for task_id in &retired {
                    inner.tasks.remove(task_id);
                    inner.remaining.remove(task_id);
                    inner.dependents.remove(task_id);
                    remove_dependent_edges(&mut inner.dependents, task_id);
                    remove_queue_memberships(&mut inner, task_id);
                }
            }
        }
        match mode {
            DetachMode::Cancel => {
                if let Some(owner_graph) = inner.owner_graphs.get_mut(owner) {
                    owner_graph.attached = false;
                }
            }
            DetachMode::Remove => {
                inner.owner_graphs.remove(owner);
            }
        }
        Ok(DetachOutcome {
            detached: true,
            mode,
            retired,
            retained,
        })
    }

    async fn inspect_owner(&self, owner: &RootOwner) -> StoreResult<Option<ProjectionView<Id>>> {
        let inner = self.inner.lock().await;
        let Some(owner_graph) = inner.owner_graphs.get(owner) else {
            return Ok(None);
        };
        if !owner_graph.attached {
            return Ok(None);
        }
        let task_ids = &owner_graph.task_ids;
        let mut tasks = Vec::with_capacity(task_ids.len());
        for task_id in task_ids {
            let record = inner.tasks.get(task_id).ok_or_else(|| {
                TaskStoreError::corrupt_msg("an attached root owner references a missing task")
            })?;
            let owners = inner.task_owners.get(task_id).ok_or_else(|| {
                TaskStoreError::corrupt_msg("owner and task ownership indexes disagree")
            })?;
            tasks.push(ProjectionTaskView {
                id: task_id.clone(),
                state: TaskStateKind::from(&record.state),
                owner_count: owners.len(),
            });
        }
        Ok(Some(ProjectionView {
            owner: owner.clone(),
            tasks,
        }))
    }

    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let task_dependencies = deps.clone();
        let dependency_state = resolve_dependency_insert_state(&g.tasks, deps);
        let is_owned = g
            .task_owners
            .get(&id)
            .is_some_and(|owners| !owners.is_empty());
        let should_reset = matches!(
            g.tasks.get(&id).map(|record| &record.state),
            Some(TaskState::Failed { .. } | TaskState::Cancelled)
        ) && !is_owned;
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
            } = &dependency_state
            {
                for dep in unresolved_deps {
                    g.dependents
                        .entry(dep.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
            g.remaining.insert(id.clone(), remaining);
            remove_queue_memberships(&mut g, &id);
            if let Some(existing) = g.tasks.get_mut(&id) {
                existing.definition_payload = payload.clone();
                existing.payload = Some(payload);
                existing.priority = prio;
                existing.attempt = 0;
                existing.lease_until_ms = None;
                existing.lease_sequence = 0;
                existing.state = next_state;
                existing.execution_policy = execution_policy;
                existing.dependencies = task_dependencies;
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
                definition_payload: payload.clone(),
                payload: Some(payload),
                state: next_state,
                priority: prio,
                attempt: 0,
                lease_until_ms: None,
                lease_sequence: 0,
                execution_policy,
                dependencies: task_dependencies,
            },
        );

        Ok(true)
    }

    async fn get_state(&self, id: &TaskId<Id>) -> StoreResult<Option<TaskState<O, Id>>> {
        let g = self.inner.lock().await;
        Ok(g.tasks.get(id).map(|r| &r.state).cloned())
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

    async fn complete_success_and_release_dependents_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        output: O,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        if !running_lease_matches(&g, id, worker, attempt) {
            return Ok(false);
        }
        remove_queue_memberships(&mut g, id);
        let record = g
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskStoreError::corrupt_msg("running task disappeared"))?;
        record.state = TaskState::Succeeded { output };
        record.lease_until_ms = None;
        release_dependents_locked(&mut g, id);
        Ok(true)
    }

    async fn complete_failure_and_fail_dependents_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        error: String,
        dependent_error: String,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        if !running_lease_matches(&g, id, worker, attempt) {
            return Ok(false);
        }
        remove_queue_memberships(&mut g, id);
        let record = g
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskStoreError::corrupt_msg("running task disappeared"))?;
        record.state = TaskState::Failed {
            error,
            caused_by_dep: None,
        };
        record.lease_until_ms = None;
        fail_dependents_locked(&mut g, id, &dependent_error);
        Ok(true)
    }

    async fn cancel_and_fail_dependents(
        &self,
        id: &TaskId<Id>,
        dependent_error: String,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        if g.task_owners
            .get(id)
            .is_some_and(|owners| !owners.is_empty())
        {
            return Err(TaskStoreError::conflict(
                "an owned task must be cancelled by detaching its root owner",
            ));
        }
        let _ = dependent_error;
        cancel_task_locked(&mut g, id)
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

    async fn get_view_state(
        &self,
        id: &TaskId<Id>,
    ) -> StoreResult<Option<(TaskStateKind, Priority)>> {
        let g = self.inner.lock().await;
        let record = g.tasks.get(id);
        Ok(record.map(|r| (TaskStateKind::from(&r.state), r.priority)))
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

    async fn get_payload(&self, id: &TaskId<Id>) -> StoreResult<Option<P>> {
        let g = self.inner.lock().await;
        Ok(g.tasks.get(id).and_then(|record| record.payload.clone()))
    }

    async fn checkpoint_payload_if_running(
        &self,
        id: &TaskId<Id>,
        worker: &str,
        attempt: u32,
        payload: P,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool> {
        let mut g = self.inner.lock().await;
        let Some(record) = g.tasks.get_mut(id) else {
            return Ok(false);
        };
        if !matches!(
            &record.state,
            TaskState::Running {
                worker: current_worker,
                attempt: current_attempt,
            } if current_worker == worker && *current_attempt == attempt
        ) {
            return Ok(false);
        }
        record.payload = Some(payload);
        record.execution_policy = execution_policy;
        Ok(true)
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
        if g.task_owners
            .get(id)
            .is_some_and(|owners| !owners.is_empty())
        {
            return Err(TaskStoreError::conflict(
                "an owned task must be removed by detaching its root owner",
            ));
        }
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
