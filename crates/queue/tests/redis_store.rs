#![cfg(feature = "redis")]
#![allow(missing_docs)]

use std::process::Command;
use std::time::Duration;

use raiko2_queue::{NewTask, Priority, RedisStore, Scheduler, TaskId, TaskState};
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

#[tokio::test]
async fn redis_store_persists_task_state_across_scheduler_restart() {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return;
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await
            .unwrap();
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
        .await
        .unwrap();

    drop(sched);

    let store2 =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await
            .unwrap();
    let sched2: Scheduler<String, String, u64> = Scheduler::new(store2);

    let view = sched2.get(id).await.unwrap().unwrap();
    assert!(matches!(view.state, TaskState::Ready));
}

#[tokio::test]
async fn redis_store_releases_dependent_after_all_deps_complete() {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return;
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-fanin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_secs(30))
            .await
            .unwrap();
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
        .await
        .unwrap();
    let a2 = sched
        .submit(
            TaskId::new(2),
            NewTask {
                priority: Priority::Medium,
                payload: "a2".to_string(),
            },
            vec![],
        )
        .await
        .unwrap();
    let b = sched
        .submit(
            TaskId::new(3),
            NewTask {
                priority: Priority::High,
                payload: "b".to_string(),
            },
            vec![a1, a2],
        )
        .await
        .unwrap();

    let t1 = sched.next_ready("w").await.unwrap().unwrap();
    let t2 = sched.next_ready("w").await.unwrap().unwrap();
    assert!(sched.next_ready("w").await.unwrap().is_none());

    sched.complete(t1, Ok("ok".to_string())).await.unwrap();
    assert!(sched.next_ready("w").await.unwrap().is_none());

    sched.complete(t2, Ok("ok".to_string())).await.unwrap();
    assert_eq!(sched.next_ready("w").await.unwrap().unwrap().id, b);
}

#[tokio::test]
async fn redis_store_requeues_task_after_lease_expires() {
    if !docker_available() {
        eprintln!("skipping redis store test: docker unavailable");
        return;
    }

    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}/");
    let namespace = format!(
        "test-lease-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    let store =
        RedisStore::<String, String, u64>::connect(&url, &namespace, Duration::from_millis(50))
            .await
            .unwrap();
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
        .await
        .unwrap();

    let lease1 = sched.next_ready("w1").await.unwrap().unwrap();
    assert_eq!(lease1.id, id);
    assert_eq!(lease1.attempt, 1);

    tokio::time::sleep(Duration::from_millis(120)).await;
    sched.maintenance_tick().await.unwrap();

    let lease2 = sched.next_ready("w2").await.unwrap().unwrap();
    assert_eq!(lease2.id, id);
    assert_eq!(lease2.attempt, 2);
}
