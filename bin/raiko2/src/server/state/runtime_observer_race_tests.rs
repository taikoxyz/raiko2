    #[tokio::test]
    async fn stale_cancellation_cannot_cancel_replacement_incarnation() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-stale-cancellation",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
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
        let stale = runtime.get_task("root").await?.context("old root")?;
        let root = runtime.get_task("root").await?.expect("old root");
        runtime.cancel_task_if_current(&root.lifetime(), None).await?;
        runtime.remove_task_if_current(&root.lifetime()).await?;
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

        assert!(matches!(
            runtime.cancel_task_if_current(&stale.lifetime(), None).await?,
            raiko2_runtime::RuntimeMutationOutcome::Stale
        ));

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

        let error = drive_engine_success(
            &observer,
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

        let root = runtime.get_task("root").await?.expect("old root");
        runtime.cancel_task_if_current(&root.lifetime(), None).await?;
        runtime.remove_task_if_current(&root.lifetime()).await?;
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
    async fn proof_checkpoint_includes_root_joined_before_checkpoint() -> Result<()> {
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

    #[tokio::test]
    async fn proof_completion_includes_distinct_root_joined_after_checkpoint() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-post-checkpoint-shared-root",
        ))?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proof_ref = proposal_task_ref(pipeline, &request);
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
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            pipeline.route(),
        );
        let execution_permit = EngineObserver::acquire_task_execution_permit(
            &observer,
            &task_id,
            &proof_task,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let completion_permit = EngineObserver::checkpoint_completed_proof(
            &observer,
            &task_id,
            &proof_task,
            &proof_fixture(),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        register_observer_task(
            runtime.as_ref(),
            "root-b",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
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

        let root_a = runtime.get_task("root-a").await?.context("root a")?;
        let root_b = runtime.get_task("root-b").await?.context("root b")?;
        assert_eq!(root_a.runner_status, RunnerStatus::Completed);
        assert_eq!(root_b.runner_status, RunnerStatus::Completed);
        assert_eq!(root_a.proof_uri, root_b.proof_uri);
        assert!(root_a.proof_uri.is_some());
        assert!(
            runtime
                .get_proof_artifact(network_pair, pipeline, pipeline.route(), &proof_ref)
                .await?
                .is_some()
        );
        Ok(())
    }

    #[tokio::test]
    async fn proof_completion_excludes_replacement_joined_after_checkpoint() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-post-checkpoint-replacement",
        ))?);
        let network_pair = "taiko_dev/ethereum";
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
        for root in ["root-a", "root-b"] {
            register_observer_task(
                runtime.as_ref(),
                root,
                network_pair,
                pipeline,
                &request,
                RunnerStatus::Running,
            )
            .await?;
        }
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            pipeline.route(),
        );
        let execution_permit = EngineObserver::acquire_task_execution_permit(
            &observer,
            &task_id,
            &proof_task,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let completion_permit = EngineObserver::checkpoint_completed_proof(
            &observer,
            &task_id,
            &proof_task,
            &proof_fixture(),
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        let old_root = runtime.get_task("root-a").await?.context("old root a")?;
        runtime
            .cancel_task_if_current(&old_root.lifetime(), None)
            .await?;
        runtime
            .remove_task_if_current(&old_root.lifetime())
            .await?;
        register_observer_task(
            runtime.as_ref(),
            "root-a",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let replacement = runtime
            .get_task("root-a")
            .await?
            .context("replacement root a")?;
        assert_ne!(replacement.incarnation_id, old_root.incarnation_id);

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

        let replacement = runtime
            .get_task("root-a")
            .await?
            .context("replacement root a")?;
        assert_eq!(replacement.runner_status, RunnerStatus::Running);
        assert!(replacement.proof_uri.is_none());
        let surviving_owner = runtime.get_task("root-b").await?.context("root b")?;
        assert_eq!(surviving_owner.runner_status, RunnerStatus::Completed);
        assert!(surviving_owner.proof_uri.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn proposal_completion_does_not_claim_an_external_aggregate_consumer() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-external-consumer",
        ))?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proof_ref = proposal_task_ref(pipeline, &request);
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
            "proposal-root",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let aggregate_request = AggregationTaskRequest {
            request_id: "external-consumer".into(),
            proposal_ids: vec![request.proposal_id],
            prover_config: ProverTaskConfig::default(),
        };
        let consumer_metadata = TaskMetadata {
            network_pair: network_pair.into(),
            network: "taiko_dev".into(),
            l1_network: "ethereum".into(),
            proof_type: ProofType::Native,
            requested_proof_type: None,
            prover_type: None,
            execution_mode: Some(ExecutionMode::Prove),
            aggregate_requested: true,
            proposals: Vec::new(),
            aggregate_task_id: Some(aggregate_task_ref(pipeline, &aggregate_request)),
            aggregate_request: Some(aggregate_request),
            aggregate_input_artifacts: vec![AggregateInputProofArtifact {
                proof_ref: proof_ref.clone(),
            }],
            runtime: RuntimeMetadata::default(),
        };
        let artifact_refs = publication_proof_artifact_refs(&consumer_metadata, pipeline);
        runtime
            .register_task(TaskRegistration {
                task_id: "aggregate-consumer".into(),
                pipeline_key: pipeline,
                route: pipeline.route(),
                task_kind: "hoodi_aggregate".into(),
                network_pair: network_pair.into(),
                artifact_refs,
                metadata: serde_json::to_value(consumer_metadata)?,
                request_fingerprint: "external-consumer".into(),
            })
            .await?;
        let mut consumer = runtime
            .get_task("aggregate-consumer")
            .await?
            .context("aggregate consumer")?;
        consumer.runner_status = RunnerStatus::Running;
        runtime.upsert_task(&consumer).await?;
        let before = runtime
            .get_task("aggregate-consumer")
            .await?
            .context("aggregate consumer")?;

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            pipeline.route(),
        );
        let execution_permit = EngineObserver::acquire_task_execution_permit(
            &observer,
            &task_id,
            &proof_task,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
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

        let after = runtime
            .get_task("aggregate-consumer")
            .await?
            .context("aggregate consumer")?;
        assert_eq!(after.runner_status, before.runner_status);
        assert_eq!(after.metadata, before.metadata);
        assert_eq!(after.proof_uri, before.proof_uri);
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(
            runtime
                .get_task("proposal-root")
                .await?
                .context("proposal root")?
                .runner_status,
            RunnerStatus::Completed
        );
        Ok(())
    }

    #[tokio::test]
    async fn terminal_failure_includes_late_joining_shared_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-late-terminal-root",
        ))?);
        let network_pair = "taiko_dev/ethereum";
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let task = EngineTask::Preflight {
            request: request.clone(),
        };
        register_observer_task(
            runtime.as_ref(),
            "root-a",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Running,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            network_pair.to_string(),
            pipeline.route(),
        );
        let execution_permit = EngineObserver::acquire_task_execution_permit(
            &observer,
            &task_id,
            &task,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        register_observer_task(
            runtime.as_ref(),
            "root-b",
            network_pair,
            pipeline,
            &request,
            RunnerStatus::Allocated,
        )
        .await?;
        let terminal_permit = EngineObserver::acquire_terminal_failure_permit(
            &observer,
            &task_id,
            &task,
            &execution_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let projection = EngineObserver::on_task_failed(
            &observer,
            &task_id,
            &task,
            "terminal failure",
            terminal_permit,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        assert_eq!(projection.root_owners().len(), 2);
        for root in ["root-a", "root-b"] {
            let record = runtime.get_task(root).await?.context("shared root")?;
            assert_eq!(record.runner_status, RunnerStatus::Failed, "{root}");
            assert_eq!(record.error.as_deref(), Some("terminal failure"), "{root}");
            assert!(
                projection.root_owners().contains(&RootOwner::new(
                    record.task_id.clone(),
                    record.incarnation_id,
                )),
                "{root}"
            );
        }
        Ok(())
    }

    async fn publication_owner_incarnations(
        runtime: &RuntimeManager,
        proof_ref: &str,
    ) -> Vec<uuid::Uuid> {
        runtime
            .tasks_referencing(proof_ref)
            .await
            .into_iter()
            .filter(|record| {
                !matches!(record.runner_status, RunnerStatus::Failed | RunnerStatus::Cancelled)
            })
            .map(|record| record.incarnation_id)
            .collect()
    }
