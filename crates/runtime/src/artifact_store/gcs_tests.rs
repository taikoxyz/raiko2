use super::*;
use raiko2_pipeline::PipelineKey;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
struct FakeGcsTransport {
    objects: Mutex<BTreeMap<String, GcsObject>>,
    next_generation: Mutex<i64>,
    delete_failure: AtomicU8,
    deleted_names: Mutex<Vec<String>>,
    remove_first_listed_object: AtomicBool,
    list_page_size: AtomicUsize,
    list_failure: AtomicBool,
}

impl Default for FakeGcsTransport {
    fn default() -> Self {
        Self {
            objects: Mutex::new(BTreeMap::new()),
            next_generation: Mutex::new(0),
            delete_failure: AtomicU8::new(0),
            deleted_names: Mutex::new(Vec::new()),
            remove_first_listed_object: AtomicBool::new(false),
            list_page_size: AtomicUsize::new(usize::MAX),
            list_failure: AtomicBool::new(false),
        }
    }
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

    fn deleted_names(&self) -> Result<Vec<String>> {
        Ok(self
            .deleted_names
            .lock()
            .map_err(|_| anyhow::anyhow!("fake deleted-name lock poisoned"))?
            .clone())
    }

    fn remove_first_listed_object_on_next_list(&self) {
        self.remove_first_listed_object
            .store(true, Ordering::SeqCst);
    }

    fn set_list_page_size(&self, page_size: usize) {
        self.list_page_size
            .store(page_size.max(1), Ordering::SeqCst);
    }

    fn fail_next_list(&self) {
        self.list_failure.store(true, Ordering::SeqCst);
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

    async fn list_prefix_page(
        &self,
        prefix: &str,
        page_token: Option<&str>,
    ) -> Result<GcsObjectPage> {
        if self.list_failure.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected GCS list failure");
        }
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?;
        let listed = objects
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .filter(|(name, _)| page_token.is_none_or(|token| name.as_str() > token))
            .map(|(name, object)| GcsObjectMetadata {
                name: name.clone(),
                generation: object.generation,
            })
            .collect::<Vec<_>>();
        let page_size = self.list_page_size.load(Ordering::SeqCst);
        let next_page_token = listed
            .get(page_size)
            .map(|_| listed[page_size - 1].name.clone());
        let listed = listed.into_iter().take(page_size).collect::<Vec<_>>();
        if self
            .remove_first_listed_object
            .swap(false, Ordering::SeqCst)
            && let Some(object) = listed.first()
        {
            objects.remove(&object.name);
        }
        Ok(GcsObjectPage {
            objects: listed,
            next_page_token,
        })
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
        drop(objects);
        self.deleted_names
            .lock()
            .map_err(|_| anyhow::anyhow!("fake deleted-name lock poisoned"))?
            .push(name.to_string());
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

#[tokio::test]
async fn namespace_reset_removes_only_the_configured_scope() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let proof = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    assert!(matches!(
        store.invalidate_exact(&key, &proof.descriptor()).await?,
        ExactInvalidationResult::Invalidated(ProofArtifactDeleteResult::Removed)
    ));
    assert!(matches!(
        store.store_runtime_state(b"runtime", None).await?,
        RuntimeStateWriteResult::Stored { .. }
    ));
    let live_key = ProofArtifactKey {
        proof_ref: "proposal-live".to_string(),
        ..key.clone()
    };
    let live_proof = store
        .put_if_absent(&live_key, br#"{"proof":"0x02"}"#)
        .await?
        .try_object()
        .expect("live proof publication should materialize content")
        .clone();
    let sibling = format!("{}-other/sentinel", store.scope_prefix());
    assert!(matches!(
        transport.create(&sibling, b"sibling").await?,
        GcsCreateResult::Created(_)
    ));

    assert_eq!(store.reset_namespace().await?, 5);
    assert!(!transport.contains(&store.content_name(&key, &proof.content_hash))?);
    assert!(!transport.contains(&store.invalidation_name(
        &key,
        proof.generation,
        &proof.content_hash
    ))?);
    assert!(!transport.contains(&store.runtime_state_name())?);
    assert!(!transport.contains(&store.content_name(&live_key, &live_proof.content_hash))?);
    assert!(!transport.contains(&store.manifest_name(&live_key))?);
    assert!(transport.contains(&sibling)?);

    let deleted = transport.deleted_names()?;
    let runtime_state_index = deleted
        .iter()
        .position(|name| name == &store.runtime_state_name())
        .expect("runtime state must be deleted");
    let manifest_index = deleted
        .iter()
        .position(|name| name == &store.manifest_name(&live_key))
        .expect("live manifest must be deleted");
    let content_index = deleted
        .iter()
        .position(|name| name == &store.content_name(&live_key, &live_proof.content_hash))
        .expect("live content must be deleted");
    assert!(runtime_state_index < manifest_index);
    assert!(manifest_index < content_index);
    Ok(())
}

#[tokio::test]
async fn namespace_reset_accepts_objects_removed_after_listing() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    assert!(matches!(
        store.store_runtime_state(b"runtime", None).await?,
        RuntimeStateWriteResult::Stored { .. }
    ));
    transport.remove_first_listed_object_on_next_list();

    assert_eq!(store.reset_namespace().await?, 1);
    assert!(!transport.contains(&store.runtime_state_name())?);
    Ok(())
}

#[tokio::test]
async fn namespace_reset_paginates_current_objects() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    transport.set_list_page_size(1);
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let proof = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    assert!(matches!(
        store.store_runtime_state(b"runtime", None).await?,
        RuntimeStateWriteResult::Stored { .. }
    ));

    assert_eq!(store.reset_namespace().await?, 3);
    assert!(!transport.contains(&store.runtime_state_name())?);
    assert!(!transport.contains(&store.manifest_name(&key))?);
    assert!(!transport.contains(&store.content_name(&key, &proof.content_hash))?);
    Ok(())
}

#[tokio::test]
async fn namespace_reset_reports_list_failures() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    transport.fail_next_list();

    let error = store
        .reset_namespace()
        .await
        .expect_err("namespace reset must stop on a list failure");
    assert!(error.to_string().contains("injected GCS list failure"));
    Ok(())
}

#[tokio::test]
async fn namespace_reset_reports_delete_failures() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    assert!(matches!(
        store.store_runtime_state(b"runtime", None).await?,
        RuntimeStateWriteResult::Stored { .. }
    ));
    transport.delete_failure.store(1, Ordering::SeqCst);

    let error = store
        .reset_namespace()
        .await
        .expect_err("namespace reset must stop on a delete failure");
    assert!(
        error
            .to_string()
            .contains("failed to delete GCS runtime namespace object")
    );
    assert!(transport.contains(&store.runtime_state_name())?);
    Ok(())
}
