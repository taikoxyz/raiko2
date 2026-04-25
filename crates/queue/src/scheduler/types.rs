use crate::{Priority, TaskExecutionPolicy, TaskId, TaskState};

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
    pub deadline_at_ms: u64,
}

#[derive(Clone)]
pub struct TaskView<O, Id> {
    pub id: TaskId<Id>,
    pub state: TaskState<O, Id>,
    pub priority: Priority,
}
