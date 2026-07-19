use super::*;
use crate::{MemoryStore, Priority, StoreResult, TaskState};
use std::time::Duration;

type TestScheduler = Scheduler<&'static str, &'static str, u64>;

fn scheduler() -> TestScheduler {
    Scheduler::new(MemoryStore::new())
}

fn owner(task_id: &str) -> RootOwner {
    RootOwner::new(task_id, uuid::Uuid::new_v4())
}

fn node(
    id: u64,
    payload: &'static str,
    dependencies: impl IntoIterator<Item = u64>,
) -> ExecutionNode<&'static str, u64> {
    ExecutionNode {
        id: TaskId::new(id),
        task: NewTask {
            priority: Priority::Medium,
            payload,
        },
        dependencies: dependencies.into_iter().map(TaskId::new).collect(),
        execution_policy: TaskExecutionPolicy {
            lease_duration: Duration::from_secs(60),
            retry: RetryPolicy::None,
        },
    }
}

#[tokio::test]
async fn shared_node_is_retained_until_the_last_owner_cancels() -> StoreResult<()> {
    let scheduler = scheduler();
    let first_owner = owner("root-1");
    let second_owner = owner("root-2");
    let proposal = TaskId::new(1);
    let first_aggregate = TaskId::new(2);
    let second_aggregate = TaskId::new(3);

    assert_eq!(
        scheduler
            .attach(
                first_owner.clone(),
                ExecutionGraph::new(vec![node(1, "proposal", []), node(2, "agg-1", [1])]),
            )
            .await?,
        AttachOutcome::Attached
    );
    scheduler
        .attach(
            second_owner.clone(),
            ExecutionGraph::new(vec![node(1, "proposal", []), node(3, "agg-2", [1])]),
        )
        .await?;

    let first_detach = scheduler.detach(&first_owner, DetachMode::Cancel).await?;
    assert_eq!(first_detach.retained, vec![proposal.clone()]);
    assert_eq!(first_detach.retired, vec![first_aggregate.clone()]);
    assert!(matches!(
        scheduler.get(first_aggregate).await?.map(|view| view.state),
        Some(TaskState::Cancelled)
    ));
    let second_view = scheduler
        .inspect(&second_owner)
        .await?
        .ok_or_else(|| TaskStoreError::corrupt_msg("expected second owner projection"))?;
    assert_eq!(
        second_view
            .tasks
            .iter()
            .find(|task| task.id == proposal)
            .map(|task| task.owner_count),
        Some(1)
    );

    let second_detach = scheduler.detach(&second_owner, DetachMode::Cancel).await?;
    assert_eq!(
        second_detach.retired,
        vec![proposal.clone(), second_aggregate.clone()]
    );
    assert!(matches!(
        scheduler
            .get(proposal.clone())
            .await?
            .map(|view| view.state),
        Some(TaskState::Cancelled)
    ));
    assert!(matches!(
        scheduler
            .get(second_aggregate.clone())
            .await?
            .map(|view| view.state),
        Some(TaskState::Failed {
            caused_by_dep: Some(cause),
            ..
        }) if cause == proposal
    ));

    let replacement_owner = owner("root-2");
    scheduler
        .attach(
            replacement_owner.clone(),
            ExecutionGraph::new(vec![node(1, "proposal", []), node(3, "agg-2", [1])]),
        )
        .await?;
    let stale_cleanup = scheduler.detach(&second_owner, DetachMode::Remove).await?;
    assert_eq!(stale_cleanup.retained, vec![proposal, second_aggregate]);
    assert!(scheduler.inspect(&replacement_owner).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn remove_detach_deletes_only_nodes_without_an_owner() -> StoreResult<()> {
    let scheduler = scheduler();
    let first_owner = owner("root-1");
    let second_owner = owner("root-2");

    scheduler
        .attach(
            first_owner.clone(),
            ExecutionGraph::new(vec![node(1, "proposal", []), node(2, "agg-1", [1])]),
        )
        .await?;
    scheduler
        .attach(
            second_owner.clone(),
            ExecutionGraph::new(vec![node(1, "proposal", []), node(3, "agg-2", [1])]),
        )
        .await?;

    let first_detach = scheduler.detach(&first_owner, DetachMode::Remove).await?;
    assert_eq!(first_detach.retained, vec![TaskId::new(1)]);
    assert_eq!(first_detach.retired, vec![TaskId::new(2)]);
    assert!(scheduler.get(TaskId::new(1)).await?.is_some());
    assert!(scheduler.get(TaskId::new(2)).await?.is_none());

    let second_detach = scheduler.detach(&second_owner, DetachMode::Remove).await?;
    assert_eq!(second_detach.retired, vec![TaskId::new(1), TaskId::new(3)]);
    assert!(scheduler.get(TaskId::new(1)).await?.is_none());
    assert!(scheduler.get(TaskId::new(3)).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn conflicting_graph_attach_is_atomic() -> StoreResult<()> {
    let scheduler = scheduler();
    scheduler
        .attach(
            owner("base"),
            ExecutionGraph::new(vec![node(1, "canonical", [])]),
        )
        .await?;
    let conflicting_owner = owner("conflict");

    let error = scheduler
        .attach(
            conflicting_owner.clone(),
            ExecutionGraph::new(vec![
                node(2, "must-not-be-inserted", []),
                node(1, "different", []),
            ]),
        )
        .await
        .expect_err("conflicting shared task definition must fail");
    assert!(matches!(error, TaskStoreError::Conflict(_)));
    assert!(scheduler.get(TaskId::new(2)).await?.is_none());
    assert!(scheduler.inspect(&conflicting_owner).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn checkpoint_policy_does_not_change_the_shared_task_definition() -> StoreResult<()> {
    let scheduler = scheduler();
    let first_owner = owner("first");
    let graph = ExecutionGraph::new(vec![node(1, "task", [])]);
    scheduler.attach(first_owner, graph.clone()).await?;
    let lease = scheduler
        .next_ready("worker")
        .await?
        .ok_or_else(|| TaskStoreError::corrupt_msg("expected running lease"))?;
    assert!(
        scheduler
            .checkpoint_payload(
                &lease,
                "checkpointed",
                TaskExecutionPolicy {
                    lease_duration: Duration::from_secs(60),
                    retry: RetryPolicy::Fixed {
                        max_attempts: 3,
                        delay: Duration::from_secs(1),
                    },
                },
            )
            .await?
    );

    assert_eq!(
        scheduler.attach(owner("second"), graph).await?,
        AttachOutcome::Attached
    );
    assert_eq!(
        scheduler
            .complete_with_disposition(lease, Ok("completed"))
            .await?,
        TaskCompletionDisposition::Succeeded
    );
    Ok(())
}

#[tokio::test]
async fn cyclic_graph_attach_is_atomic() -> StoreResult<()> {
    let scheduler = scheduler();
    let cyclic_owner = owner("cycle");
    let error = scheduler
        .attach(
            cyclic_owner.clone(),
            ExecutionGraph::new(vec![node(1, "a", [2]), node(2, "b", [1])]),
        )
        .await
        .expect_err("cyclic graph must fail");
    assert!(matches!(error, TaskStoreError::InvalidGraph(_)));
    assert!(scheduler.get(TaskId::new(1)).await?.is_none());
    assert!(scheduler.get(TaskId::new(2)).await?.is_none());
    assert!(scheduler.inspect(&cyclic_owner).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn exact_owner_and_lease_token_fence_remove_recreate_aba() -> StoreResult<()> {
    let scheduler = scheduler();
    let stale_owner = owner("same-root");
    scheduler
        .attach(
            stale_owner.clone(),
            ExecutionGraph::new(vec![node(1, "task", [])]),
        )
        .await?;
    let stale_lease = scheduler
        .next_ready("worker")
        .await?
        .ok_or_else(|| TaskStoreError::corrupt_msg("expected stale lease"))?;
    scheduler.detach(&stale_owner, DetachMode::Remove).await?;

    let replacement_owner = owner("same-root");
    scheduler
        .attach(
            replacement_owner.clone(),
            ExecutionGraph::new(vec![node(1, "task", [])]),
        )
        .await?;
    let replacement_lease = scheduler
        .next_ready("worker")
        .await?
        .ok_or_else(|| TaskStoreError::corrupt_msg("expected replacement lease"))?;
    assert_ne!(stale_lease.lease_token, replacement_lease.lease_token);

    let stale_detach = scheduler.detach(&stale_owner, DetachMode::Remove).await?;
    assert!(!stale_detach.detached);
    assert!(scheduler.inspect(&replacement_owner).await?.is_some());
    assert_eq!(
        scheduler
            .complete_with_disposition(stale_lease, Ok("stale"))
            .await?,
        TaskCompletionDisposition::Stale
    );
    assert_eq!(
        scheduler
            .complete_with_disposition(replacement_lease, Ok("current"))
            .await?,
        TaskCompletionDisposition::Succeeded
    );
    Ok(())
}

#[tokio::test]
async fn reattaching_a_failed_owner_graph_makes_its_terminal_tasks_runnable() -> StoreResult<()> {
    let scheduler = scheduler();
    let root = owner("recoverable-root");
    let graph = ExecutionGraph::new(vec![node(1, "proposal", [])]);

    scheduler.attach(root.clone(), graph.clone()).await?;
    let failed_lease = scheduler
        .next_ready("worker")
        .await?
        .ok_or_else(|| TaskStoreError::corrupt_msg("expected first lease"))?;
    assert!(
        scheduler
            .checkpoint_payload(
                &failed_lease,
                "publication-checkpoint",
                TaskExecutionPolicy {
                    lease_duration: Duration::from_secs(120),
                    retry: RetryPolicy::Fixed {
                        max_attempts: 1,
                        delay: Duration::from_secs(1),
                    },
                },
            )
            .await?
    );
    assert!(
        scheduler
            .complete(failed_lease, Err("transient store failure".to_string()))
            .await?
    );

    assert_eq!(
        scheduler.attach(root, graph).await?,
        AttachOutcome::Attached
    );
    let retry_lease = scheduler
        .next_ready("worker")
        .await?
        .ok_or_else(|| TaskStoreError::corrupt_msg("expected recovered lease"))?;
    assert_eq!(retry_lease.attempt, 1);
    assert_eq!(retry_lease.payload, "proposal");
    assert_eq!(retry_lease.execution_policy.retry, RetryPolicy::None);
    assert!(
        scheduler
            .complete_with_disposition(retry_lease, Ok("recovered"))
            .await?
            .applied()
    );
    Ok(())
}
