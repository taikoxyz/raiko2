#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

#[cfg(feature = "redis")]
mod redis_store;
mod scheduler;
mod store;
mod types;

#[cfg(feature = "redis")]
pub use redis_store::RedisStore;
pub use scheduler::{NewTask, RetryPolicy, Scheduler, SchedulerConfig, TaskLease, TaskView};
pub use store::{MemoryStore, StoreResult, TaskStore, TaskStoreError};
pub use types::{Priority, TaskId, TaskKind, TaskState};
