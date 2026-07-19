use super::*;
use raiko2_pipeline::PipelineKey;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

#[derive(Debug, Default)]
struct FakeGcsTransport {
    objects: Mutex<BTreeMap<String, GcsObject>>,
    next_generation: Mutex<i64>,
    delete_failure: AtomicU8,
}

impl FakeGcsTransport {
    fn generation(&self) -> Result<i64> {
        let mut next = self
            .next_generation
            .lock()
            .map_err(|_| anyhow::anyhow!("fake generation lock poisoned"))?;
        *next += 1;
        Ok(*next)
    }

    fn remove(&self, name: &str) -> Result<()> {
        self.objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?
            .remove(name);
        Ok(())
    }

    fn contains(&self, name: &str) -> Result<bool> {
        Ok(self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?
            .contains_key(name))
    }

    fn replace_bytes(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
        let object = objects
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("fake object does not exist"))?;
        object.bytes = bytes.to_vec();
        Ok(())
    }
}

#[async_trait]
impl GcsTransport for FakeGcsTransport {
    async fn read(&self, name: &str) -> Result<Option<GcsObject>> {
        let objects = self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
        let Some(object) = objects.get(name) else {
            return Ok(None);
        };
        Ok(Some(GcsObject {
            bytes: object.bytes.clone(),
            generation: object.generation,
        }))
    }

    async fn create(&self, name: &str, bytes: &[u8]) -> Result<GcsCreateResult> {
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
        if objects.contains_key(name) {
            return Ok(GcsCreateResult::AlreadyExists);
        }
        let generation = self.generation()?;
        objects.insert(
            name.to_string(),
            GcsObject {
                bytes: bytes.to_vec(),
                generation,
            },
        );
        Ok(GcsCreateResult::Created(generation))
    }

    async fn write_if_generation(
        &self,
        name: &str,
        bytes: &[u8],
        expected_generation: Option<i64>,
    ) -> Result<GcsWriteResult> {
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
        let current = objects.get(name).map(|object| object.generation);
        if current != expected_generation {
            return Ok(GcsWriteResult::Conflict);
        }
        let generation = self.generation()?;
        objects.insert(
            name.to_string(),
            GcsObject {
                bytes: bytes.to_vec(),
                generation,
            },
        );
        Ok(GcsWriteResult::Stored(generation))
    }

    async fn delete_if_generation(
        &self,
        name: &str,
        generation: Option<i64>,
    ) -> Result<ProofArtifactDeleteResult> {
        let failure = self.delete_failure.swap(0, Ordering::SeqCst);
        if failure == 1 {
            anyhow::bail!("injected GCS delete failure before commit");
        }
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
        let Some(current) = objects.get(name) else {
            return Ok(ProofArtifactDeleteResult::Missing);
        };
        anyhow::ensure!(
            generation.is_none_or(|generation| generation == current.generation),
            "fake GCS generation precondition failed"
        );
        objects.remove(name);
        if failure == 2 {
            anyhow::bail!("injected GCS delete failure after commit");
        }
        Ok(ProofArtifactDeleteResult::Removed)
    }
}

fn key() -> ProofArtifactKey {
    ProofArtifactKey {
        network_pair: "taiko_dev/ethereum".to_string(),
        pipeline_key: PipelineKey::ShastaSp1,
        route: PipelineKey::ShastaSp1.route(),
        proof_ref: "proposal-1".to_string(),
    }
}

fn store(transport: Arc<FakeGcsTransport>) -> Result<GcsProofArtifactStore> {
    GcsProofArtifactStore::with_transport(
        "test".to_string(),
        "gcs-seam".to_string(),
        "bucket".to_string(),
        "runtime",
        transport,
    )
}

#[tokio::test]
async fn same_hash_publication_repairs_missing_content_through_gcs_seam() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let proof = br#"{"proof":"0x01"}"#;
    let first = store
        .put_if_absent(&key, proof)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    transport.remove(&store.content_name(&key, &first.content_hash))?;
    let repaired = store.put_if_absent(&key, proof).await?;

    assert!(matches!(repaired, ProofArtifactPutResult::AlreadyExists(_)));
    assert_eq!(store.get(&key).await?.expect("repaired proof").bytes, proof);
    Ok(())
}

#[tokio::test]
async fn missing_manifest_rejects_corrupt_preexisting_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let proof = br#"{"proof":"0x01"}"#;
    let hash = content_hash(proof);
    let content_name = store.content_name(&key, &hash);
    assert!(matches!(
        transport
            .create(&content_name, br#"{"proof":"corrupted"}"#)
            .await?,
        GcsCreateResult::Created(_)
    ));

    let error = store
        .put_if_absent(&key, proof)
        .await
        .expect_err("corrupt immutable content must not gain a manifest");

    assert!(error.to_string().contains("content hash mismatch"));
    assert!(!transport.contains(&store.manifest_name(&key))?);
    Ok(())
}

#[tokio::test]
async fn different_hash_conflicts_with_dangling_manifest_through_gcs_seam() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let first = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    transport.remove(&store.content_name(&key, &first.content_hash))?;

    let conflict = store.put_if_absent(&key, br#"{"proof":"0x02"}"#).await?;

    let ProofArtifactPutResult::Conflict(conflict) = conflict else {
        anyhow::bail!("different proof did not conflict with dangling manifest");
    };
    assert_eq!(conflict.descriptor, first.descriptor());
    assert_eq!(conflict.object, None);
    let rejected_hash = content_hash(br#"{"proof":"0x02"}"#);
    assert!(
        !transport.contains(&store.content_name(&key, &rejected_hash))?,
        "a known manifest conflict must not upload unreferenced content"
    );
    Ok(())
}

#[tokio::test]
async fn descriptor_survives_missing_content_through_gcs_seam() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let first = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    transport.remove(&store.content_name(&key, &first.content_hash))?;

    assert_eq!(store.get_descriptor(&key).await?, Some(first.descriptor()));
    Ok(())
}

#[tokio::test]
async fn prefix_read_rejects_manifest_with_missing_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let proof = br#"{"proof":"0x01"}"#;
    let first = store
        .put_if_absent(&key, proof)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    transport.remove(&store.content_name(&key, &first.content_hash))?;

    let error = store
        .get_prefix(&key, 64)
        .await
        .expect_err("dangling manifest must be reported as corruption");
    assert!(error.to_string().contains("missing content"));
    Ok(())
}

#[tokio::test]
async fn prefix_read_rejects_corrupted_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let proof = br#"{"proof":"0x01"}"#;
    let first = store
        .put_if_absent(&key, proof)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    transport.replace_bytes(
        &store.content_name(&key, &first.content_hash),
        br#"{"proof":"corrupted"}"#,
    )?;

    let error = store
        .get_prefix(&key, 4)
        .await
        .expect_err("prefix reads must validate the complete immutable content");
    assert!(error.to_string().contains("content hash mismatch"));
    Ok(())
}

#[tokio::test]
async fn generation_scoped_invalidation_allows_identical_republication() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(transport)?;
    let key = key();
    let proof = br#"{"proof":"0x01"}"#;
    let first = store
        .put_if_absent(&key, proof)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    assert!(matches!(
        store.invalidate_exact(&key, &first.descriptor()).await?,
        ExactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
    ));
    let second = store
        .put_if_absent(&key, proof)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    assert_ne!(first.generation, second.generation);
    assert!(store.is_invalidated(&key, &first.descriptor()).await?);
    assert!(!store.is_invalidated(&key, &second.descriptor()).await?);
    assert!(
        store.delete_exact(&key, &first.descriptor()).await.is_err(),
        "a stale generation must not delete the replacement manifest"
    );
    assert_eq!(
        store
            .get(&key)
            .await?
            .expect("replacement proof")
            .generation,
        second.generation
    );
    Ok(())
}

#[tokio::test]
async fn exact_invalidation_recovers_commit_then_error_by_readback() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let object = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    transport.delete_failure.store(2, Ordering::SeqCst);

    assert_eq!(
        store.invalidate_exact(&key, &object.descriptor()).await?,
        ExactInvalidationResult::AlreadyInvalidated
    );
    assert!(store.is_invalidated(&key, &object.descriptor()).await?);
    assert_eq!(store.get_descriptor(&key).await?, None);
    assert_eq!(
        store.invalidate_exact(&key, &object.descriptor()).await?,
        ExactInvalidationResult::AlreadyInvalidated
    );
    Ok(())
}

#[tokio::test]
async fn exact_invalidation_retries_fail_before_commit() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let object = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    transport.delete_failure.store(1, Ordering::SeqCst);

    let error = store
        .invalidate_exact(&key, &object.descriptor())
        .await
        .expect_err("a pre-commit delete failure must remain retryable");
    assert!(error.to_string().contains("before commit"));
    assert_eq!(store.get_descriptor(&key).await?, Some(object.descriptor()));
    assert!(store.is_invalidated(&key, &object.descriptor()).await?);
    assert!(matches!(
        store.invalidate_exact(&key, &object.descriptor()).await?,
        ExactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
    ));
    Ok(())
}

#[tokio::test]
async fn delete_reports_removed_then_missing_through_gcs_seam() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(transport)?;
    let key = key();
    let object = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    assert_eq!(
        store.delete_exact(&key, &object.descriptor()).await?,
        ProofArtifactDeleteResult::Removed
    );
    assert_eq!(
        store.delete_exact(&key, &object.descriptor()).await?,
        ProofArtifactDeleteResult::Missing
    );
    Ok(())
}

#[tokio::test]
async fn runtime_state_conflict_reads_back_the_observed_generation() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let first = store.store_runtime_state(b"first", None).await?;
    let RuntimeStateWriteResult::Stored {
        generation: Some(first_generation),
    } = first
    else {
        anyhow::bail!("initial runtime state was not stored");
    };
    let concurrent = transport
        .write_if_generation(
            &store.runtime_state_name(),
            b"concurrent",
            Some(first_generation),
        )
        .await?;
    assert!(matches!(concurrent, GcsWriteResult::Stored(_)));

    let conflict = store
        .store_runtime_state(b"stale", Some(first_generation))
        .await?;
    let RuntimeStateWriteResult::Conflict(Some(observed)) = conflict else {
        anyhow::bail!("stale runtime state write did not return observed conflict");
    };
    assert_eq!(observed.bytes, b"concurrent");
    assert_ne!(observed.generation, Some(first_generation));
    Ok(())
}
