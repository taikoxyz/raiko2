use crate::{Priority, TaskExecutionPolicy, TaskId, TaskState, TaskStateKind};

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
    pub worker: String,
    pub execution_policy: TaskExecutionPolicy,
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
