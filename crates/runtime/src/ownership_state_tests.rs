#[derive(Debug)]
enum OwnerFailureMode {
    Renew,
    Verify,
}

#[derive(Debug)]
struct OwnerFailureStore {
    mode: OwnerFailureMode,
    remaining_failures: std::sync::atomic::AtomicUsize,
}

#[derive(Clone, Copy, Debug)]
enum RuntimeWriteFailureMode {
    CommitThenError,
    ConflictThenRetry,
}

#[derive(Debug)]
struct RuntimeWriteFailureStore {
    inner: MemoryProofArtifactStore,
    mode: RuntimeWriteFailureMode,
    first_write: std::sync::atomic::AtomicBool,
}

impl RuntimeWriteFailureStore {
    fn new(namespace: &str, mode: RuntimeWriteFailureMode) -> Result<Self> {
        Ok(Self {
            inner: MemoryProofArtifactStore::new("test".into(), namespace.into())?,
            mode,
            first_write: std::sync::atomic::AtomicBool::new(true),
        })
    }
}

#[async_trait::async_trait]
impl ProofArtifactStore for RuntimeWriteFailureStore {
    fn environment(&self) -> &str {
        self.inner.environment()
    }

    fn namespace(&self) -> &str {
        self.inner.namespace()
    }

    fn backend_name(&self) -> &'static str {
        "runtime-write-failure"
    }

    async fn put_if_absent(
        &self,
        _key: &ProofArtifactKey,
        _bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        unreachable!("artifact operations are not used by runtime write tests")
    }

    async fn get(&self, _key: &ProofArtifactKey) -> Result<Option<ProofArtifactObject>> {
        unreachable!("artifact operations are not used by runtime write tests")
    }

    async fn get_prefix(
        &self,
        _key: &ProofArtifactKey,
        _max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>> {
        unreachable!("artifact operations are not used by runtime write tests")
    }

    async fn mark_invalidated(
        &self,
        _key: &ProofArtifactKey,
        _generation: Option<i64>,
        _content_hash: &str,
    ) -> Result<()> {
        unreachable!("artifact operations are not used by runtime write tests")
    }

    async fn is_invalidated(
        &self,
        _key: &ProofArtifactKey,
        _generation: Option<i64>,
        _content_hash: &str,
    ) -> Result<bool> {
        unreachable!("artifact operations are not used by runtime write tests")
    }

    async fn delete(
        &self,
        _key: &ProofArtifactKey,
        _generation: Option<i64>,
        _expected_content_hash: &str,
    ) -> Result<()> {
        unreachable!("artifact operations are not used by runtime write tests")
    }

    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
        self.inner.load_runtime_state().await
    }

    async fn store_runtime_state(
        &self,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<RuntimeStateWriteResult> {
        if self.first_write.swap(false, Ordering::AcqRel) {
            match self.mode {
                RuntimeWriteFailureMode::CommitThenError => {
                    let _ = self
                        .inner
                        .store_runtime_state(bytes, expected_generation)
                        .await?;
                    anyhow::bail!("simulated response loss after committed write");
                }
                RuntimeWriteFailureMode::ConflictThenRetry => {
                    let baseline = serde_json::to_vec(&RuntimeState::default())?;
                    let _ = self
                        .inner
                        .store_runtime_state(&baseline, expected_generation)
                        .await?;
                    return Ok(RuntimeStateWriteResult::Conflict(
                        self.inner.load_runtime_state().await?,
                    ));
                }
            }
        }
        self.inner
            .store_runtime_state(bytes, expected_generation)
            .await
    }

    async fn claim_namespace_owner(
        &self,
        owner_id: &str,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        self.inner
            .claim_namespace_owner(owner_id, now_secs, lease_secs)
            .await
    }

    async fn renew_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        now_secs: u64,
        lease_secs: u64,
    ) -> Result<Option<NamespaceOwnerLease>> {
        self.inner
            .renew_namespace_owner(lease, now_secs, lease_secs)
            .await
    }

    async fn verify_namespace_owner(
        &self,
        lease: &NamespaceOwnerLease,
        now_secs: u64,
    ) -> Result<bool> {
        self.inner.verify_namespace_owner(lease, now_secs).await
    }

    async fn release_namespace_owner(&self, lease: &NamespaceOwnerLease) -> Result<bool> {
        self.inner.release_namespace_owner(lease).await
    }
}

#[async_trait::async_trait]
impl ProofArtifactStore for OwnerFailureStore {
    fn environment(&self) -> &'static str {
        "test"
    }

    fn namespace(&self) -> &'static str {
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

    async fn load_runtime_state(&self) -> Result<Option<RuntimeStateObject>> {
        Ok(None)
    }

    async fn store_runtime_state(
        &self,
        _bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<RuntimeStateWriteResult> {
        Ok(RuntimeStateWriteResult::Stored {
            generation: Some(expected_generation.unwrap_or(0).saturating_add(1)),
        })
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
            OwnerFailureMode::Renew
                if self
                    .remaining_failures
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok() =>
            {
                anyhow::bail!("authoritative owner renewal unavailable")
            }
            OwnerFailureMode::Renew => Ok(Some(artifact_store::NamespaceOwnerLease {
                owner_id: _lease.owner_id.clone(),
                epoch: _lease.epoch,
                expires_at_secs: _now_secs.saturating_add(_lease_secs),
                generation: _lease.generation,
            })),
            OwnerFailureMode::Verify => unreachable!("renew is not used by readiness test"),
        }
    }

    async fn verify_namespace_owner(
        &self,
        _lease: &artifact_store::NamespaceOwnerLease,
        _now_secs: u64,
    ) -> Result<bool> {
        match self.mode {
            OwnerFailureMode::Verify
                if self
                    .remaining_failures
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok() =>
            {
                anyhow::bail!("authoritative owner read unavailable")
            }
            OwnerFailureMode::Verify | OwnerFailureMode::Renew => Ok(true),
        }
    }

    async fn release_namespace_owner(
        &self,
        _lease: &artifact_store::NamespaceOwnerLease,
    ) -> Result<bool> {
        Ok(true)
    }
}

#[tokio::test]
async fn owner_store_read_failure_makes_runtime_unready() -> Result<()> {
    let runtime = RuntimeManager::with_store(Arc::new(OwnerFailureStore {
        mode: OwnerFailureMode::Verify,
        remaining_failures: std::sync::atomic::AtomicUsize::new(1),
    }))?;
    runtime.acquire_namespace_owner(60).await?;

    let error = runtime
        .check_readiness()
        .await
        .expect_err("authoritative owner read failure must reject readiness");
    assert!(error.to_string().contains("authoritative owner read unavailable"));
    runtime.check_readiness().await?;
    runtime
        .register_task(proposal_registration(
            "recovered-after-owner-read-error",
            1,
            PipelineKey::ShastaNative,
        ))
        .await?;
    Ok(())
}

#[tokio::test]
async fn owner_renewal_failure_freezes_runtime_admissions() -> Result<()> {
    let runtime = RuntimeManager::with_store(Arc::new(OwnerFailureStore {
        mode: OwnerFailureMode::Renew,
        remaining_failures: std::sync::atomic::AtomicUsize::new(1),
    }))?;
    runtime.acquire_namespace_owner(60).await?;

    let error = runtime
        .renew_namespace_owner(60)
        .await
        .expect_err("authoritative renewal failure must propagate");
    assert!(error.to_string().contains("authoritative owner renewal unavailable"));
    runtime.check_readiness().await?;
    runtime
        .register_task(proposal_registration(
            "recovered-after-renewal-error",
            1,
            PipelineKey::ShastaNative,
        ))
        .await?;
    Ok(())
}

#[tokio::test]
async fn graceful_release_allows_immediate_namespace_replacement() -> Result<()> {
    let store = Arc::new(MemoryProofArtifactStore::new(
        "test".into(),
        "graceful-release".into(),
    )?);
    let first = RuntimeManager::with_store(store.clone())?;
    first.acquire_namespace_owner(60).await?;
    assert!(first.release_namespace_owner().await?);
    assert!(first.check_readiness().await.is_err());

    let second = RuntimeManager::with_store(store)?;
    second.acquire_namespace_owner(60).await?;
    second.check_readiness().await?;
    Ok(())
}

#[tokio::test]
async fn committed_write_with_lost_response_is_reconciled_without_generation_wedge() -> Result<()> {
    let runtime = RuntimeManager::with_store(Arc::new(RuntimeWriteFailureStore::new(
        "commit-then-error",
        RuntimeWriteFailureMode::CommitThenError,
    )?))?;
    runtime.acquire_namespace_owner(60).await?;
    runtime
        .register_task(proposal_registration(
            "first-after-ambiguous-write",
            1,
            PipelineKey::ShastaNative,
        ))
        .await?;
    runtime
        .register_task(proposal_registration(
            "second-after-ambiguous-write",
            2,
            PipelineKey::ShastaNative,
        ))
        .await?;
    runtime.check_readiness().await?;
    Ok(())
}

#[tokio::test]
async fn generation_conflict_reloads_and_reapplies_the_mutation() -> Result<()> {
    let runtime = RuntimeManager::with_store(Arc::new(RuntimeWriteFailureStore::new(
        "conflict-retry",
        RuntimeWriteFailureMode::ConflictThenRetry,
    )?))?;
    runtime.acquire_namespace_owner(60).await?;
    runtime
        .register_task(proposal_registration(
            "created-after-conflict",
            1,
            PipelineKey::ShastaNative,
        ))
        .await?;
    assert!(runtime.get_task("created-after-conflict").await?.is_some());
    runtime.check_readiness().await?;
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
