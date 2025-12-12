use crate::{Priority, TaskId, TaskKind, TaskState, TaskStore};
use async_trait::async_trait;
use redis::AsyncCommands;
use serde::{Serialize, de::DeserializeOwned};
use std::{marker::PhantomData, time::Duration};
use tokio::sync::Mutex;

const FIELD_KIND: &str = "kind";
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

pub struct RedisStore<P, O> {
    conn: Mutex<redis::aio::MultiplexedConnection>,
    namespace: String,
    lease: Duration,
    _phantom: PhantomData<fn(P, O)>,
}

impl<P, O> RedisStore<P, O>
where
    P: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Clone + Send + 'static,
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

    fn task_key(&self, id: &TaskId) -> String {
        format!("{}:task:{}", self.namespace, id)
    }

    fn dependents_key(&self, id: &TaskId) -> String {
        format!("{}:dependents:{}", self.namespace, id)
    }

    fn ready_key(&self, prio: Priority) -> String {
        format!("{}:ready:{}", self.namespace, prio_str(prio))
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
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn prio_str(prio: Priority) -> &'static str {
    match prio {
        Priority::High => "high",
        Priority::Medium => "medium",
        Priority::Low => "low",
    }
}

fn parse_prio(s: &str) -> Option<Priority> {
    match s {
        "high" => Some(Priority::High),
        "medium" => Some(Priority::Medium),
        "low" => Some(Priority::Low),
        _ => None,
    }
}

fn kind_str(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Preflight => "preflight",
        TaskKind::BuildGuestInput => "build_guest_input",
        TaskKind::BatchProof => "batch_proof",
        TaskKind::Aggregation => "aggregation",
    }
}

fn parse_kind(s: &str) -> Option<TaskKind> {
    match s {
        "preflight" => Some(TaskKind::Preflight),
        "build_guest_input" => Some(TaskKind::BuildGuestInput),
        "batch_proof" => Some(TaskKind::BatchProof),
        "aggregation" => Some(TaskKind::Aggregation),
        _ => None,
    }
}

#[async_trait]
impl<P, O> TaskStore<P, O> for RedisStore<P, O>
where
    P: Serialize + DeserializeOwned + Send + 'static,
    O: Serialize + DeserializeOwned + Clone + Send + 'static,
{
    async fn insert_task(
        &self,
        id: TaskId,
        kind: TaskKind,
        payload: P,
        prio: Priority,
        deps: Vec<TaskId>,
    ) {
        let task_key = self.task_key(&id);
        let payload = bincode::serialize(&payload).expect("serialize payload");

        let mut conn = self.conn.lock().await;
        let _: () = redis::cmd("HSET")
            .arg(&task_key)
            .arg(FIELD_KIND)
            .arg(kind_str(kind))
            .arg(FIELD_PRIORITY)
            .arg(prio_str(prio))
            .arg(FIELD_STATE)
            .arg(STATE_PENDING)
            .arg(FIELD_REMAINING)
            .arg(deps.len() as i64)
            .arg(FIELD_PAYLOAD)
            .arg(payload)
            .arg(FIELD_ATTEMPT)
            .arg(0i64)
            .query_async(&mut *conn)
            .await
            .expect("redis HSET task");

        for dep in deps {
            let dep_key = self.dependents_key(&dep);
            let _: () = redis::cmd("SADD")
                .arg(dep_key)
                .arg(id.to_string())
                .query_async(&mut *conn)
                .await
                .expect("redis SADD dependent");
        }
    }

    async fn get_state(&self, id: &TaskId) -> Option<TaskState<O>> {
        let task_key = self.task_key(id);
        let mut conn = self.conn.lock().await;

        let state: Option<String> = conn.hget(&task_key, FIELD_STATE).await.ok()?;
        let state = state?;

        match state.as_str() {
            STATE_PENDING => {
                let remaining: i64 = conn.hget(&task_key, FIELD_REMAINING).await.ok()?;
                Some(TaskState::pending(remaining.max(0) as usize))
            }
            STATE_READY => Some(TaskState::Ready),
            STATE_RUNNING => {
                let worker: String = conn.hget(&task_key, FIELD_WORKER).await.ok()?;
                Some(TaskState::Running { worker })
            }
            STATE_RETRYING => {
                let error: String = conn.hget(&task_key, FIELD_ERROR).await.ok()?;
                let attempt: i64 = conn.hget(&task_key, FIELD_ATTEMPT).await.ok()?;
                let next_ready_at_ms: i64 =
                    conn.hget(&task_key, FIELD_NEXT_READY_AT_MS).await.ok()?;
                Some(TaskState::Retrying {
                    error,
                    attempt: attempt.max(0) as u32,
                    next_ready_at_ms: next_ready_at_ms.max(0) as u64,
                })
            }
            STATE_SUCCEEDED => {
                let output: Vec<u8> = conn.hget(&task_key, FIELD_OUTPUT).await.ok()?;
                let output: O = bincode::deserialize(&output).ok()?;
                Some(TaskState::Succeeded { output })
            }
            STATE_FAILED => {
                let error: String = conn.hget(&task_key, FIELD_ERROR).await.ok()?;
                let caused_by: Option<String> = conn.hget(&task_key, FIELD_CAUSED_BY).await.ok()?;
                let caused_by_dep = caused_by.and_then(|s| s.parse().ok());
                Some(TaskState::Failed {
                    error,
                    caused_by_dep,
                })
            }
            STATE_CANCELLED => Some(TaskState::Cancelled),
            _ => None,
        }
    }

    async fn set_state(&self, id: &TaskId, state: TaskState<O>) {
        let task_key = self.task_key(id);
        let mut conn = self.conn.lock().await;

        if !matches!(state, TaskState::Running { .. }) {
            let running_key = self.running_key();
            let _: () = redis::cmd("ZREM")
                .arg(running_key)
                .arg(id.to_string())
                .query_async(&mut *conn)
                .await
                .expect("redis ZREM running");
            let _: () = redis::cmd("HDEL")
                .arg(&task_key)
                .arg(FIELD_WORKER)
                .arg(FIELD_LEASE_UNTIL_MS)
                .query_async(&mut *conn)
                .await
                .expect("redis HDEL running fields");
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
                    .expect("redis HSET pending");
            }
            TaskState::Ready => {
                let _: () = conn
                    .hset(&task_key, FIELD_STATE, STATE_READY)
                    .await
                    .expect("redis HSET ready");
            }
            TaskState::Running { worker } => {
                let _: () = redis::cmd("HSET")
                    .arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_RUNNING)
                    .arg(FIELD_WORKER)
                    .arg(worker)
                    .query_async(&mut *conn)
                    .await
                    .expect("redis HSET running");
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
                    .expect("redis HSET retrying");
            }
            TaskState::Succeeded { output } => {
                let output = bincode::serialize(&output).expect("serialize output");
                let _: () = redis::cmd("HSET")
                    .arg(&task_key)
                    .arg(FIELD_STATE)
                    .arg(STATE_SUCCEEDED)
                    .arg(FIELD_OUTPUT)
                    .arg(output)
                    .query_async(&mut *conn)
                    .await
                    .expect("redis HSET succeeded");
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
                    cmd.arg(FIELD_CAUSED_BY).arg(dep.to_string());
                } else {
                    let _: () = conn
                        .hdel(&task_key, FIELD_CAUSED_BY)
                        .await
                        .expect("redis HDEL caused_by_dep");
                }

                let _: () = cmd
                    .query_async(&mut *conn)
                    .await
                    .expect("redis HSET failed");
            }
            TaskState::Cancelled => {
                let _: () = conn
                    .hset(&task_key, FIELD_STATE, STATE_CANCELLED)
                    .await
                    .expect("redis HSET cancelled");
            }
        }
    }

    async fn get_view(&self, id: &TaskId) -> Option<(TaskState<O>, TaskKind, Priority)> {
        let task_key = self.task_key(id);
        let mut conn = self.conn.lock().await;

        let kind: Option<String> = conn.hget(&task_key, FIELD_KIND).await.ok()?;
        let kind = parse_kind(kind?.as_str())?;

        let prio: Option<String> = conn.hget(&task_key, FIELD_PRIORITY).await.ok()?;
        let priority = parse_prio(prio?.as_str())?;

        drop(conn);
        let state = self.get_state(id).await?;
        Some((state, kind, priority))
    }

    async fn dependents_of(&self, dep: &TaskId) -> Vec<TaskId> {
        let dep_key = self.dependents_key(dep);
        let mut conn = self.conn.lock().await;
        let ids: Vec<String> = conn.smembers(dep_key).await.unwrap_or_default();
        ids.into_iter().filter_map(|s| s.parse().ok()).collect()
    }

    async fn dec_remaining_deps(&self, id: &TaskId) -> usize {
        let task_key = self.task_key(id);
        let mut conn = self.conn.lock().await;
        let remaining: i64 = redis::cmd("HINCRBY")
            .arg(task_key)
            .arg(FIELD_REMAINING)
            .arg(-1i64)
            .query_async(&mut *conn)
            .await
            .expect("redis HINCRBY remaining_deps");

        remaining.max(0) as usize
    }

    async fn try_mark_ready(&self, id: &TaskId) -> Option<Priority> {
        let task_key = self.task_key(id);
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
            .ok()?;

        parse_prio(prio?.as_str())
    }

    async fn push_ready(&self, prio: Priority, id: TaskId) {
        let key = self.ready_key(prio);
        let mut conn = self.conn.lock().await;
        let _: () = conn
            .rpush(key, id.to_string())
            .await
            .expect("redis RPUSH ready");
    }

    async fn pop_ready(&self, prio: Priority) -> Option<TaskId> {
        let key = self.ready_key(prio);
        let mut conn = self.conn.lock().await;
        let id: Option<String> = conn.lpop(key, None).await.ok()?;
        id.and_then(|s| s.parse().ok())
    }

    async fn take_ready(&self, id: &TaskId, worker: &str) -> Option<(P, TaskKind, Priority, u32)> {
        let task_key = self.task_key(id);
        let running_key = self.running_key();

        let lease_ms = self.lease.as_millis().min(u64::MAX as u128) as u64;
        let lease_until_ms = now_millis().saturating_add(lease_ms);
        let script = redis::Script::new(
            r#"
local state = redis.call('HGET', KEYS[1], ARGV[1])
if state ~= ARGV[2] then
  return nil
end

redis.call('HSET', KEYS[1], ARGV[1], ARGV[3], ARGV[4], ARGV[5], ARGV[10], ARGV[11])
local attempt = redis.call('HINCRBY', KEYS[1], ARGV[9], 1)
redis.call('ZADD', KEYS[2], ARGV[11], ARGV[12])

local payload = redis.call('HGET', KEYS[1], ARGV[6])
local kind = redis.call('HGET', KEYS[1], ARGV[7])
local priority = redis.call('HGET', KEYS[1], ARGV[8])
return {payload, kind, priority, attempt}
"#,
        );

        let mut conn = self.conn.lock().await;
        let result: Option<(Vec<u8>, String, String, i64)> = script
            .key(task_key)
            .key(running_key)
            .arg(FIELD_STATE)
            .arg(STATE_READY)
            .arg(STATE_RUNNING)
            .arg(FIELD_WORKER)
            .arg(worker)
            .arg(FIELD_PAYLOAD)
            .arg(FIELD_KIND)
            .arg(FIELD_PRIORITY)
            .arg(FIELD_ATTEMPT)
            .arg(FIELD_LEASE_UNTIL_MS)
            .arg(lease_until_ms as i64)
            .arg(id.to_string())
            .invoke_async(&mut *conn)
            .await
            .ok()?;

        let (payload_bytes, kind, prio, attempt) = result?;
        let payload: P = bincode::deserialize(&payload_bytes).ok()?;
        Some((
            payload,
            parse_kind(kind.as_str())?,
            parse_prio(prio.as_str())?,
            attempt.max(0) as u32,
        ))
    }

    async fn put_payload(&self, id: &TaskId, payload: P) {
        let task_key = self.task_key(id);
        let payload = bincode::serialize(&payload).expect("serialize payload");
        let mut conn = self.conn.lock().await;
        let _: () = conn
            .hset(task_key, FIELD_PAYLOAD, payload)
            .await
            .expect("redis HSET payload");
    }

    async fn schedule(&self, id: TaskId, not_before_ms: u64) {
        let key = self.scheduled_key();
        let mut conn = self.conn.lock().await;
        let _: () = redis::cmd("ZADD")
            .arg(key)
            .arg(not_before_ms as i64)
            .arg(id.to_string())
            .query_async(&mut *conn)
            .await
            .expect("redis ZADD scheduled");
    }

    async fn promote_scheduled(&self, now_ms: u64, limit: usize) -> usize {
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
            .unwrap_or(0);

        moved.max(0) as usize
    }

    async fn requeue_expired_leases(&self, now_ms: u64, limit: usize) -> usize {
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
            .unwrap_or(0);

        moved.max(0) as usize
    }
}
