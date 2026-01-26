use crate::{
    Priority, StoreResult, TaskId, TaskState, TaskStore, TaskStoreError, decode_task_id,
    encode_task_id,
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

const STATE_PENDING: &str = "pending";
const STATE_READY: &str = "ready";
const STATE_RUNNING: &str = "running";
const STATE_RETRYING: &str = "retrying";
const STATE_SUCCEEDED: &str = "succeeded";
const STATE_FAILED: &str = "failed";
const STATE_CANCELLED: &str = "cancelled";

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
        Ok(format!("{}:task:{}", self.namespace, self.encode_id(id)?))
    }

    fn dependents_key(&self, id: &TaskId<Id>) -> StoreResult<String> {
        Ok(format!(
            "{}:dependents:{}",
            self.namespace,
            self.encode_id(id)?
        ))
    }

    fn ready_key(&self, prio: Priority) -> String {
        format!("{}:ready:{}", self.namespace, prio.as_str())
    }

    fn scheduled_key(&self) -> String {
        format!("{}:scheduled", self.namespace)
    }

    fn running_key(&self) -> String {
        format!("{}:running", self.namespace)
    }

    fn task_key_prefix(&self) -> String {
        format!("{}:task:", self.namespace)
    }

    fn encode_id(&self, id: &TaskId<Id>) -> StoreResult<String> {
        encode_task_id(id).map_err(TaskStoreError::corrupt_data)
    }

    fn decode_id(&self, raw: &str) -> StoreResult<TaskId<Id>> {
        decode_task_id(raw).map_err(TaskStoreError::corrupt_data)
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

#[async_trait]
impl<P, O, Id> TaskStore<P, O, Id> for RedisStore<P, O, Id>
where
    P: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Clone + Send + 'static,
    Id: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    async fn insert_task(
        &self,
        id: TaskId<Id>,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId<Id>>,
    ) -> StoreResult<bool> {
        let task_key = self.task_key(&id)?;
        let payload = bincode::serialize(&payload)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("serialize payload: {e}")))?;

        let mut conn = self.conn.lock().await;
        let inserted: i64 = redis::Script::new(
            r#"
if redis.call('EXISTS', KEYS[1]) == 1 then
  return 0
end

redis.call('HSET', KEYS[1],
  ARGV[1], ARGV[2],
  ARGV[3], ARGV[4],
  ARGV[5], ARGV[6],
  ARGV[7], ARGV[8],
  ARGV[9], ARGV[10])
return 1
"#,
        )
        .key(&task_key)
        .arg(FIELD_PRIORITY)
        .arg(prio.as_str())
        .arg(FIELD_STATE)
        .arg(STATE_PENDING)
        .arg(FIELD_REMAINING)
        .arg(deps.len() as i64)
        .arg(FIELD_PAYLOAD)
        .arg(payload)
        .arg(FIELD_ATTEMPT)
        .arg(0i64)
        .invoke_async(&mut *conn)
        .await
        .map_err(TaskStoreError::backend)?;

        if inserted == 0 {
            return Ok(false);
        }

        for dep in deps {
            let dep_key = self.dependents_key(&dep)?;
            let _: () = redis::cmd("SADD")
                .arg(dep_key)
                .arg(self.encode_id(&id)?)
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
                Ok(Some(TaskState::pending(remaining.max(0) as usize)))
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
                    attempt: attempt.max(0).min(u32::MAX as i64) as u32,
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
                    attempt: attempt.max(0) as u32,
                    next_ready_at_ms: next_ready_at_ms.max(0) as u64,
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
                    Some(s) => Some(self.decode_id(&s)?),
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
            let encoded = self.encode_id(id)?;
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
                    .arg(remaining_deps as i64)
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
                    .arg(attempt as i64)
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
                    .arg(attempt as i64)
                    .arg(FIELD_NEXT_READY_AT_MS)
                    .arg(next_ready_at_ms as i64)
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
                    cmd.arg(FIELD_CAUSED_BY).arg(self.encode_id(&dep)?);
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

    async fn dependents_of(&self, dep: &TaskId<Id>) -> StoreResult<Vec<TaskId<Id>>> {
        let dep_key = self.dependents_key(dep)?;
        let mut conn = self.conn.lock().await;
        let ids: Vec<String> = conn
            .smembers(dep_key)
            .await
            .map_err(TaskStoreError::backend)?;
        ids.into_iter().map(|s| self.decode_id(&s)).collect()
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

        Ok(remaining.max(0) as usize)
    }

    async fn try_mark_ready(&self, id: &TaskId<Id>) -> StoreResult<Option<Priority>> {
        let task_key = self.task_key(id)?;
        let script = redis::Script::new(
            r#"
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
"#,
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
        let encoded = self.encode_id(&id)?;
        let mut conn = self.conn.lock().await;
        let _: () = conn
            .rpush(key, encoded)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(())
    }

    async fn pop_ready(&self, prio: Priority) -> StoreResult<Option<TaskId<Id>>> {
        let key = self.ready_key(prio);
        let mut conn = self.conn.lock().await;
        let id: Option<String> = conn
            .lpop(key, None)
            .await
            .map_err(TaskStoreError::backend)?;
        match id {
            Some(raw) => Ok(Some(self.decode_id(&raw)?)),
            None => Ok(None),
        }
    }

    async fn pop_ready_and_take(
        &self,
        prio: Priority,
        worker: &str,
    ) -> StoreResult<Option<(TaskId<Id>, P, Priority, u32)>> {
        let ready_key = self.ready_key(prio);
        let running_key = self.running_key();

        let lease_ms = self.lease.as_millis().min(u64::MAX as u128) as u64;
        let lease_until_ms = now_millis().saturating_add(lease_ms);
        let script = redis::Script::new(
            r#"
while true do
  local id = redis.call('LPOP', KEYS[1])
  if not id then
    return nil
  end

  local task_key = ARGV[1] .. id
  local state = redis.call('HGET', task_key, ARGV[2])
  if state == ARGV[3] then
    redis.call('HSET', task_key, ARGV[2], ARGV[4], ARGV[5], ARGV[6], ARGV[7], ARGV[8])
    local attempt = redis.call('HINCRBY', task_key, ARGV[9], 1)
    redis.call('ZADD', KEYS[2], ARGV[8], id)

    local payload = redis.call('HGET', task_key, ARGV[10])
    local priority = redis.call('HGET', task_key, ARGV[11])
    return {id, payload, priority, attempt}
  end
end
"#,
        );

        let mut conn = self.conn.lock().await;
        let result: Option<(String, Vec<u8>, String, i64)> = script
            .key(ready_key)
            .key(running_key)
            .arg(self.task_key_prefix())
            .arg(FIELD_STATE)
            .arg(STATE_READY)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(lease_until_ms as i64)
            .arg(FIELD_ATTEMPT)
            .arg(FIELD_PAYLOAD)
            .arg(FIELD_PRIORITY)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        let Some((id, payload_bytes, prio, attempt)) = result else {
            return Ok(None);
        };
        let id = self.decode_id(&id)?;
        let payload: P = bincode::deserialize(&payload_bytes)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("deserialize payload: {e}")))?;
        let prio = Priority::parse(prio.as_str())
            .ok_or_else(|| TaskStoreError::corrupt_msg(format!("unknown priority: {prio}")))?;
        Ok(Some((id, payload, prio, attempt.max(0) as u32)))
    }

    async fn take_ready(
        &self,
        id: &TaskId<Id>,
        worker: &str,
    ) -> StoreResult<Option<(P, Priority, u32)>> {
        let task_key = self.task_key(id)?;
        let running_key = self.running_key();
        let encoded = self.encode_id(id)?;

        let lease_ms = self.lease.as_millis().min(u64::MAX as u128) as u64;
        let lease_until_ms = now_millis().saturating_add(lease_ms);
        let script = redis::Script::new(
            r#"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return nil
end

redis.call('HSET', KEYS[1], ARGV[1], ARGV[3], ARGV[4], ARGV[5], ARGV[9], ARGV[10])
local attempt = redis.call('HINCRBY', KEYS[1], ARGV[8], 1)
redis.call('ZADD', KEYS[2], ARGV[10], ARGV[11])

local payload = redis.call('HGET', KEYS[1], ARGV[6])
local priority = redis.call('HGET', KEYS[1], ARGV[7])
return {payload, priority, attempt}
"#,
        );

        let mut conn = self.conn.lock().await;
        let result: Option<(Vec<u8>, String, i64)> = script
            .key(task_key)
            .key(running_key)
            .arg(FIELD_STATE)
            .arg(STATE_READY)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_PAYLOAD)
            .arg(FIELD_PRIORITY)
            .arg(FIELD_ATTEMPT)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(lease_until_ms as i64)
            .arg(encoded)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        let Some((payload_bytes, prio, attempt)) = result else {
            return Ok(None);
        };
        let payload: P = bincode::deserialize(&payload_bytes)
            .map_err(|e| TaskStoreError::corrupt_msg(format!("deserialize payload: {e}")))?;
        let prio = Priority::parse(prio.as_str())
            .ok_or_else(|| TaskStoreError::corrupt_msg(format!("unknown priority: {prio}")))?;
        Ok(Some((payload, prio, attempt.max(0) as u32)))
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

    async fn schedule(&self, id: TaskId<Id>, not_before_ms: u64) -> StoreResult<()> {
        let key = self.scheduled_key();
        let encoded = self.encode_id(&id)?;
        let mut conn = self.conn.lock().await;
        let _: () = redis::cmd("ZADD")
            .arg(key)
            .arg(not_before_ms as i64)
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

        let script = redis::Script::new(
            r#"
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
      redis.call('RPUSH', KEYS[3], id)
      moved = moved + 1
    elseif prio == 'low' then
      redis.call('RPUSH', KEYS[4], id)
      moved = moved + 1
    end
  end
end
return moved
"#,
        );

        let mut conn = self.conn.lock().await;
        let moved: i64 = script
            .key(scheduled_key)
            .key(ready_high)
            .key(ready_medium)
            .key(ready_low)
            .arg(now_ms as i64)
            .arg(limit as i64)
            .arg(self.task_key_prefix())
            .arg(FIELD_STATE)
            .arg(STATE_RETRYING)
            .arg(STATE_READY)
            .arg(FIELD_PRIORITY)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(moved.max(0) as usize)
    }

    async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> StoreResult<usize> {
        let running_key = self.running_key();
        let ready_high = self.ready_key(Priority::High);
        let ready_medium = self.ready_key(Priority::Medium);
        let ready_low = self.ready_key(Priority::Low);

        let script = redis::Script::new(
            r#"
local ids = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
local moved = 0
for _, id in ipairs(ids) do
  redis.call('ZREM', KEYS[1], id)
  local task_key = ARGV[3] .. id
  local state = redis.call('HGET', task_key, ARGV[4])
  if state == ARGV[5] then
    redis.call('HSET', task_key, ARGV[4], ARGV[6])
    redis.call('HDEL', task_key, ARGV[7], ARGV[8])
    local prio = redis.call('HGET', task_key, ARGV[9])
    if prio == 'high' then
      redis.call('RPUSH', KEYS[2], id)
      moved = moved + 1
    elseif prio == 'medium' then
      redis.call('RPUSH', KEYS[3], id)
      moved = moved + 1
    elseif prio == 'low' then
      redis.call('RPUSH', KEYS[4], id)
      moved = moved + 1
    end
  end
end
return moved
"#,
        );

        let mut conn = self.conn.lock().await;
        let moved: i64 = script
            .key(running_key)
            .key(ready_high)
            .key(ready_medium)
            .key(ready_low)
            .arg(now_ms as i64)
            .arg(limit as i64)
            .arg(self.task_key_prefix())
            .arg(FIELD_STATE)
            .arg(STATE_RUNNING)
            .arg(STATE_READY)
            .arg(FIELD_WORKER)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(FIELD_PRIORITY)
            .invoke_async(&mut *conn)
            .await
            .map_err(TaskStoreError::backend)?;

        Ok(moved.max(0) as usize)
    }
}
