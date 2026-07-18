    #[tokio::test]
    async fn stale_cancellation_cannot_cancel_replacement_incarnation() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-stale-cancellation",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let stale_incarnation = runtime
            .get_task("root")
            .await?
            .context("old root")?
            .incarnation_id;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            pipeline.route(),
        );
        let stale_permit = observer
            .acquire_task_cancellation_permit(&task_id)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        runtime
            .sync_status("root", RunnerStatus::Cancelled, None, None)
            .await?;
        runtime.remove_task("root").await?;
        register_observer_task(
            runtime.as_ref(),
            "root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let replacement = runtime.get_task("root").await?.context("replacement root")?;
        assert_ne!(stale_incarnation, replacement.incarnation_id);

        observer.on_task_cancelled(&task_id, &stale_permit).await;

        let replacement = runtime.get_task("root").await?.context("replacement root")?;
        assert_eq!(replacement.runner_status, RunnerStatus::Running);
        Ok(())
    }
    #[tokio::test]
    async fn proof_publication_without_active_roots_invalidates_artifact_and_outbox() -> Result<()>
    {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-zero-active-publication",
        ))?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let proof_ref = proposal_task_ref(pipeline, &request);
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        for (root_id, status) in [
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
        ] {
            register_observer_task(
                runtime.as_ref(),
                root_id,
                network_pair,
                pipeline,
                &request,
                status,
            )
            .await?;
        }
        let proof = proof_fixture();
        let proof_bytes = serde_json::to_vec(&proof)?;
        runtime
            .upsert_pending_proof_publication(
                network_pair,
                pipeline,
                route,
                &proof_ref,
                &proof_bytes,
            )
            .await?;
        runtime
            .publish_proof_artifact_bytes(network_pair, pipeline, route, &proof_ref, &proof_bytes)
            .await?;
        let observer = RuntimeObserver::new(Arc::clone(&runtime), network_pair.to_string(), route);

        let error = observer
            .on_task_succeeded(
                &task_id,
                &EngineTask::ProveProposal {
                    request: request.clone(),
                    input_task: task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof,
                },
            )
            .await
            .expect_err("publication without an active root must be invalidated");

        assert!(matches!(error, EngineObserverError::ProofInvalidated(_)));
        for (root_id, status) in [
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
        ] {
            let terminal = runtime.get_task(root_id).await?.expect("terminal task");
            assert_eq!(terminal.runner_status, status, "{root_id}");
            assert_eq!(terminal.proof_uri, None, "{root_id}");
        }
        assert!(
            runtime
                .read_proof_artifact_bytes(network_pair, pipeline, route, &proof_ref)
                .await?
                .is_none()
        );
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline, route, &proof_ref)
                .await?
                .is_none()
        );
        assert!(
            runtime
                .get_pending_proof_publication(network_pair, pipeline, route, &proof_ref)
                .await?
                .is_none()
        );
        assert!(
            observer
                .load_completed_proof(&task_id, &EngineTask::Proposal { request })
                .await
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        Ok(())
    }
    #[tokio::test]
    async fn stale_execution_permit_cannot_mutate_replacement_incarnation() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-stale-execution-permit",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let stage_task = EngineTask::Preflight {
            request: request.clone(),
        };
        register_observer_task(
            runtime.as_ref(),
            "root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            pipeline.route(),
        );
        let stale_permit = EngineObserver::acquire_task_execution_permit(
            &observer,
            &task_id,
            &stage_task,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        runtime
            .sync_status("root", RunnerStatus::Cancelled, None, None)
            .await?;
        runtime.remove_task("root").await?;
        register_observer_task(
            runtime.as_ref(),
            "root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let before = runtime.get_task("root").await?.context("replacement root")?;

        let Err(checkpoint_error) = EngineObserver::checkpoint_completed_proof(
            &observer,
            &task_id,
            &EngineTask::ProveProposal {
                request: request.clone(),
                input_task: task_id.clone(),
            },
            &proof_fixture(),
            &stale_permit,
        )
        .await
        else {
            anyhow::bail!("stale proof checkpoint claimed the replacement");
        };
        assert!(matches!(checkpoint_error, EngineObserverError::RuntimeSync(_)));

        EngineObserver::on_task_started(
            &observer,
            &task_id,
            &stage_task,
            "stale-worker",
            &stale_permit,
        )
        .await;
        let error = EngineObserver::on_task_succeeded(
            &observer,
            &task_id,
            &stage_task,
            &EngineTaskSuccess::GuestInput {
                stage: raiko2_pipeline::PipelineStage::Preflight,
            },
            None,
            &stale_permit,
        )
        .await
        .expect_err("stale completion must be rejected");
        assert!(matches!(error, EngineObserverError::RuntimeSync(_)));

        let after = runtime.get_task("root").await?.context("replacement root")?;
        assert_eq!(after.incarnation_id, before.incarnation_id);
        assert_eq!(after.runner_status, before.runner_status);
        assert_eq!(after.error, before.error);
        assert_eq!(after.metadata, before.metadata);
        Ok(())
    }

    #[tokio::test]
    async fn proof_checkpoint_includes_late_joining_shared_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-late-shared-root",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let proof_task = EngineTask::ProveProposal {
            request: request.clone(),
            input_task: task_id.clone(),
        };
        register_observer_task(
            runtime.as_ref(),
            "root-a",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            pipeline.route(),
        );
        let execution_permit = EngineObserver::acquire_task_execution_permit(
            &observer,
            &task_id,
            &proof_task,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        register_observer_task(
            runtime.as_ref(),
            "root-b",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let completion_permit = EngineObserver::checkpoint_completed_proof(
            &observer,
            &task_id,
            &proof_task,
            &proof_fixture(),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        EngineObserver::on_task_succeeded(
            &observer,
            &task_id,
            &proof_task,
            &EngineTaskSuccess::Proof {
                stage: raiko2_pipeline::PipelineStage::Prove,
                proof: proof_fixture(),
            },
            Some(&completion_permit),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        for root in ["root-a", "root-b"] {
            let record = runtime.get_task(root).await?.context("shared root")?;
            assert_eq!(record.runner_status, RunnerStatus::Completed, "{root}");
            assert!(record.proof_uri.is_some(), "{root}");
        }
        Ok(())
    }

    async fn publication_owner_incarnations(
        runtime: &RuntimeManager,
        proof_ref: &str,
    ) -> Vec<uuid::Uuid> {
        runtime
            .get_tasks_by_ref(proof_ref)
            .await
            .into_iter()
            .filter(|record| {
                !matches!(record.runner_status, RunnerStatus::Failed | RunnerStatus::Cancelled)
            })
            .map(|record| record.incarnation_id)
            .collect()
    }
