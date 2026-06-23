use crate::{
    Priority, ReadyQueueSort, StoreResult, TaskExecutionPolicy, TaskId, TaskState, TaskStateKind,
    TaskStore, TaskStoreError, decode_task_id, encode_task_id, encoded_from_zset_member,
    sort_prefix_hex, zset_member_from_encoded,
};
use async_trait::async_trait;
use redis::AsyncCommands;
use serde::{Serialize, de::DeserializeOwned};
use std::{marker::PhantomData, time::Duration};
use tokio::sync::Mutex;

const FIELD_PRIORITY: &str = "priority";
const FIELD_STATE: &str = "state";
const FIELD_REMAINING: &str = "remaining_deps";
const FIELD_PAYLOAD: &str = "payload";
const FIELD_ATTEMPT: &str = "attempt";
const FIELD_OUTPUT: &str = "output";
const FIELD_ERROR: &str = "error";
const FIELD_NEXT_READY_AT_MS: &str = "next_ready_at_ms";
const FIELD_CAUSED_BY: &str = "caused_by_dep";
const FIELD_WORKER: &str = "worker";
const FIELD_LEASE_UNTIL_MS: &str = "lease_until_ms";
const FIELD_LEASE_DURATION_MS: &str = "lease_duration_ms";
const FIELD_EXECUTION_POLICY: &str = "execution_policy";
const FIELD_RUNNING_MEMBER: &str = "running_member";
/// Hex-encoded [`ReadyQueueSort::ready_queue_sort_prefix`] (32 chars); used to build ZSET members.
const FIELD_RQP_HEX: &str = "rqp";
const FIELD_READY_MEMBER: &str = "ready_member";

const STATE_PENDING: &str = "pending";
const STATE_READY: &str = "ready";
const STATE_RUNNING: &str = "running";
const STATE_RETRYING: &str = "retrying";
const STATE_SUCCEEDED: &str = "succeeded";
const STATE_FAILED: &str = "failed";
const STATE_CANCELLED: &str = "cancelled";

fn task_state_kind(raw: &str) -> StoreResult<TaskStateKind> {
    match raw {
        STATE_PENDING => Ok(TaskStateKind::Pending),
        STATE_READY => Ok(TaskStateKind::Ready),
        STATE_RUNNING => Ok(TaskStateKind::Running),
        STATE_RETRYING => Ok(TaskStateKind::Retrying),
        STATE_SUCCEEDED => Ok(TaskStateKind::Succeeded),
        STATE_FAILED => Ok(TaskStateKind::Failed),
        STATE_CANCELLED => Ok(TaskStateKind::Cancelled),
        other => Err(TaskStoreError::corrupt_msg(format!(
            "unknown task state: {other}"
        ))),
    }
}

const SET_STATE_IF_RUNNING_SCRIPT: &str = r"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return 0
end

local current_worker = redis.call('HGET', KEYS[1], ARGV[3])
if current_worker ~= ARGV[4] then
  return 0
end

local current_attempt = redis.call('HGET', KEYS[1], ARGV[5])
if not current_attempt or tonumber(current_attempt) ~= tonumber(ARGV[6]) then
  return 0
end

if ARGV[10] == '1' then
  redis.call('HSET', KEYS[1], ARGV[9], ARGV[11])
end

    local running_member = redis.call('HGET', KEYS[1], ARGV[13])
    if not running_member then
      return redis.error_reply('missing running member')
    end

    redis.call('ZREM', KEYS[2], running_member)
    redis.call('HDEL', KEYS[1], ARGV[3], ARGV[7], ARGV[13])

    local idx = 14
    local set_count = tonumber(ARGV[12])
for _ = 1, set_count do
  redis.call('HSET', KEYS[1], ARGV[idx], ARGV[idx + 1])
  idx = idx + 2
end

local hdel_count = tonumber(ARGV[idx])
idx = idx + 1
for _ = 1, hdel_count do
  redis.call('HDEL', KEYS[1], ARGV[idx])
  idx = idx + 1
end

return 1
";

pub struct RedisStore<P, O, Id> {
    conn: Mutex<redis::aio::MultiplexedConnection>,
    namespace: String,
    lease: Duration,
    _phantom: PhantomData<fn(P, O, Id)>,
}

impl<P, O, Id> RedisStore<P, O, Id>
where
    P: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Clone + Send + 'static,
    Id: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// Connects to Redis and creates a namespaced task store.
    ///
    /// # Errors
    ///
    /// Returns the Redis client or connection error when the URL is invalid or the server cannot
    /// be reached.
    pub async fn connect(
        url: &str,
        namespace: &str,
        lease: Duration,
    ) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(url)?;
        let conn = client.get_multiplexed_async_connection().await?;
        Ok(Self {
            conn: Mutex::new(conn),
            namespace: namespace.to_string(),
            lease,
            _phantom: PhantomData,
        })
    }

    fn task_key(&self, id: &TaskId<Id>) -> StoreResult<String> {
        Ok(format!("{}:task:{}", self.namespace, Self::encode_id(id)?))
    }

    fn dependents_key(&self, id: &TaskId<Id>) -> StoreResult<String> {
        Ok(format!(
            "{}:dependents:{}",
            self.namespace,
            Self::encode_id(id)?
        ))
    }

    fn ready_key(&self, prio: Priority) -> String {
        match prio {
            Priority::High => format!("{}:ready:{}", self.namespace, prio.as_str()),
            // Sorted-set queues (see `push_ready` / `pop_ready`). Suffix avoids colliding with older LIST keys.
            Priority::Medium | Priority::Low => {
                format!("{}:ready:{}:zq", self.namespace, prio.as_str())
            }
        }
    }

    fn scheduled_key(&self) -> String {
        format!("{}:scheduled", self.namespace)
    }

    fn running_key(&self) -> String {
        format!("{}:running", self.namespace)
    }

    fn ready_sequence_key(&self) -> String {
        format!("{}:ready:seq", self.namespace)
    }

    fn task_index_key(&self) -> String {
        format!("{}:tasks", self.namespace)
    }

    fn task_key_prefix(&self) -> String {
        format!("{}:task:", self.namespace)
    }

    async fn scan_task_index_ids_locked(
        conn: &mut redis::aio::MultiplexedConnection,
        prefix: &str,
    ) -> StoreResult<Vec<String>> {
        let pattern = format!("{prefix}*");
        let mut cursor = 0u64;
        let mut encoded_ids = Vec::new();
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(128)
                .query_async(conn)
                .await
                .map_err(TaskStoreError::backend)?;
            for key in keys {
                if let Some(encoded) = key.strip_prefix(prefix) {
                    encoded_ids.push(encoded.to_string());
                }
            }
            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }
        Ok(encoded_ids)
    }

    fn encode_id(id: &TaskId<Id>) -> StoreResult<String> {
        encode_task_id(id).map_err(TaskStoreError::corrupt_data)
    }

    fn decode_id(raw: &str) -> StoreResult<TaskId<Id>> {
        decode_task_id(raw).map_err(TaskStoreError::corrupt_data)
    }

    async fn remove_queue_memberships_locked(
        &self,
        conn: &mut redis::aio::MultiplexedConnection,
        task_key: &str,
        encoded: &str,
    ) -> StoreResult<()> {
        let ready_member: Option<String> = conn
            .hget(task_key, FIELD_READY_MEMBER)
            .await
            .map_err(TaskStoreError::backend)?;
        let running_member: Option<String> = conn
            .hget(task_key, FIELD_RUNNING_MEMBER)
            .await
            .map_err(TaskStoreError::backend)?;

        let ready_high = self.ready_key(Priority::High);
        let ready_medium = self.ready_key(Priority::Medium);
        let ready_low = self.ready_key(Priority::Low);
        let scheduled_key = self.scheduled_key();
        let running_key = self.running_key();

        let mut pipe = redis::pipe();
        pipe.cmd("LREM")
            .arg(&ready_high)
            .arg(0)
            .arg(encoded)
            .ignore();
        if let Some(member) = ready_member {
            pipe.cmd("ZREM").arg(&ready_medium).arg(&member).ignore();
            pipe.cmd("ZREM").arg(&ready_low).arg(&member).ignore();
        }
        pipe.cmd("ZREM").arg(&scheduled_key).arg(encoded).ignore();
        if let Some(member) = running_member {
            pipe.cmd("ZREM").arg(&running_key).arg(member).ignore();
        }
        pipe.cmd("HDEL")
            .arg(task_key)
            .arg(FIELD_READY_MEMBER)
            .arg(FIELD_RUNNING_MEMBER)
            .ignore()
            .query_async(conn)
            .await
            .map_err(TaskStoreError::backend)
    }
}

#[cfg(test)]
mod tests {
    use super::SET_STATE_IF_RUNNING_SCRIPT;

    #[test]
    fn set_state_if_running_script_writes_optional_payload_from_expected_args() {
        assert!(
            SET_STATE_IF_RUNNING_SCRIPT.contains("if ARGV[10] == '1' then"),
            "payload presence flag must use ARGV[10]"
        );
        assert!(
            SET_STATE_IF_RUNNING_SCRIPT.contains("redis.call('HSET', KEYS[1], ARGV[9], ARGV[11])"),
            "payload write must use ARGV[9] field and ARGV[11] bytes"
        );
        assert!(
            SET_STATE_IF_RUNNING_SCRIPT.contains("local set_count = tonumber(ARGV[12])"),
            "set_args count must remain after optional payload args"
        );
        assert!(
            SET_STATE_IF_RUNNING_SCRIPT.contains("local idx = 14"),
            "set_args must start at ARGV[14]"
        );
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

fn duration_millis_saturating(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
}

fn execution_policy_lease_duration_ms(execution_policy: &TaskExecutionPolicy) -> u64 {
    duration_millis_saturating(execution_policy.lease_duration)
}

fn i64_from_usize(value: usize, context: &str) -> StoreResult<i64> {
    i64::try_from(value)
        .map_err(|_| TaskStoreError::corrupt_msg(format!("{context} exceeds i64 range")))
}

fn i64_from_u64(value: u64, context: &str) -> StoreResult<i64> {
    i64::try_from(value)
        .map_err(|_| TaskStoreError::corrupt_msg(format!("{context} exceeds i64 range")))
}

fn nonnegative_i64_to_usize(value: i64) -> usize {
    usize::try_from(value.max(0)).unwrap_or(usize::MAX)
}

fn nonnegative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

fn bounded_i64_to_u32(value: i64) -> u32 {
    u32::try_from(value.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
}

async fn next_ready_sequence(
    conn: &mut redis::aio::MultiplexedConnection,
    key: &str,
) -> StoreResult<u64> {
    let sequence: i64 = redis::cmd("INCR")
        .arg(key)
        .query_async(conn)
        .await
        .map_err(TaskStoreError::backend)?;
    Ok(nonnegative_i64_to_u64(sequence))
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl<P, O, Id> TaskStore<P, O, Id> for RedisStore<P, O, Id>
where
    P: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Clone + Send + 'static,
    Id: ReadyQueueSort + Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
        execution_policy: TaskExecutionPolicy,
    ) -> StoreResult<bool> {
        let task_key = self.task_key(&id)?;
        let lease_duration_ms = execution_policy_lease_duration_ms(&execution_policy);
        let payload = bincode::serialize(&payload)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize payload: {e}")))?;
        let execution_policy = bincode::serialize(&execution_policy)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize execution policy: {e}")))?;

        let mut conn = self.conn.lock().await;
        let mut unresolved_deps = Vec::with_capacity(deps.len());
        let mut initial_state = STATE_PENDING;
        let mut failure_error = None;

        for dep in deps {
            let dep_state: Option<String> = conn
                .hget(self.task_key(&dep)?, FIELD_STATE)
                .await
                .map_err(TaskStoreError::backend)?;
            match dep_state.as_deref() {
                Some(STATE_SUCCEEDED) => {}
                Some(STATE_FAILED) => {
                    initial_state = STATE_FAILED;
                    failure_error = Some("dependency failed");
                    unresolved_deps.clear();
                    break;
                }
                Some(STATE_CANCELLED) => {
                    initial_state = STATE_CANCELLED;
                    unresolved_deps.clear();
                    break;
                }
                _ => unresolved_deps.push(dep),
            }
        }

        let remaining = if initial_state == STATE_PENDING {
            unresolved_deps.len()
        } else {
            0
        };
        let encoded_id = Self::encode_id(&id)?;
        let inserted: i64 = redis::Script::new(
            r"
local exists = redis.call('EXISTS', KEYS[1])
if exists == 1 then
    local state = redis.call('HGET', KEYS[1], ARGV[3])
    if state == 'failed' or state == 'cancelled' then
    redis.call('HSET', KEYS[1],
      ARGV[1], ARGV[2],
      ARGV[3], ARGV[4],
      ARGV[5], ARGV[6],
      ARGV[7], ARGV[8],
            ARGV[9], ARGV[10],
            ARGV[11], ARGV[12],
            ARGV[13], ARGV[14])
        redis.call('HDEL', KEYS[1], ARGV[15], ARGV[16], ARGV[17], ARGV[18], ARGV[19], ARGV[20])
        redis.call('SADD', KEYS[2], ARGV[22])
        return 2
    end
    return 0
end

redis.call('HSET', KEYS[1],
  ARGV[1], ARGV[2],
  ARGV[3], ARGV[4],
  ARGV[5], ARGV[6],
  ARGV[7], ARGV[8],
    ARGV[9], ARGV[10],
    ARGV[11], ARGV[12],
    ARGV[13], ARGV[14])
redis.call('SADD', KEYS[2], ARGV[22])
return 1
",
        )
        .key(&task_key)
        .key(self.task_index_key())
        .arg(FIELD_PRIORITY)
        .arg(prio.as_str())
        .arg(FIELD_STATE)
        .arg(initial_state)
        .arg(FIELD_REMAINING)
        .arg(i64_from_usize(remaining, FIELD_REMAINING)?)
        .arg(FIELD_PAYLOAD)
        .arg(payload)
        .arg(FIELD_ATTEMPT)
        .arg(0i64)
        .arg(FIELD_LEASE_DURATION_MS)
        .arg(i64_from_u64(lease_duration_ms, FIELD_LEASE_DURATION_MS)?)
        .arg(FIELD_EXECUTION_POLICY)
        .arg(execution_policy)
        .arg(FIELD_OUTPUT)
        .arg(FIELD_ERROR)
        .arg(FIELD_NEXT_READY_AT_MS)
        .arg(FIELD_WORKER)
        .arg(FIELD_LEASE_UNTIL_MS)
        .arg(FIELD_CAUSED_BY)
        .arg(FIELD_RUNNING_MEMBER)
        .arg(&encoded_id)
        .invoke_async(&mut *conn)
        .await
        .map_err(TaskStoreError::backend)?;

        if inserted == 0 {
            return Ok(false);
        }

        if inserted == 2 {
            self.remove_queue_memberships_locked(&mut conn, &task_key, &encoded_id)
                .await?;

            let pattern = format!("{}:dependents:*", self.namespace);
            let mut cursor = 0u64;
            loop {
                let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                    .arg(cursor)
                    .arg("MATCH")
                    .arg(&pattern)
                    .arg("COUNT")
                    .arg(128)
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
                for key in keys {
                    let _: () = redis::cmd("SREM")
                        .arg(key)
                        .arg(&encoded_id)
                        .query_async(&mut *conn)
                        .await
                        .map_err(TaskStoreError::backend)?;
                }
                if next_cursor == 0 {
                    break;
                }
                cursor = next_cursor;
            }
        }

        let rqp_hex = sort_prefix_hex(&id.0.ready_queue_sort_prefix());
        let _: () = redis::cmd("HSET")
            .arg(&task_key)
            .arg(FIELD_RQP_HEX)
            .arg(rqp_hex.as_str())
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        if let Some(error) = failure_error {
            let _: () = redis::cmd("HSET")
                .arg(&task_key)
                .arg(FIELD_ERROR)
                .arg(error)
                .query_async(&mut *conn)
                .await
                .map_err(TaskStoreError::backend)?;
        }

        for dep in unresolved_deps {
            let dep_key = self.dependents_key(&dep)?;
            let _: () = redis::cmd("SADD")
                .arg(dep_key)
                .arg(&encoded_id)
                .query_async(&mut *conn)
                .await
                .map_err(TaskStoreError::backend)?;
        }

        Ok(true)
    }

    async fn get_state(&self, id: &TaskId<Id>) -> StoreResult<Option<TaskState<O, Id>>> {
        let task_key = self.task_key(id)?;
        let mut conn = self.conn.lock().await;

        let state: Option<String> = conn
            .hget(&task_key, FIELD_STATE)
            .await
            .map_err(TaskStoreError::backend)?;
        let Some(state) = state else {
            return Ok(None);
        };

        match state.as_str() {
            STATE_PENDING => {
                let remaining: Option<i64> = conn
                    .hget(&task_key, FIELD_REMAINING)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let remaining = remaining.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing remaining_deps for pending task")
                })?;
                Ok(Some(TaskState::pending(nonnegative_i64_to_usize(
                    remaining,
                ))))
            }
            STATE_READY => Ok(Some(TaskState::Ready)),
            STATE_RUNNING => {
                let worker: Option<String> = conn
                    .hget(&task_key, FIELD_WORKER)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let worker = worker.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing worker for running task")
                })?;

                let attempt: Option<i64> = conn
                    .hget(&task_key, FIELD_ATTEMPT)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let attempt = attempt.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing attempt for running task")
                })?;

                Ok(Some(TaskState::Running {
                    worker,
                    attempt: bounded_i64_to_u32(attempt),
                }))
            }
            STATE_RETRYING => {
                let error: Option<String> = conn
                    .hget(&task_key, FIELD_ERROR)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let error = error.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing error for retrying task")
                })?;

                let attempt: Option<i64> = conn
                    .hget(&task_key, FIELD_ATTEMPT)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let attempt = attempt.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing attempt for retrying task")
                })?;

                let next_ready_at_ms: Option<i64> = conn
                    .hget(&task_key, FIELD_NEXT_READY_AT_MS)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let next_ready_at_ms = next_ready_at_ms.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing next_ready_at_ms for retrying task")
                })?;

                Ok(Some(TaskState::Retrying {
                    error,
                    attempt: bounded_i64_to_u32(attempt),
                    next_ready_at_ms: nonnegative_i64_to_u64(next_ready_at_ms),
                }))
            }
            STATE_SUCCEEDED => {
                let output: Option<Vec<u8>> = conn
                    .hget(&task_key, FIELD_OUTPUT)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let output = output.ok_or_else(|| {
                    TaskStoreError::corrupt_msg("missing output for succeeded task")
                })?;
                let output: O = bincode::deserialize(&output)
                    .map_err(|e| TaskStoreError::corrupt_msg(format!("deserialize output: {e}")))?;
                Ok(Some(TaskState::Succeeded { output }))
            }
            STATE_FAILED => {
                let error: Option<String> = conn
                    .hget(&task_key, FIELD_ERROR)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let error = error
                    .ok_or_else(|| TaskStoreError::corrupt_msg("missing error for failed task"))?;

                let caused_by: Option<String> = conn
                    .hget(&task_key, FIELD_CAUSED_BY)
                    .await
                    .map_err(TaskStoreError::backend)?;
                let caused_by_dep = match caused_by {
                    Some(s) => Some(Self::decode_id(&s)?),
                    None => None,
                };

                Ok(Some(TaskState::Failed {
                    error,
                    caused_by_dep,
                }))
            }
            STATE_CANCELLED => Ok(Some(TaskState::Cancelled)),
            other => Err(TaskStoreError::corrupt_msg(format!(
                "unknown task state: {other}"
            ))),
        }
    }

    async fn set_state(&self, id: &TaskId<Id>, state: TaskState<O, Id>) -> StoreResult<()> {
        let task_key = self.task_key(id)?;
        let mut conn = self.conn.lock().await;

        if !matches!(state, TaskState::Running { .. }) {
            let running_key = self.running_key();
            let encoded = Self::encode_id(id)?;
            self.remove_queue_memberships_locked(&mut conn, &task_key, &encoded)
                .await?;
            let _: () = redis::cmd("ZREM")
                .arg(running_key)
                .arg(encoded)
                .query_async(&mut *conn)
                .await
                .map_err(TaskStoreError::backend)?;
            let _: () = redis::cmd("HDEL")
                .arg(&task_key)
                .arg(FIELD_WORKER)
                .arg(FIELD_LEASE_UNTIL_MS)
                .arg(FIELD_RUNNING_MEMBER)
                .query_async(&mut *conn)
                .await
                .map_err(TaskStoreError::backend)?;
        }

        match state {
            TaskState::Pending { remaining_deps } => {
                let _: () = redis::cmd("HSET")
                    .arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_PENDING)
                    .arg(FIELD_REMAINING)
                    .arg(i64_from_usize(remaining_deps, FIELD_REMAINING)?)
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            TaskState::Ready => {
                let _: () = conn
                    .hset(&task_key, FIELD_STATE, STATE_READY)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            TaskState::Running { worker, attempt } => {
                let _: () = redis::cmd("HSET")
                    .arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_RUNNING)
                    .arg(FIELD_WORKER)
                    .arg(worker)
                    .arg(FIELD_ATTEMPT)
                    .arg(i64::from(attempt))
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            TaskState::Retrying {
                error,
                attempt,
                next_ready_at_ms,
            } => {
                let _: () = redis::cmd("HSET")
                    .arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_RETRYING)
                    .arg(FIELD_ERROR)
                    .arg(error)
                    .arg(FIELD_ATTEMPT)
                    .arg(i64::from(attempt))
                    .arg(FIELD_NEXT_READY_AT_MS)
                    .arg(i64_from_u64(next_ready_at_ms, FIELD_NEXT_READY_AT_MS)?)
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            TaskState::Succeeded { output } => {
                let output = bincode::serialize(&output)
                    .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize output: {e}")))?;
                let _: () = redis::cmd("HSET")
                    .arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_SUCCEEDED)
                    .arg(FIELD_OUTPUT)
                    .arg(output)
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            TaskState::Failed {
                error,
                caused_by_dep,
            } => {
                let mut cmd = redis::cmd("HSET");
                cmd.arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_FAILED)
                    .arg(FIELD_ERROR)
                    .arg(error);

                if let Some(dep) = caused_by_dep {
                    cmd.arg(FIELD_CAUSED_BY).arg(Self::encode_id(&dep)?);
                } else {
                    let _: () = conn
                        .hdel(&task_key, FIELD_CAUSED_BY)
                        .await
                        .map_err(TaskStoreError::backend)?;
                }

                let _: () = cmd
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            TaskState::Cancelled => {
                let _: () = conn
                    .hset(&task_key, FIELD_STATE, STATE_CANCELLED)
                    .await
                    .map_err(TaskStoreError::backend)?;
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
        let task_key = self.task_key(id)?;
        let running_key = self.running_key();
        let encoded = Self::encode_id(id)?;
        let payload = payload
            .map(|payload| {
                bincode::serialize(&payload)
                    .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize payload: {e}")))
            })
            .transpose()?;

        let mut set_args = Vec::new();
        let mut hdel_args = Vec::new();
        match state {
            TaskState::Ready => {
                set_args.push(FIELD_STATE.as_bytes().to_vec());
                set_args.push(STATE_READY.as_bytes().to_vec());
                hdel_args.extend([
                    FIELD_OUTPUT,
                    FIELD_ERROR,
                    FIELD_NEXT_READY_AT_MS,
                    FIELD_CAUSED_BY,
                    FIELD_READY_MEMBER,
                ]);
            }
            TaskState::Retrying {
                error,
                attempt,
                next_ready_at_ms,
            } => {
                set_args.push(FIELD_STATE.as_bytes().to_vec());
                set_args.push(STATE_RETRYING.as_bytes().to_vec());
                set_args.push(FIELD_ERROR.as_bytes().to_vec());
                set_args.push(error.into_bytes());
                set_args.push(FIELD_ATTEMPT.as_bytes().to_vec());
                set_args.push(i64::from(attempt).to_string().into_bytes());
                set_args.push(FIELD_NEXT_READY_AT_MS.as_bytes().to_vec());
                set_args.push(
                    i64_from_u64(next_ready_at_ms, FIELD_NEXT_READY_AT_MS)?
                        .to_string()
                        .into_bytes(),
                );
                hdel_args.extend([FIELD_OUTPUT, FIELD_CAUSED_BY, FIELD_READY_MEMBER]);
            }
            TaskState::Succeeded { output } => {
                let output = bincode::serialize(&output)
                    .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize output: {e}")))?;
                set_args.push(FIELD_STATE.as_bytes().to_vec());
                set_args.push(STATE_SUCCEEDED.as_bytes().to_vec());
                set_args.push(FIELD_OUTPUT.as_bytes().to_vec());
                set_args.push(output);
                hdel_args.extend([
                    FIELD_ERROR,
                    FIELD_NEXT_READY_AT_MS,
                    FIELD_CAUSED_BY,
                    FIELD_READY_MEMBER,
                ]);
            }
            TaskState::Failed {
                error,
                caused_by_dep,
            } => {
                set_args.push(FIELD_STATE.as_bytes().to_vec());
                set_args.push(STATE_FAILED.as_bytes().to_vec());
                set_args.push(FIELD_ERROR.as_bytes().to_vec());
                set_args.push(error.into_bytes());
                if let Some(dep) = caused_by_dep {
                    set_args.push(FIELD_CAUSED_BY.as_bytes().to_vec());
                    set_args.push(Self::encode_id(&dep)?.into_bytes());
                } else {
                    hdel_args.push(FIELD_CAUSED_BY);
                }
                hdel_args.extend([FIELD_OUTPUT, FIELD_NEXT_READY_AT_MS, FIELD_READY_MEMBER]);
            }
            TaskState::Cancelled => {
                set_args.push(FIELD_STATE.as_bytes().to_vec());
                set_args.push(STATE_CANCELLED.as_bytes().to_vec());
                hdel_args.extend([
                    FIELD_OUTPUT,
                    FIELD_ERROR,
                    FIELD_NEXT_READY_AT_MS,
                    FIELD_CAUSED_BY,
                    FIELD_READY_MEMBER,
                ]);
            }
            TaskState::Pending { .. } | TaskState::Running { .. } => {
                return Err(TaskStoreError::corrupt_msg(
                    "set_state_if_running only supports terminal, ready, or retrying states",
                ));
            }
        }

        let script = redis::Script::new(SET_STATE_IF_RUNNING_SCRIPT);

        let mut conn = self.conn.lock().await;
        let mut invocation = script.prepare_invoke();
        invocation
            .key(task_key)
            .key(running_key)
            .arg(FIELD_STATE)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_ATTEMPT)
            .arg(i64::from(attempt))
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(encoded)
            .arg(FIELD_PAYLOAD)
            .arg(if payload.is_some() { "1" } else { "0" })
            .arg(payload.unwrap_or_default())
            .arg(set_args.len() / 2)
            .arg(FIELD_RUNNING_MEMBER);
        for arg in set_args {
            invocation.arg(arg);
        }
        invocation.arg(hdel_args.len());
        for field in hdel_args {
            invocation.arg(field);
        }

        let updated: i64 = invocation
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(updated == 1)
    }

    async fn retry_now_if_running(
        &self,
        id: TaskId<Id>,
        worker: &str,
        attempt: u32,
        _priority: Priority,
        payload: P,
    ) -> StoreResult<bool> {
        let task_key = self.task_key(&id)?;
        let running_key = self.running_key();
        let ready_high = self.ready_key(Priority::High);
        let ready_medium = self.ready_key(Priority::Medium);
        let ready_low = self.ready_key(Priority::Low);
        let ready_sequence = self.ready_sequence_key();
        let encoded = Self::encode_id(&id)?;
        let payload = bincode::serialize(&payload)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize payload: {e}")))?;
        let script = redis::Script::new(
            r"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return 0
end

local current_worker = redis.call('HGET', KEYS[1], ARGV[3])
if current_worker ~= ARGV[4] then
  return 0
end

local current_attempt = redis.call('HGET', KEYS[1], ARGV[5])
if not current_attempt or tonumber(current_attempt) ~= tonumber(ARGV[6]) then
  return 0
end

local running_member = redis.call('HGET', KEYS[1], ARGV[7])
if not running_member then
  return redis.error_reply('missing running member')
end

redis.call('ZREM', KEYS[2], running_member)
redis.call('HSET', KEYS[1], ARGV[1], ARGV[18], ARGV[9], ARGV[10])
redis.call('HDEL', KEYS[1], ARGV[3], ARGV[8], ARGV[7], ARGV[11], ARGV[12], ARGV[13], ARGV[14], ARGV[15])

local prio = redis.call('HGET', KEYS[1], ARGV[16])
if prio == 'high' then
  redis.call('RPUSH', KEYS[3], ARGV[19])
elseif prio == 'medium' or prio == 'low' then
  local rqp = redis.call('HGET', KEYS[1], ARGV[17])
  if not rqp then
    return redis.error_reply('missing ready queue sort prefix')
  end
  local seq = redis.call('INCR', KEYS[6])
  local member = rqp .. string.format('%020d', seq) .. ARGV[19]
  redis.call('HSET', KEYS[1], ARGV[15], member)
  if prio == 'medium' then
    redis.call('ZADD', KEYS[4], 0, member)
  else
    redis.call('ZADD', KEYS[5], 0, member)
  end
else
  return redis.error_reply('unknown priority')
end

return 1
",
        );

        let mut conn = self.conn.lock().await;
        let updated: i64 = script
            .key(task_key)
            .key(running_key)
            .key(ready_high)
            .key(ready_medium)
            .key(ready_low)
            .key(ready_sequence)
            .arg(FIELD_STATE)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_ATTEMPT)
            .arg(i64::from(attempt))
            .arg(FIELD_RUNNING_MEMBER)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(FIELD_PAYLOAD)
            .arg(payload)
            .arg(FIELD_OUTPUT)
            .arg(FIELD_ERROR)
            .arg(FIELD_NEXT_READY_AT_MS)
            .arg(FIELD_CAUSED_BY)
            .arg(FIELD_READY_MEMBER)
            .arg(FIELD_PRIORITY)
            .arg(FIELD_RQP_HEX)
            .arg(STATE_READY)
            .arg(encoded)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(updated == 1)
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
        let task_key = self.task_key(&id)?;
        let running_key = self.running_key();
        let scheduled_key = self.scheduled_key();
        let encoded = Self::encode_id(&id)?;
        let payload = bincode::serialize(&payload)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize payload: {e}")))?;
        let script = redis::Script::new(
            r"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return 0
end

local current_worker = redis.call('HGET', KEYS[1], ARGV[3])
if current_worker ~= ARGV[4] then
  return 0
end

local current_attempt = redis.call('HGET', KEYS[1], ARGV[5])
if not current_attempt or tonumber(current_attempt) ~= tonumber(ARGV[6]) then
  return 0
end

local running_member = redis.call('HGET', KEYS[1], ARGV[7])
if not running_member then
  return redis.error_reply('missing running member')
end

redis.call('ZREM', KEYS[2], running_member)
redis.call('HSET', KEYS[1],
  ARGV[1], ARGV[18],
  ARGV[8], ARGV[9],
  ARGV[10], ARGV[11],
  ARGV[5], ARGV[6],
  ARGV[12], ARGV[13])
redis.call('HDEL', KEYS[1], ARGV[3], ARGV[14], ARGV[7], ARGV[15], ARGV[16], ARGV[17])
redis.call('ZADD', KEYS[3], ARGV[13], ARGV[19])
return 1
",
        );

        let mut conn = self.conn.lock().await;
        let updated: i64 = script
            .key(task_key)
            .key(running_key)
            .key(scheduled_key)
            .arg(FIELD_STATE)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_ATTEMPT)
            .arg(i64::from(attempt))
            .arg(FIELD_RUNNING_MEMBER)
            .arg(FIELD_PAYLOAD)
            .arg(payload)
            .arg(FIELD_ERROR)
            .arg(error)
            .arg(FIELD_NEXT_READY_AT_MS)
            .arg(i64_from_u64(next_ready_at_ms, FIELD_NEXT_READY_AT_MS)?)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(FIELD_OUTPUT)
            .arg(FIELD_CAUSED_BY)
            .arg(FIELD_READY_MEMBER)
            .arg(STATE_RETRYING)
            .arg(encoded)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(updated == 1)
    }

    async fn get_view(&self, id: &TaskId<Id>) -> StoreResult<Option<(TaskState<O, Id>, Priority)>> {
        let task_key = self.task_key(id)?;
        let mut conn = self.conn.lock().await;

        let prio: Option<String> = conn
            .hget(&task_key, FIELD_PRIORITY)
            .await
            .map_err(TaskStoreError::backend)?;
        let Some(prio) = prio else {
            return Ok(None);
        };
        let priority = Priority::parse(prio.as_str())
            .ok_or_else(|| TaskStoreError::corrupt_msg(format!("unknown priority: {prio}")))?;

        drop(conn);
        let Some(state) = self.get_state(id).await? else {
            return Ok(None);
        };
        Ok(Some((state, priority)))
    }

    async fn get_view_state(
        &self,
        id: &TaskId<Id>,
    ) -> StoreResult<Option<(TaskStateKind, Priority)>> {
        let task_key = self.task_key(id)?;
        let mut conn = self.conn.lock().await;
        let (state, priority): (Option<String>, Option<String>) = redis::cmd("HMGET")
            .arg(&task_key)
            .arg(FIELD_STATE)
            .arg(FIELD_PRIORITY)
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;
        let Some(state) = state else {
            return Ok(None);
        };
        let Some(priority) = priority else {
            return Ok(None);
        };
        let priority = Priority::parse(priority.as_str())
            .ok_or_else(|| TaskStoreError::corrupt_msg(format!("unknown priority: {priority}")))?;
        Ok(Some((task_state_kind(state.as_str())?, priority)))
    }

    async fn list_view_states(&self) -> StoreResult<Vec<(TaskId<Id>, TaskStateKind, Priority)>> {
        let prefix = self.task_key_prefix();
        let index_key = self.task_index_key();
        let mut conn = self.conn.lock().await;
        let mut encoded_ids: Vec<String> = conn
            .smembers(&index_key)
            .await
            .map_err(TaskStoreError::backend)?;
        let mut views = Vec::new();

        if encoded_ids.is_empty() {
            encoded_ids = Self::scan_task_index_ids_locked(&mut conn, &prefix).await?;
            if encoded_ids.is_empty() {
                return Ok(views);
            }
        }

        let mut batch = Vec::with_capacity(encoded_ids.len());
        for encoded in encoded_ids {
            let id = Self::decode_id(&encoded)?;
            batch.push((format!("{prefix}{encoded}"), encoded, id));
        }

        let mut pipe = redis::pipe();
        for (key, _, _) in &batch {
            pipe.cmd("HMGET")
                .arg(key)
                .arg(FIELD_STATE)
                .arg(FIELD_PRIORITY);
        }
        let rows: Vec<(Option<String>, Option<String>)> = pipe
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;
        let mut stale_ids = Vec::new();
        for ((_, encoded, id), (state, priority)) in batch.into_iter().zip(rows) {
            let Some(state) = state else {
                stale_ids.push(encoded);
                continue;
            };
            let Some(priority) = priority else {
                stale_ids.push(encoded);
                continue;
            };
            let priority = Priority::parse(priority.as_str()).ok_or_else(|| {
                TaskStoreError::corrupt_msg(format!("unknown priority: {priority}"))
            })?;
            views.push((id, task_state_kind(state.as_str())?, priority));
        }
        if !stale_ids.is_empty() {
            let _: () = redis::cmd("SREM")
                .arg(index_key)
                .arg(stale_ids)
                .query_async(&mut *conn)
                .await
                .map_err(TaskStoreError::backend)?;
        }
        Ok(views)
    }

    async fn dependents_of(&self, dep: &TaskId<Id>) -> StoreResult<Vec<TaskId<Id>>> {
        let dep_key = self.dependents_key(dep)?;
        let mut conn = self.conn.lock().await;
        let ids: Vec<String> = conn
            .smembers(dep_key)
            .await
            .map_err(TaskStoreError::backend)?;
        ids.into_iter().map(|s| Self::decode_id(&s)).collect()
    }

    async fn dec_remaining_deps(&self, id: &TaskId<Id>) -> StoreResult<usize> {
        let task_key = self.task_key(id)?;
        let mut conn = self.conn.lock().await;
        let remaining: i64 = redis::cmd("HINCRBY")
            .arg(task_key)
            .arg(FIELD_REMAINING)
            .arg(-1i64)
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(nonnegative_i64_to_usize(remaining))
    }

    async fn try_mark_ready(&self, id: &TaskId<Id>) -> StoreResult<Option<Priority>> {
        let task_key = self.task_key(id)?;
        let script = redis::Script::new(
            r"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return nil
end

local remaining = tonumber(redis.call('HGET', KEYS[1], ARGV[3]) or '0')
if remaining ~= 0 then
  return nil
end

redis.call('HSET', KEYS[1], ARGV[1], ARGV[4])
return redis.call('HGET', KEYS[1], ARGV[5])
",
        );

        let mut conn = self.conn.lock().await;
        let prio: Option<String> = script
            .key(task_key)
            .arg(FIELD_STATE)
            .arg(STATE_PENDING)
            .arg(FIELD_REMAINING)
            .arg(STATE_READY)
            .arg(FIELD_PRIORITY)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        let Some(prio) = prio else {
            return Ok(None);
        };

        let parsed = Priority::parse(prio.as_str())
            .ok_or_else(|| TaskStoreError::corrupt_msg(format!("unknown priority: {prio}")))?;
        Ok(Some(parsed))
    }

    async fn push_ready(&self, prio: Priority, id: TaskId<Id>) -> StoreResult<()> {
        let key = self.ready_key(prio);
        let encoded = Self::encode_id(&id)?;
        let mut conn = self.conn.lock().await;
        match prio {
            Priority::High => {
                let _: () = conn
                    .rpush(&key, encoded)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            Priority::Medium | Priority::Low => {
                let sequence = next_ready_sequence(&mut conn, &self.ready_sequence_key()).await?;
                let member = zset_member_from_encoded(&id, &encoded, sequence);
                let task_key = self.task_key(&id)?;
                let _: () = redis::pipe()
                    .cmd("ZADD")
                    .arg(&key)
                    .arg(0i64)
                    .arg(&member)
                    .ignore()
                    .cmd("HSET")
                    .arg(task_key)
                    .arg(FIELD_READY_MEMBER)
                    .arg(member)
                    .ignore()
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
        }

        Ok(())
    }

    async fn pop_ready(&self, prio: Priority) -> StoreResult<Option<TaskId<Id>>> {
        let key = self.ready_key(prio);
        let mut conn = self.conn.lock().await;
        match prio {
            Priority::High => {
                let id: Option<String> = conn
                    .lpop(&key, None)
                    .await
                    .map_err(TaskStoreError::backend)?;
                match id {
                    Some(raw) => Ok(Some(Self::decode_id(&raw)?)),
                    None => Ok(None),
                }
            }
            Priority::Medium | Priority::Low => {
                let popped: Vec<String> = redis::cmd("ZPOPMIN")
                    .arg(&key)
                    .arg(1i64)
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
                if popped.len() < 2 {
                    return Ok(None);
                }
                let member = &popped[0];
                let enc = encoded_from_zset_member(member).ok_or_else(|| {
                    TaskStoreError::corrupt_msg("ready zset member missing encoded task id suffix")
                })?;
                Ok(Some(Self::decode_id(enc)?))
            }
        }
    }

    async fn take_ready(
        &self,
        id: &TaskId<Id>,
        worker: &str,
    ) -> StoreResult<Option<(P, Priority, u32, TaskExecutionPolicy)>> {
        let task_key = self.task_key(id)?;
        let running_key = self.running_key();
        let encoded = Self::encode_id(id)?;

        let default_lease_ms = duration_millis_saturating(self.lease);
        let script = redis::Script::new(
            r"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return nil
end

local lease_duration_ms = tonumber(redis.call('HGET', KEYS[1], ARGV[12]) or ARGV[13])
local lease_until_ms = tonumber(ARGV[10]) + lease_duration_ms
	local seq = redis.call('INCR', KEYS[3])
	local running_member = string.format('%020d', seq) .. ARGV[11]
	redis.call('HSET', KEYS[1],
	  ARGV[1], ARGV[3],
	  ARGV[4], ARGV[5],
	  ARGV[9], lease_until_ms,
 ARGV[16], running_member)
            redis.call('HDEL', KEYS[1], ARGV[15])
	local attempt = redis.call('HINCRBY', KEYS[1], ARGV[8], 1)
	redis.call('ZADD', KEYS[2], lease_until_ms, running_member)

local payload = redis.call('HGET', KEYS[1], ARGV[6])
local priority = redis.call('HGET', KEYS[1], ARGV[7])
local execution_policy = redis.call('HGET', KEYS[1], ARGV[14])
return {payload, priority, attempt, execution_policy}
",
        );

        let mut conn = self.conn.lock().await;
        let result: Option<(Vec<u8>, String, i64, Vec<u8>)> = script
            .key(task_key)
            .key(running_key)
            .key(self.ready_sequence_key())
            .arg(FIELD_STATE)
            .arg(STATE_READY)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_PAYLOAD)
            .arg(FIELD_PRIORITY)
            .arg(FIELD_ATTEMPT)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(i64_from_u64(now_millis(), "now_ms")?)
            .arg(encoded)
            .arg(FIELD_LEASE_DURATION_MS)
            .arg(i64_from_u64(default_lease_ms, FIELD_LEASE_DURATION_MS)?)
            .arg(FIELD_EXECUTION_POLICY)
            .arg(FIELD_READY_MEMBER)
            .arg(FIELD_RUNNING_MEMBER)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        let Some((payload_bytes, prio, attempt, execution_policy_bytes)) = result else {
            return Ok(None);
        };
        let payload: P = bincode::deserialize(&payload_bytes)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("deserialize payload: {e}")))?;
        let prio = Priority::parse(prio.as_str())
            .ok_or_else(|| TaskStoreError::corrupt_msg(format!("unknown priority: {prio}")))?;
        let execution_policy: TaskExecutionPolicy = bincode::deserialize(&execution_policy_bytes)
            .map_err(|e| {
            TaskStoreError::corrupt_msg(format!("deserialize execution policy: {e}"))
        })?;
        Ok(Some((
            payload,
            prio,
            bounded_i64_to_u32(attempt),
            execution_policy,
        )))
    }

    async fn put_payload(&self, id: &TaskId<Id>, payload: P) -> StoreResult<()> {
        let task_key = self.task_key(id)?;
        let payload = bincode::serialize(&payload)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize payload: {e}")))?;
        let mut conn = self.conn.lock().await;
        let _: () = conn
            .hset(task_key, FIELD_PAYLOAD, payload)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(())
    }

    async fn renew_lease(&self, id: &TaskId<Id>, worker: &str, attempt: u32) -> StoreResult<bool> {
        let task_key = self.task_key(id)?;
        let running_key = self.running_key();
        let default_lease_ms = duration_millis_saturating(self.lease);

        let script = redis::Script::new(
            r"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return 0
end

local current_worker = redis.call('HGET', KEYS[1], ARGV[3])
if current_worker ~= ARGV[4] then
  return 0
end

local current_attempt = redis.call('HGET', KEYS[1], ARGV[5])
if not current_attempt or tonumber(current_attempt) ~= tonumber(ARGV[6]) then
  return 0
end

	local lease_duration_ms = tonumber(redis.call('HGET', KEYS[1], ARGV[10]) or ARGV[11])
	local lease_until_ms = tonumber(ARGV[8]) + lease_duration_ms
	local running_member = redis.call('HGET', KEYS[1], ARGV[9])
	if not running_member then
	  return redis.error_reply('missing running member')
	end
	redis.call('HSET', KEYS[1], ARGV[7], lease_until_ms)
	redis.call('ZADD', KEYS[2], lease_until_ms, running_member)
	return 1
",
        );

        let mut conn = self.conn.lock().await;
        let renewed: i64 = script
            .key(task_key)
            .key(running_key)
            .arg(FIELD_STATE)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_ATTEMPT)
            .arg(i64::from(attempt))
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(i64_from_u64(now_millis(), "now_ms")?)
            .arg(FIELD_RUNNING_MEMBER)
            .arg(FIELD_LEASE_DURATION_MS)
            .arg(i64_from_u64(default_lease_ms, FIELD_LEASE_DURATION_MS)?)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(renewed == 1)
    }

    async fn schedule(&self, id: TaskId<Id>, not_before_ms: u64) -> StoreResult<()> {
        let key = self.scheduled_key();
        let encoded = Self::encode_id(&id)?;
        let mut conn = self.conn.lock().await;
        let _: () = redis::cmd("ZADD")
            .arg(key)
            .arg(i64_from_u64(not_before_ms, "not_before_ms")?)
            .arg(encoded)
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(())
    }

    async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> StoreResult<usize> {
        let scheduled_key = self.scheduled_key();
        let ready_high = self.ready_key(Priority::High);
        let ready_medium = self.ready_key(Priority::Medium);
        let ready_low = self.ready_key(Priority::Low);
        let ready_sequence = self.ready_sequence_key();

        let script = redis::Script::new(
            r"
local ids = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
local moved = 0
for _, id in ipairs(ids) do
  redis.call('ZREM', KEYS[1], id)
  local task_key = ARGV[3] .. id
  local state = redis.call('HGET', task_key, ARGV[4])
  if state == ARGV[5] then
    redis.call('HSET', task_key, ARGV[4], ARGV[6])
    local prio = redis.call('HGET', task_key, ARGV[7])
	    if prio == 'high' then
	      redis.call('RPUSH', KEYS[2], id)
	      moved = moved + 1
	    elseif prio == 'medium' then
	      local rqp = redis.call('HGET', task_key, ARGV[8])
	      if not rqp then
	        return redis.error_reply('missing ready queue sort prefix')
	      end
	      local seq = redis.call('INCR', KEYS[5])
	      local member = rqp .. string.format('%020d', seq) .. id
	      redis.call('HSET', task_key, ARGV[9], member)
	      redis.call('ZADD', KEYS[3], 0, member)
	      moved = moved + 1
	    elseif prio == 'low' then
	      local rqp = redis.call('HGET', task_key, ARGV[8])
	      if not rqp then
	        return redis.error_reply('missing ready queue sort prefix')
	      end
	      local seq = redis.call('INCR', KEYS[5])
	      local member = rqp .. string.format('%020d', seq) .. id
	      redis.call('HSET', task_key, ARGV[9], member)
	      redis.call('ZADD', KEYS[4], 0, member)
	      moved = moved + 1
	    end
  end
end
return moved
",
        );

        let mut conn = self.conn.lock().await;
        let moved: i64 = script
            .key(scheduled_key)
            .key(ready_high)
            .key(ready_medium)
            .key(ready_low)
            .key(ready_sequence)
            .arg(i64_from_u64(now_ms, "now_ms")?)
            .arg(i64_from_usize(limit, "limit")?)
            .arg(self.task_key_prefix())
            .arg(FIELD_STATE)
            .arg(STATE_RETRYING)
            .arg(STATE_READY)
            .arg(FIELD_PRIORITY)
            .arg(FIELD_RQP_HEX)
            .arg(FIELD_READY_MEMBER)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(nonnegative_i64_to_usize(moved))
    }

    async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> StoreResult<usize> {
        let running_key = self.running_key();
        let ready_high = self.ready_key(Priority::High);
        let ready_medium = self.ready_key(Priority::Medium);
        let ready_low = self.ready_key(Priority::Low);
        let ready_sequence = self.ready_sequence_key();

        let script = redis::Script::new(
            r"
	local members = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
	local moved = 0
	for _, running_member in ipairs(members) do
	  redis.call('ZREM', KEYS[1], running_member)
	  local id = string.sub(running_member, 21)
	  local task_key = ARGV[3] .. id
	  local state = redis.call('HGET', task_key, ARGV[4])
	  if state == ARGV[5] then
	    redis.call('HSET', task_key, ARGV[4], ARGV[6])
	    redis.call('HDEL', task_key, ARGV[7], ARGV[8], ARGV[12])
    local prio = redis.call('HGET', task_key, ARGV[9])
	    if prio == 'high' then
	      redis.call('RPUSH', KEYS[2], id)
	      moved = moved + 1
	    elseif prio == 'medium' then
	      local rqp = redis.call('HGET', task_key, ARGV[10])
	      if not rqp then
	        return redis.error_reply('missing ready queue sort prefix')
	      end
	      local seq = redis.call('INCR', KEYS[5])
	      local member = rqp .. string.format('%020d', seq) .. id
	      redis.call('HSET', task_key, ARGV[11], member)
	      redis.call('ZADD', KEYS[3], 0, member)
	      moved = moved + 1
	    elseif prio == 'low' then
	      local rqp = redis.call('HGET', task_key, ARGV[10])
	      if not rqp then
	        return redis.error_reply('missing ready queue sort prefix')
	      end
	      local seq = redis.call('INCR', KEYS[5])
	      local member = rqp .. string.format('%020d', seq) .. id
	      redis.call('HSET', task_key, ARGV[11], member)
	      redis.call('ZADD', KEYS[4], 0, member)
	      moved = moved + 1
	    end
  end
end
return moved
",
        );

        let mut conn = self.conn.lock().await;
        let moved: i64 = script
            .key(running_key)
            .key(ready_high)
            .key(ready_medium)
            .key(ready_low)
            .key(ready_sequence)
            .arg(i64_from_u64(now_ms, "now_ms")?)
            .arg(i64_from_usize(limit, "limit")?)
            .arg(self.task_key_prefix())
            .arg(FIELD_STATE)
            .arg(STATE_RUNNING)
            .arg(STATE_READY)
            .arg(FIELD_WORKER)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(FIELD_PRIORITY)
            .arg(FIELD_RQP_HEX)
            .arg(FIELD_READY_MEMBER)
            .arg(FIELD_RUNNING_MEMBER)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(nonnegative_i64_to_usize(moved))
    }

    async fn remove_task(&self, id: &TaskId<Id>) -> StoreResult<bool> {
        let encoded = Self::encode_id(id)?;
        let task_key = self.task_key(id)?;
        let dependents_key = self.dependents_key(id)?;
        let task_index_key = self.task_index_key();
        let pattern = format!("{}:dependents:*", self.namespace);

        let mut conn = self.conn.lock().await;
        let existed: i64 = redis::cmd("EXISTS")
            .arg(&task_key)
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        self.remove_queue_memberships_locked(&mut conn, &task_key, &encoded)
            .await?;

        let _: () = redis::pipe()
            .cmd("DEL")
            .arg(&task_key)
            .arg(&dependents_key)
            .ignore()
            .cmd("SREM")
            .arg(&task_index_key)
            .arg(&encoded)
            .ignore()
            .query_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        let mut cursor = 0u64;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(128)
                .query_async(&mut *conn)
                .await
                .map_err(TaskStoreError::backend)?;
            for key in keys {
                let _: () = redis::cmd("SREM")
                    .arg(key)
                    .arg(&encoded)
                    .query_async(&mut *conn)
                    .await
                    .map_err(TaskStoreError::backend)?;
            }
            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        Ok(existed == 1)
    }
}
