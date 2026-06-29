#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod ready_sort;
#[cfg(feature = "redis")]
mod redis_store;
mod scheduler;
mod store;
mod types;

pub use ready_sort::{
    READY_SORT_PREFIX_HEX_LEN, ReadyQueueSort, encoded_from_zset_member, sort_prefix_hex,
    zset_member_from_encoded,
};
#[cfg(feature = "redis")]
pub use redis_store::RedisStore;
pub use scheduler::{
    NewTask, RetryPolicy, Scheduler, SchedulerConfig, TaskExecutionPolicy, TaskLease, TaskView,
    TaskViewState,
};
pub use store::{MemoryStore, StoreResult, TaskStore, TaskStoreError};
pub use types::{
    Priority, TaskId, TaskIdCodecError, TaskState, TaskStateKind, decode_task_id, encode_task_id,
};
