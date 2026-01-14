use crate::{Priority, TaskId, TaskState};

#[derive(Clone)]
pub struct NewTask<P> {
    pub priority: Priority,
    pub payload: P,
}

pub struct TaskLease<P, Id> {
    pub id: TaskId<Id>,
    pub payload: P,
    pub priority: Priority,
    pub attempt: u32,
    pub worker: String,
}

#[derive(Clone)]
pub struct TaskView<O, Id> {
    pub id: TaskId<Id>,
    pub state: TaskState<O, Id>,
    pub priority: Priority,
}
