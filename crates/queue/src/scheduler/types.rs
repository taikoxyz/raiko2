use crate::{Priority, RootOwner, TaskExecutionPolicy, TaskId, TaskState, TaskStateKind};

#[derive(Clone)]
pub struct NewTask<P> {
    pub priority: Priority,
    pub payload: P,
}

#[derive(Clone)]
pub struct TaskLease<P, Id> {
    pub id: TaskId<Id>,
    pub payload: P,
    pub priority: Priority,
    pub attempt: u32,
    /// Opaque identity for this exact lease; never reused after task replacement.
    pub lease_token: String,
    pub worker: String,
    pub execution_policy: TaskExecutionPolicy,
}

#[derive(Clone)]
pub struct ExecutionNode<P, Id> {
    pub id: TaskId<Id>,
    pub task: NewTask<P>,
    pub dependencies: Vec<TaskId<Id>>,
    pub execution_policy: TaskExecutionPolicy,
}

#[derive(Clone)]
pub struct ExecutionGraph<P, Id> {
    pub nodes: Vec<ExecutionNode<P, Id>>,
}

impl<P, Id> ExecutionGraph<P, Id> {
    #[must_use]
    pub const fn new(nodes: Vec<ExecutionNode<P, Id>>) -> Self {
        Self { nodes }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachOutcome {
    Attached,
    AlreadyAttached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachMode {
    Cancel,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachOutcome<Id> {
    pub detached: bool,
    pub mode: DetachMode,
    pub retired: Vec<TaskId<Id>>,
    pub retained: Vec<TaskId<Id>>,
}

impl<Id> DetachOutcome<Id> {
    #[must_use]
    pub const fn not_attached(mode: DetachMode) -> Self {
        Self {
            detached: false,
            mode,
            retired: Vec::new(),
            retained: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionTaskView<Id> {
    pub id: TaskId<Id>,
    pub state: TaskStateKind,
    pub owner_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionView<Id> {
    pub owner: RootOwner,
    pub tasks: Vec<ProjectionTaskView<Id>>,
}

#[derive(Clone)]
pub struct TaskView<O, Id> {
    pub id: TaskId<Id>,
    pub state: TaskState<O, Id>,
    pub priority: Priority,
}

#[derive(Clone)]
pub struct TaskViewState<Id> {
    pub id: TaskId<Id>,
    pub state: TaskStateKind,
    pub priority: Priority,
}
