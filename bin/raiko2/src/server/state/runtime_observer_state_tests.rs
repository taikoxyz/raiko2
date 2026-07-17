    #[tokio::test]
    async fn invalidated_publication_cancels_active_and_completed_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-invalidated-completion-rollback",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let roots = [
            ("task_allocated", RunnerStatus::Allocated),
            ("task_running", RunnerStatus::Running),
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
            ("task_completed", RunnerStatus::Completed),
        ];
        for (task_id, status) in roots {
            register_observer_task(
                runtime.as_ref(),
                task_id,
                "taiko_dev/ethereum",
                pipeline,
                &request,
                status,
            )
            .await?;
        }
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );

        observer
            .mark_proof_publication_failed(
                &task_id,
                "prove",
                "proof invalidated during completion",
                PublicationFailureDisposition::Invalidated,
            )
            .await;

        for task_id in ["task_allocated", "task_running", "task_completed"] {
            let invalidated = runtime.get_task(task_id).await?.expect("invalidated task");
            assert_eq!(
                invalidated.runner_status,
                RunnerStatus::Cancelled,
                "{task_id}"
            );
            assert_eq!(invalidated.proof_uri, None, "{task_id}");
            assert_eq!(
                invalidated.error.as_deref(),
                Some("proof invalidated during completion"),
                "{task_id}"
            );
        }
        for (task_id, status) in [
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
        ] {
            let terminal = runtime.get_task(task_id).await?.expect("terminal task");
            assert_eq!(terminal.runner_status, status, "{task_id}");
            assert_eq!(terminal.proof_uri, None, "{task_id}");
            assert_eq!(terminal.error, None, "{task_id}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn publication_sync_accepts_concurrently_reconciled_completed_root() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-publication-reconciled-root",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        register_observer_task(
            runtime.as_ref(),
            "task_reconciled_root",
            "taiko_dev/ethereum",
            pipeline,
            &request,
            RunnerStatus::Completed,
        )
        .await?;
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            route,
        );

        observer
            .on_task_succeeded(
                &task_id,
                &EngineTask::ProveProposal {
                    request: request.clone(),
                    input_task: task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await
            .map_err(anyhow::Error::msg)?;

        assert_eq!(
            runtime
                .get_task("task_reconciled_root")
                .await?
                .expect("completed task")
                .runner_status,
            RunnerStatus::Completed
        );
        let proof_ref = proposal_task_ref(pipeline, &request);
        assert!(
            runtime
                .read_proof_artifact_bytes(
                    "taiko_dev/ethereum",
                    pipeline,
                    route,
                    &proof_ref,
                )
                .await?
                .is_some(),
            "publication retry invalidated the reconciled root artifact"
        );
        assert!(
            runtime
                .get_pending_proof_publication(
                    "taiko_dev/ethereum",
                    pipeline,
                    route,
                    &proof_ref,
                )
                .await?
                .is_none(),
            "successful retry left a pending publication checkpoint"
        );
        Ok(())
    }

    #[tokio::test]
    async fn retryable_publication_failure_resets_only_active_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-retryable-publication-matrix",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });
        let roots = [
            ("task_allocated", RunnerStatus::Allocated),
            ("task_running", RunnerStatus::Running),
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
            ("task_completed", RunnerStatus::Completed),
        ];
        for (root_id, status) in roots {
            register_observer_task(
                runtime.as_ref(),
                root_id,
                "taiko_dev/ethereum",
                pipeline,
                &request,
                status,
            )
            .await?;
        }
        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            pipeline.route(),
        );

        observer
            .mark_proof_publication_failed(
                &task_id,
                "prove",
                "transient artifact store failure",
                PublicationFailureDisposition::Retryable,
            )
            .await;

        for task_id in ["task_allocated", "task_running"] {
            let retryable = runtime.get_task(task_id).await?.expect("retryable task");
            assert_eq!(
                retryable.runner_status,
                RunnerStatus::Allocated,
                "{task_id}"
            );
            assert_eq!(retryable.proof_uri, None, "{task_id}");
            assert_eq!(
                retryable.error.as_deref(),
                Some("transient artifact store failure"),
                "{task_id}"
            );
        }
        for (task_id, status) in [
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
            ("task_completed", RunnerStatus::Completed),
        ] {
            let terminal = runtime.get_task(task_id).await?.expect("terminal task");
            assert_eq!(terminal.runner_status, status, "{task_id}");
            assert_eq!(terminal.proof_uri, None, "{task_id}");
            assert_eq!(terminal.error, None, "{task_id}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn proof_publication_success_updates_active_and_completed_shared_roots() -> Result<()> {
        let runtime = Arc::new(RuntimeManager::new(unique_runtime_root(
            "runtime-observer-terminal-root",
        ))?);
        let pipeline = PipelineKey::ShastaNative;
        let request = proposal_request();
        let proposal_task_id = EngineTaskId::new(EngineTaskKey::Proposal {
            pipeline,
            request: request.clone(),
        });

        let roots = [
            ("task_allocated", RunnerStatus::Allocated),
            ("task_running", RunnerStatus::Running),
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
            ("task_completed", RunnerStatus::Completed),
        ];
        for (task_id, status) in roots {
            register_observer_task(
                runtime.as_ref(),
                task_id,
                "taiko_dev/ethereum",
                pipeline,
                &request,
                status,
            )
            .await?;
        }

        let observer = RuntimeObserver::new(
            Arc::clone(&runtime),
            "taiko_dev/ethereum".to_string(),
            PipelineKey::ShastaNative.route(),
        );
        observer
            .on_task_succeeded(
                &proposal_task_id,
                &EngineTask::ProveProposal {
                    request,
                    input_task: proposal_task_id.clone(),
                },
                &EngineTaskSuccess::Proof {
                    stage: raiko2_pipeline::PipelineStage::Prove,
                    proof: proof_fixture(),
                },
            )
            .await
            .map_err(anyhow::Error::msg)?;

        for task_id in ["task_allocated", "task_running"] {
            let active = runtime.get_task(task_id).await?.expect("active task");
            assert_eq!(active.runner_status, RunnerStatus::Completed, "{task_id}");
            assert!(active.proof_uri.is_some(), "{task_id}");
        }
        for (task_id, status) in [
            ("task_cancelled", RunnerStatus::Cancelled),
            ("task_failed", RunnerStatus::Failed),
        ] {
            let terminal = runtime.get_task(task_id).await?.expect("terminal task");
            assert_eq!(terminal.runner_status, status, "{task_id}");
            assert_eq!(terminal.proof_uri, None, "{task_id}");
        }
        let completed = runtime
            .get_task("task_completed")
            .await?
            .expect("completed task");
        assert_eq!(completed.runner_status, RunnerStatus::Completed);
        assert!(completed.proof_uri.is_some());
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
