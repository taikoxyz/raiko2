#[derive(Debug)]
enum OwnerFailureMode {
    Renew,
    Verify,
}

#[derive(Debug)]
struct OwnerFailureStore {
    mode: OwnerFailureMode,
}

#[async_trait::async_trait]
impl ProofArtifactStore for OwnerFailureStore {
    fn environment(&self) -> &str {
        "test"
    }

    fn namespace(&self) -> &str {
        "owner-failure"
    }

    fn backend_name(&self) -> &'static str {
        "owner-failure"
    }

    async fn put_if_absent(
        &self,
        _key: &ProofArtifactKey,
        _bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        unreachable!("artifact operations are not used by ownership tests")
    }

    async fn get(&self, _key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        unreachable!("artifact operations are not used by ownership tests")
    }

    async fn get_prefix(
        &self,
        _key: &ProofArtifactKey,
        _max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>> {
        unreachable!("artifact operations are not used by ownership tests")
    }

    async fn mark_invalidated(
        &self,
        _key: &ProofArtifactKey,
        _generation: Option<i64>,
        _content_hash: &str,
    ) -> Result<()> {
        unreachable!("artifact operations are not used by ownership tests")
    }

    async fn is_invalidated(
        &self,
        _key: &ProofArtifactKey,
        _generation: Option<i64>,
        _content_hash: &str,
    ) -> Result<bool> {
        unreachable!("artifact operations are not used by ownership tests")
    }

    async fn delete(
        &self,
        _key: &ProofArtifactKey,
        _generation: Option<i64>,
        _expected_content_hash: &str,
    ) -> Result<()> {
        unreachable!("artifact operations are not used by ownership tests")
    }

    async fn claim_namespace_owner(
        &self,
        owner_id: &str,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<artifact_store::NamespaceOwnerLease>> {
        Ok(Some(artifact_store::NamespaceOwnerLease {
            owner_id: owner_id.to_string(),
            epoch: 1,
            expires_at_secs: now_secs.saturating_add(lease_secs),
            generation: Some(1),
        }))
    }

    async fn renew_namespace_owner(
        &self,
        _lease: &artifact_store::NamespaceOwnerLease,
        _now_secs: u64,
        _lease_secs: u64,
    ) -> Result<Option<artifact_store::NamespaceOwnerLease>> {
        match self.mode {
            OwnerFailureMode::Renew => anyhow::bail!("authoritative owner renewal unavailable"),
            OwnerFailureMode::Verify => unreachable!("renew is not used by readiness test"),
        }
    }

    async fn verify_namespace_owner(
        &self,
        _lease: &artifact_store::NamespaceOwnerLease,
        _now_secs: u64,
    ) -> Result<bool> {
        match self.mode {
            OwnerFailureMode::Verify => anyhow::bail!("authoritative owner read unavailable"),
            OwnerFailureMode::Renew => Ok(true),
        }
    }
}

#[tokio::test]
async fn owner_store_read_failure_makes_runtime_unready() -> Result<()> {
    let runtime = RuntimeManager::with_store(Arc::new(OwnerFailureStore {
        mode: OwnerFailureMode::Verify,
    }))?;
    runtime.acquire_namespace_owner(60).await?;

    let error = runtime
        .check_readiness()
        .await
        .expect_err("authoritative owner read failure must reject readiness");
    assert!(error.to_string().contains("authoritative owner read unavailable"));
    assert!(
        runtime
            .register_task(proposal_registration(
                "frozen-after-owner-read-error",
                1,
                PipelineKey::ShastaNative,
            ))
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn owner_renewal_failure_freezes_runtime_admissions() -> Result<()> {
    let runtime = RuntimeManager::with_store(Arc::new(OwnerFailureStore {
        mode: OwnerFailureMode::Renew,
    }))?;
    runtime.acquire_namespace_owner(60).await?;

    let error = runtime
        .renew_namespace_owner(60)
        .await
        .expect_err("authoritative renewal failure must propagate");
    assert!(error.to_string().contains("authoritative owner renewal unavailable"));
    assert!(runtime.check_readiness().await.is_err());
    assert!(
        runtime
            .register_task(proposal_registration(
                "frozen-after-renewal-error",
                1,
                PipelineKey::ShastaNative,
            ))
            .await
            .is_err()
    );
    Ok(())
}

    #[tokio::test]
    async fn current_namespace_owner_is_ready_and_can_mutate_runtime() -> Result<()> {
        let runtime = RuntimeManager::new_memory("test".into(), "current-owner".into())?;
        runtime.acquire_namespace_owner(60).await?;
        runtime.initialize().await?;
        runtime.fence_namespace_owner().await?;

        assert!(runtime.renew_namespace_owner(60).await?);
        runtime.check_readiness().await?;
        runtime
            .register_task(proposal_registration(
                "current-owner-task",
                1,
                PipelineKey::ShastaNative,
            ))
            .await?;

        assert!(runtime.get_task("current-owner-task").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn takeover_deactivates_stale_owner_and_blocks_runtime_and_tombstone_writes() -> Result<()>
    {
        let store = Arc::new(MemoryProofArtifactStore::new(
            "test".into(),
            "owner-takeover".into(),
        )?);
        let first = RuntimeManager::with_store(store.clone())?;
        first.acquire_namespace_owner(1).await?;
        first.initialize().await?;
        first.fence_namespace_owner().await?;
        first.check_readiness().await?;

        let pipeline = PipelineKey::ShastaNative;
        let route = pipeline.route();
        let proof_ref = "proposal-before-takeover";
        let publication = first
            .publish_proof_artifact_bytes(
                "l1-l2",
                pipeline,
                route,
                proof_ref,
                br#"{"proof":"0x01"}"#,
            )
            .await?;
        let artifact = publication.object().clone();
        first
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "l1-l2".into(),
                proof_ref: proof_ref.into(),
                pipeline_key: pipeline,
                route,
                proof_uri: artifact.proof_uri.clone(),
                content_hash: artifact.content_hash.clone(),
                generation: artifact.generation,
            })
            .await?;

        let first_lease = first
            .owner
            .lock()
            .await
            .as_ref()
            .context("first owner must hold a lease")?
            .clone();
        let second_lease = store
            .claim_namespace_owner("new-owner", first_lease.expires_at_secs, 60)
            .await?
            .context("expired owner must be replaceable")?;

        let second = RuntimeManager::with_store(store.clone())?;
        *second.owner.lock().await = Some(second_lease);
        second.initialize().await?;
        second.fence_namespace_owner().await?;
        second.check_readiness().await?;
        second
            .register_task(proposal_registration("new-owner-task", 2, pipeline))
            .await?;

        assert!(!first.renew_namespace_owner(60).await?);
        assert!(first.check_readiness().await.is_err());
        assert!(
            first
                .register_task(proposal_registration("stale-owner-task", 3, pipeline))
                .await
                .is_err()
        );
        assert!(
            first
                .mark_proof_artifact_invalidated(
                    "l1-l2",
                    pipeline,
                    route,
                    proof_ref,
                    &artifact.content_hash,
                )
                .await
                .is_err()
        );
        assert!(
            !store
                .is_invalidated(
                    &RuntimeManager::artifact_key("l1-l2", pipeline, route, proof_ref),
                    artifact.generation,
                    &artifact.content_hash,
                )
                .await?
        );
        assert!(
            second
                .read_proof_artifact_bytes("l1-l2", pipeline, route, proof_ref)
                .await?
                .is_some()
        );
        Ok(())
    }
