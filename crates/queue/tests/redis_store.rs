//! Redis-backed queue store integration tests.
#![cfg(feature = "redis")]

use std::process::Command;
use std::time::Duration;

use raiko2_queue::{
    NewTask, Priority, ReadyQueueSort, RedisStore, RetryPolicy, Scheduler, SchedulerConfig, TaskId,
    TaskState, encode_task_id,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use testcontainers::{
    GenericImage,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct EqualSortId(u64);

impl ReadyQueueSort for EqualSortId {
    fn ready_queue_sort_prefix(&self) -> [u8; 16] {
        [7u8; 16]
    }
}

#[tokio::test]
async fn redis_store_persists_task_state_across_scheduler_restart()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);

    let id = sched
        .submit(
            TaskId::new(1),
            NewTask {
                priority: Priority::Medium,
                payload: "hello".to_string(),
            },
            vec![],
        )
        .await?;

    drop(sched);

    let store2 =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched2: Scheduler<String, String, u64> = Scheduler::new(store2);

    let view = sched2
        .get(id)
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "expected task view"))?;
    assert!(matches!(view.state, TaskState::Ready));
    Ok(())
}

#[tokio::test]
async fn redis_store_releases_dependent_after_all_deps_complete()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-fanin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);

    let a1 = sched
        .submit(
            TaskId::new(1),
            NewTask {
                priority: Priority::Medium,
                payload: "a1".to_string(),
            },
            vec![],
        )
        .await?;
    let a2 = sched
        .submit(
            TaskId::new(2),
            NewTask {
                priority: Priority::Medium,
                payload: "a2".to_string(),
            },
            vec![],
        )
        .await?;
    let b = sched
        .submit(
            TaskId::new(3),
            NewTask {
                priority: Priority::High,
                payload: "b".to_string(),
            },
            vec![a1, a2],
        )
        .await?;

    let t1 = sched
        .next_ready("w")
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "expected ready task"))?;
    let t2 = sched
        .next_ready("w")
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "expected ready task"))?;
    assert!(sched.next_ready("w").await?.is_none());

    sched.complete(t1, Ok("ok".to_string())).await?;
    assert!(sched.next_ready("w").await?.is_none());

    sched.complete(t2, Ok("ok".to_string())).await?;
    let next = sched
        .next_ready("w")
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "expected ready task"))?;
    assert_eq!(next.id, b);
    Ok(())
}

#[tokio::test]
async fn redis_store_requeues_task_after_lease_expires() -> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-lease-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_millis(50))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::with_config(
        store,
        SchedulerConfig {
            lease_duration: Duration::from_millis(50),
            retry: RetryPolicy::None,
        },
    );

    let id = sched
        .submit(
            TaskId::new(1),
            NewTask {
                priority: Priority::Medium,
                payload: "hello".to_string(),
            },
            vec![],
        )
        .await?;

    let lease1 = sched
        .next_ready("w1")
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "expected ready task"))?;
    assert_eq!(lease1.id, id);
    assert_eq!(lease1.attempt, 1);

    tokio::time::sleep(Duration::from_millis(120)).await;
    sched.maintenance_tick().await?;

    let lease2 = sched
        .next_ready("w2")
        .await?
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "expected ready task"))?;
    assert_eq!(lease2.id, id);
    assert_eq!(lease2.attempt, 2);
    Ok(())
}

#[tokio::test]
async fn redis_store_delayed_retry_preserves_ready_sort_member()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-delayed-retry-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::with_config(
        store,
        SchedulerConfig {
            lease_duration: Duration::from_secs(30),
            retry: RetryPolicy::Fixed {
                max_attempts: 2,
                delay: Duration::from_millis(10),
            },
        },
    );

    let id = sched
        .submit(
            TaskId::new(7),
            NewTask {
                priority: Priority::Medium,
                payload: "retry".to_string(),
            },
            vec![],
        )
        .await?;

    let first = sched
        .next_ready("worker-a")
        .await?
        .ok_or_else(|| std::io::Error::other("expected first lease"))?;
    sched.complete(first, Err("transient".to_string())).await?;
    assert!(sched.next_ready("worker-b").await?.is_none());

    tokio::time::sleep(Duration::from_millis(30)).await;
    sched.maintenance_tick().await?;

    let second = sched
        .next_ready("worker-b")
        .await?
        .ok_or_else(|| std::io::Error::other("expected retry lease"))?;
    assert_eq!(second.id, id);
    assert_eq!(second.attempt, 2);
    Ok(())
}

#[tokio::test]
async fn redis_store_medium_and_low_ready_sorted_by_sort_prefix()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-sort-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);

    for id in [100, 5, 42] {
        sched
            .submit(
                TaskId::new(id),
                NewTask {
                    priority: Priority::Medium,
                    payload: id.to_string(),
                },
                vec![],
            )
            .await?;
    }

    for expected in [5, 42, 100] {
        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| std::io::Error::other("expected sorted medium task"))?;
        assert_eq!(lease.id, TaskId::new(expected));
        assert_eq!(lease.payload, expected.to_string());
    }
    assert!(sched.next_ready("w").await?.is_none());

    for id in [200, 9, 17] {
        sched
            .submit(
                TaskId::new(id),
                NewTask {
                    priority: Priority::Low,
                    payload: id.to_string(),
                },
                vec![],
            )
            .await?;
    }

    for expected in [9, 17, 200] {
        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| std::io::Error::other("expected sorted low task"))?;
        assert_eq!(lease.id, TaskId::new(expected));
        assert_eq!(lease.payload, expected.to_string());
    }
    assert!(sched.next_ready("w").await?.is_none());
    Ok(())
}

#[tokio::test]
async fn redis_store_equal_sort_prefix_keeps_fifo() -> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-sort-fifo-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store = RedisStore::<String, String, EqualSortId>::connect(
        &url,
        &namespace,
        Duration::from_secs(30),
    )
    .await?;
    let sched: Scheduler<String, String, EqualSortId> = Scheduler::new(store);

    for id in [3, 1, 2] {
        sched
            .submit(
                TaskId::new(EqualSortId(id)),
                NewTask {
                    priority: Priority::Medium,
                    payload: id.to_string(),
                },
                vec![],
            )
            .await?;
    }

    for expected in [3, 1, 2] {
        let lease = sched
            .next_ready("w")
            .await?
            .ok_or_else(|| std::io::Error::other("expected fifo medium task"))?;
        assert_eq!(lease.id, TaskId::new(EqualSortId(expected)));
        assert_eq!(lease.payload, expected.to_string());
    }
    assert!(sched.next_ready("w").await?.is_none());
    Ok(())
}

#[tokio::test]
async fn redis_store_resubmitting_cancelled_task_clears_stale_running_member()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-resubmit-running-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);
    let id = TaskId::new(7);
    sched
        .submit(
            id.clone(),
            NewTask {
                priority: Priority::Medium,
                payload: "first".to_string(),
            },
            vec![],
        )
        .await?;
    let _lease = sched
        .next_ready("w")
        .await?
        .ok_or_else(|| std::io::Error::other("expected running task"))?;
    sched.cancel(id.clone()).await?;

    let encoded = encode_task_id(&id)?;
    let task_key = format!("{namespace}:task:{encoded}");
    let running_key = format!("{namespace}:running");
    let stale_member = format!("{:020}{encoded}", 1);
    let client = redis::Client::open(url.as_str())?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    let _: () = redis::pipe()
        .cmd("HSET")
        .arg(&task_key)
        .arg("running_member")
        .arg(&stale_member)
        .ignore()
        .cmd("ZADD")
        .arg(&running_key)
        .arg(1)
        .arg(&stale_member)
        .ignore()
        .query_async(&mut conn)
        .await?;

    sched
        .submit(
            id,
            NewTask {
                priority: Priority::Medium,
                payload: "second".to_string(),
            },
            vec![],
        )
        .await?;

    let running_count: usize = conn.zcard(running_key).await?;
    assert_eq!(running_count, 0);
    Ok(())
}

#[tokio::test]
async fn redis_store_list_uses_task_index_and_removes_deleted_tasks()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-list-index-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);
    let first = TaskId::new(1);
    let second = TaskId::new(2);

    sched
        .submit(
            first.clone(),
            NewTask {
                priority: Priority::Medium,
                payload: "first".to_string(),
            },
            vec![],
        )
        .await?;
    sched
        .submit(
            second.clone(),
            NewTask {
                priority: Priority::Low,
                payload: "second".to_string(),
            },
            vec![],
        )
        .await?;

    let mut listed = sched
        .list()
        .await?
        .into_iter()
        .map(|view| view.id)
        .collect::<Vec<_>>();
    listed.sort_by_key(|id| id.0);
    assert_eq!(listed, vec![first.clone(), second.clone()]);

    sched.remove(first.clone()).await?;
    let listed_after_remove = sched
        .list()
        .await?
        .into_iter()
        .map(|view| view.id)
        .collect::<Vec<_>>();
    assert_eq!(listed_after_remove, vec![second]);

    Ok(())
}

#[tokio::test]
async fn redis_store_list_falls_back_to_task_scan_when_index_missing()
-> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-list-index-fallback-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);
    let id = TaskId::new(1);
    sched
        .submit(
            id.clone(),
            NewTask {
                priority: Priority::Medium,
                payload: "first".to_string(),
            },
            vec![],
        )
        .await?;

    let mut conn = redis::Client::open(url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    let _: () = conn.del(format!("{namespace}:tasks")).await?;

    let listed = sched
        .list()
        .await?
        .into_iter()
        .map(|view| view.id)
        .collect::<Vec<_>>();
    assert_eq!(listed, vec![id]);

    Ok(())
}

#[tokio::test]
async fn redis_store_list_backfills_partial_task_index() -> Result<(), Box<dyn std::error::Error>> {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return Ok(());
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await?;
    let port = container.get_host_port_ipv4(6379).await?;
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-list-index-partial-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await?;
    let sched: Scheduler<String, String, u64> = Scheduler::new(store);
    let missing_from_index = TaskId::new(1);
    let present_in_index = TaskId::new(2);
    sched
        .submit(
            missing_from_index.clone(),
            NewTask {
                priority: Priority::Medium,
                payload: "first".to_string(),
            },
            vec![],
        )
        .await?;
    sched
        .submit(
            present_in_index.clone(),
            NewTask {
                priority: Priority::Low,
                payload: "second".to_string(),
            },
            vec![],
        )
        .await?;

    let mut conn = redis::Client::open(url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    let missing_encoded = encode_task_id(&missing_from_index)?;
    let _: () = conn
        .srem(format!("{namespace}:tasks"), &missing_encoded)
        .await?;

    let mut listed = sched
        .list()
        .await?
        .into_iter()
        .map(|view| view.id)
        .collect::<Vec<_>>();
    listed.sort_by_key(|id| id.0);
    assert_eq!(listed, vec![missing_from_index, present_in_index]);

    let indexed: Vec<String> = conn.smembers(format!("{namespace}:tasks")).await?;
    assert!(indexed.contains(&missing_encoded));

    Ok(())
}
