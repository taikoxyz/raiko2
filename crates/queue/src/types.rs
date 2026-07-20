use std::{error::Error, fmt};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId<Id>(pub Id);

impl<Id> TaskId<Id> {
    pub const fn new(id: Id) -> Self {
        Self(id)
    }

    pub fn into_inner(self) -> Id {
        self.0
    }
}

impl<Id> From<Id> for TaskId<Id> {
    fn from(id: Id) -> Self {
        Self(id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RootOwner {
    pub task_id: String,
    pub incarnation_id: Uuid,
}

impl RootOwner {
    #[must_use]
    pub fn new(task_id: impl Into<String>, incarnation_id: Uuid) -> Self {
        Self {
            task_id: task_id.into(),
            incarnation_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "high" => Some(Priority::High),
            "medium" => Some(Priority::Medium),
            "low" => Some(Priority::Low),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState<O, Id> {
    Pending {
        remaining_deps: usize,
    },
    Ready,
    Running {
        lease_token: String,
        attempt: u32,
    },
    Retrying {
        error: String,
        attempt: u32,
        next_ready_at_ms: u64,
    },
    Succeeded {
        output: O,
    },
    Failed {
        error: String,
        caused_by_dep: Option<TaskId<Id>>,
    },
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStateKind {
    Pending,
    Ready,
    Running,
    Retrying,
    Succeeded,
    Failed,
    Cancelled,
}

impl<O, Id> From<&TaskState<O, Id>> for TaskStateKind {
    fn from(state: &TaskState<O, Id>) -> Self {
        match state {
            TaskState::Pending { .. } => Self::Pending,
            TaskState::Ready => Self::Ready,
            TaskState::Running { .. } => Self::Running,
            TaskState::Retrying { .. } => Self::Retrying,
            TaskState::Succeeded { .. } => Self::Succeeded,
            TaskState::Failed { .. } => Self::Failed,
            TaskState::Cancelled => Self::Cancelled,
        }
    }
}

impl<O, Id> TaskState<O, Id> {
    #[must_use]
    pub const fn pending(remaining: usize) -> Self {
        TaskState::Pending {
            remaining_deps: remaining,
        }
    }
}

#[derive(Debug)]
pub struct TaskIdCodecError {
    message: String,
}

impl TaskIdCodecError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TaskIdCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for TaskIdCodecError {}

/// # Errors
///
/// Returns an error if the task ID cannot be serialized.
pub fn encode_task_id<Id>(id: &TaskId<Id>) -> Result<String, TaskIdCodecError>
where
    Id: Serialize,
{
    let bytes = bincode::serialize(&id.0)
        .map_err(|e| TaskIdCodecError::new(format!("serialize task id: {e}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// # Errors
///
/// Returns an error if the task ID cannot be decoded or deserialized.
pub fn decode_task_id<Id>(raw: &str) -> Result<TaskId<Id>, TaskIdCodecError>
where
    Id: for<'de> Deserialize<'de>,
{
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|e| TaskIdCodecError::new(format!("decode task id: {e}")))?;
    let id = bincode::deserialize(&bytes)
        .map_err(|e| TaskIdCodecError::new(format!("deserialize task id: {e}")))?;
    Ok(TaskId(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_is_unique() {
        let a = TaskId::new(1u64);
        let b = TaskId::new(2u64);
        assert_ne!(a, b);
    }
}
