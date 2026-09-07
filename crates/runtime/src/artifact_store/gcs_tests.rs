use super::*;
use alloy_primitives::B256;
use raiko2_pipeline::PipelineKey;
use raiko2_pipeline::forks::shasta::preflight_cache::{
    CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightDeleteResult, CanonicalPreflightKeyV1,
    CanonicalPreflightPutResult, CanonicalPreflightStore,
};
use raiko2_primitives::{L2BlockRange, ShastaCheckpoint};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Barrier, Notify};

#[test]
fn grpc_not_found_is_classified_as_a_missing_gcs_object() {
    use google_cloud_gax::error::{
        Error,
        rpc::{Code, Status},
    };

    let error = Error::service(Status::default().set_code(Code::NotFound));
    assert!(gcs_error_is_not_found(&error));
}

#[derive(Debug)]
struct FakeGcsTransport {
    objects: Mutex<BTreeMap<String, GcsObject>>,
    next_generation: Mutex<i64>,
    delete_failure: AtomicU8,
    deleted_names: Mutex<Vec<String>>,
    remove_first_listed_object: AtomicBool,
    list_page_size: AtomicUsize,
    list_failure: AtomicBool,
    active_deletes: AtomicUsize,
    max_active_deletes: AtomicUsize,
    block_next_delete: AtomicBool,
    remove_on_create_conflict: AtomicBool,
    delete_entered: Notify,
    allow_delete: Notify,
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
            active_deletes: AtomicUsize::new(0),
            max_active_deletes: AtomicUsize::new(0),
            block_next_delete: AtomicBool::new(false),
            remove_on_create_conflict: AtomicBool::new(false),
            delete_entered: Notify::new(),
            allow_delete: Notify::new(),
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

    fn names_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        Ok(self
            .objects
            .lock()
            .map_err(|_| anyhow::anyhow!("fake object lock poisoned"))?
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect())
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

    fn max_active_deletes(&self) -> usize {
        self.max_active_deletes.load(Ordering::SeqCst)
    }

    fn block_next_delete(&self) {
        self.block_next_delete.store(true, Ordering::SeqCst);
    }

    fn remove_on_next_create_conflict(&self) {
        self.remove_on_create_conflict.store(true, Ordering::SeqCst);
    }
}

struct ActiveDeleteGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveDeleteGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
            if self.remove_on_create_conflict.swap(false, Ordering::SeqCst) {
                objects.remove(name);
            }
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
        let active = self.active_deletes.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active_deletes.fetch_max(active, Ordering::SeqCst);
        let _active = ActiveDeleteGuard(&self.active_deletes);
        if self.block_next_delete.swap(false, Ordering::SeqCst) {
            self.delete_entered.notify_one();
            self.allow_delete.notified().await;
        }
        tokio::task::yield_now().await;
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

fn canonical_preflight_key() -> CanonicalPreflightKeyV1 {
    CanonicalPreflightKeyV1 {
        schema: CANONICAL_PREFLIGHT_SCHEMA_V1,
        blob_proof_type: Default::default(),
        l1_chain_id: 32_382,
        l2_chain_id: 167_001,
        proposal_id: 42,
        l2_block_range: L2BlockRange {
            start: 100,
            end: 102,
        },
        l1_inclusion_block_number: 77,
        last_anchor_block_number: 99,
        checkpoint: Some(ShastaCheckpoint {
            block_number: 102,
            block_hash: B256::repeat_byte(0x11),
            state_root: B256::repeat_byte(0x22),
        }),
        l1_inclusion_hash: B256::repeat_byte(0x33),
        proposal_event_digest: B256::repeat_byte(0x44),
        chain_rules_fingerprint: B256::repeat_byte(0x55),
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
async fn canonical_preflight_publication_roundtrips_and_reuses_identical_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = canonical_preflight_key();
    let bytes = b"canonical-preflight";

    let first = store.put_canonical_preflight_if_absent(&key, bytes).await?;
    let second = store.put_canonical_preflight_if_absent(&key, bytes).await?;

    assert!(matches!(first, CanonicalPreflightPutResult::Created(_)));
    assert!(matches!(
        second,
        CanonicalPreflightPutResult::AlreadyExists(_)
    ));
    assert_eq!(
        store
            .get_canonical_preflight(&key)
            .await?
            .expect("canonical preflight")
            .bytes,
        bytes
    );
    let object_name = store.canonical_preflight_object_name(&key)?;
    assert!(object_name.contains(&format!("/preflights/v{CANONICAL_PREFLIGHT_SCHEMA_V1}/")));
    assert!(object_name.ends_with(&format!("{:x}.preflight.bincode", key.digest()?)));
    assert_ne!(
        store.canonical_preflight_version_prefix(key.schema),
        store.canonical_preflight_version_prefix(key.schema + 1)
    );
    assert!(transport.contains(&object_name)?);
    assert_eq!(
        transport.names_with_prefix(&format!("{}/preflights/", store.scope_prefix()))?,
        vec![object_name]
    );
    Ok(())
}

#[tokio::test]
async fn delayed_incompatible_version_create_is_unreachable_from_current_version() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = canonical_preflight_key();
    let incompatible_version = key.schema.saturating_sub(1);
    let delayed_object = format!(
        "{}/{:x}.preflight.bincode",
        store.canonical_preflight_version_prefix(incompatible_version),
        key.digest()?
    );
    let current_object = store.canonical_preflight_object_name(&key)?;

    assert_ne!(delayed_object, current_object);
    assert!(matches!(
        transport.create(&delayed_object, b"old-version").await?,
        GcsCreateResult::Created(_)
    ));
    assert!(transport.contains(&delayed_object)?);
    assert!(store.get_canonical_preflight(&key).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn canonical_preflight_object_is_first_write_wins() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(transport)?;
    let key = canonical_preflight_key();
    let first = store
        .put_canonical_preflight_if_absent(&key, b"first")
        .await?
        .try_object()
        .expect("first publication")
        .clone();

    let conflict = store
        .put_canonical_preflight_if_absent(&key, b"second")
        .await?;

    let CanonicalPreflightPutResult::Conflict(descriptor) = conflict else {
        anyhow::bail!("different canonical content did not conflict");
    };
    assert_eq!(descriptor, first.descriptor());
    assert_eq!(
        store
            .get_canonical_preflight(&key)
            .await?
            .expect("winning canonical preflight")
            .bytes,
        b"first"
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_canonical_preflight_publication_has_one_winner() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = Arc::new(store(transport)?);
    let key = canonical_preflight_key();
    let barrier = Arc::new(Barrier::new(3));
    let mut attempts = Vec::new();
    for bytes in [b"canonical-a".as_slice(), b"canonical-b".as_slice()] {
        let store = Arc::clone(&store);
        let key = key.clone();
        let barrier = Arc::clone(&barrier);
        let bytes = bytes.to_vec();
        attempts.push(tokio::spawn(async move {
            barrier.wait().await;
            let result = store
                .put_canonical_preflight_if_absent(&key, &bytes)
                .await?;
            Ok::<_, anyhow::Error>((bytes, result))
        }));
    }
    barrier.wait().await;

    let mut winner = None;
    let mut conflicts = 0;
    for attempt in attempts {
        let (bytes, result) = attempt.await??;
        match result {
            CanonicalPreflightPutResult::Created(_) => winner = Some(bytes),
            CanonicalPreflightPutResult::Conflict(_) => conflicts += 1,
            CanonicalPreflightPutResult::AlreadyExists(_) => {
                anyhow::bail!("different concurrent payloads cannot be identical")
            }
        }
    }

    let winner = winner.expect("one concurrent publisher wins");
    assert_eq!(conflicts, 1);
    assert_eq!(
        store
            .get_canonical_preflight(&key)
            .await?
            .expect("winning canonical preflight")
            .bytes,
        winner
    );
    Ok(())
}

#[tokio::test]
async fn canonical_preflight_invalidation_is_generation_scoped() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = canonical_preflight_key();
    let first = store
        .put_canonical_preflight_if_absent(&key, b"first")
        .await?
        .try_object()
        .expect("first publication")
        .clone();
    let mut first_version = first.descriptor();
    first_version.content_hash = "diagnostic-hash-is-not-the-delete-fence".to_string();

    assert_eq!(
        store
            .delete_canonical_preflight_exact(&key, &first_version)
            .await?,
        CanonicalPreflightDeleteResult::Removed
    );
    assert!(!transport.contains(&store.canonical_preflight_object_name(&key)?)?);
    let second = store
        .put_canonical_preflight_if_absent(&key, b"second")
        .await?
        .try_object()
        .expect("replacement publication")
        .clone();

    assert_ne!(first.generation, second.generation);
    assert_eq!(
        store
            .delete_canonical_preflight_exact(&key, &first.descriptor())
            .await?,
        CanonicalPreflightDeleteResult::Stale
    );
    assert_eq!(
        store
            .get_canonical_preflight(&key)
            .await?
            .expect("replacement canonical preflight")
            .bytes,
        b"second"
    );
    Ok(())
}

#[tokio::test]
async fn canonical_preflight_delete_retries_precommit_error_by_storage_version() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = canonical_preflight_key();
    let object = store
        .put_canonical_preflight_if_absent(&key, b"canonical-preflight")
        .await?
        .try_object()
        .expect("canonical preflight object")
        .clone();
    let mut version = object.descriptor();
    version.content_hash = "diagnostic-hash-is-not-the-delete-fence".to_string();
    transport.delete_failure.store(1, Ordering::SeqCst);

    let error = store
        .delete_canonical_preflight_exact(&key, &version)
        .await
        .expect_err("a pre-commit delete failure must remain retryable");

    assert!(error.to_string().contains("before commit"));
    assert!(store.get_canonical_preflight(&key).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn canonical_preflight_delete_recovers_postcommit_error_by_readback() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = canonical_preflight_key();
    let object = store
        .put_canonical_preflight_if_absent(&key, b"canonical-preflight")
        .await?
        .try_object()
        .expect("canonical preflight object")
        .clone();
    transport.delete_failure.store(2, Ordering::SeqCst);

    assert_eq!(
        store
            .delete_canonical_preflight_exact(&key, &object.descriptor())
            .await?,
        CanonicalPreflightDeleteResult::Removed
    );
    assert!(store.get_canonical_preflight(&key).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn canonical_preflight_create_conflict_reports_disappeared_object() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = canonical_preflight_key();
    store
        .put_canonical_preflight_if_absent(&key, b"first")
        .await?;
    transport.remove_on_next_create_conflict();

    let error = store
        .put_canonical_preflight_if_absent(&key, b"second")
        .await
        .expect_err("a create conflict whose winner disappears must be explicit");

    assert!(error.to_string().contains("object is missing"));
    assert!(store.get_canonical_preflight(&key).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn gcs_canonical_preflight_rejects_unknown_schema_before_delete_fences() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(transport)?;
    let mut key = canonical_preflight_key();
    key.schema = CANONICAL_PREFLIGHT_SCHEMA_V1 + 1;
    let descriptor = CanonicalPreflightDescriptor {
        key_digest: B256::ZERO,
        content_hash: "unused".to_string(),
        generation: None,
    };

    let error = store
        .delete_canonical_preflight_exact(&key, &descriptor)
        .await
        .expect_err("unknown schema must fail before stale descriptor checks");

    assert!(
        error
            .to_string()
            .contains("unsupported canonical preflight key schema")
    );
    Ok(())
}

#[tokio::test]
async fn runtime_drain_waits_for_admitted_gcs_preflight_exact_delete() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = Arc::new(store(Arc::clone(&transport))?);
    let runtime = Arc::new(crate::RuntimeManager::from_shared_store(Arc::clone(&store)));
    let key = canonical_preflight_key();
    let object = runtime
        .put_canonical_preflight_if_absent(&key, b"canonical-preflight")
        .await?
        .try_object()
        .expect("canonical preflight object")
        .clone();
    transport.block_next_delete();

    let deletion = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        let key = key.clone();
        async move {
            runtime
                .delete_canonical_preflight_exact(&key, &object.descriptor())
                .await
        }
    });
    transport.delete_entered.notified().await;

    let mut draining = tokio::spawn({
        let runtime = Arc::clone(&runtime);
        async move { runtime.begin_draining().await }
    });
    let drained_early =
        match tokio::time::timeout(std::time::Duration::from_millis(20), &mut draining).await {
            Ok(result) => {
                result?;
                true
            }
            Err(_) => false,
        };

    transport.allow_delete.notify_one();
    assert_eq!(deletion.await??, CanonicalPreflightDeleteResult::Removed);
    if !drained_early {
        draining.await?;
    }

    assert!(
        !drained_early,
        "draining completed while an admitted GCS exact delete was in flight"
    );
    assert!(!transport.contains(&store.canonical_preflight_object_name(&key)?)?);
    Ok(())
}

#[tokio::test]
async fn gcs_preflight_exact_delete_is_rejected_after_draining_starts() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = Arc::new(store(Arc::clone(&transport))?);
    let runtime = crate::RuntimeManager::from_shared_store(Arc::clone(&store));
    let key = canonical_preflight_key();
    let object = runtime
        .put_canonical_preflight_if_absent(&key, b"canonical-preflight")
        .await?
        .try_object()
        .expect("canonical preflight object")
        .clone();

    runtime.start_draining();
    let error = runtime
        .delete_canonical_preflight_exact(&key, &object.descriptor())
        .await
        .expect_err("exact deletion must be rejected while draining");

    assert!(error.to_string().contains("runtime is draining"));
    assert!(transport.contains(&store.canonical_preflight_object_name(&key)?)?);
    Ok(())
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
async fn proof_read_rejects_manifest_with_missing_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let object = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    transport.remove(&store.content_name(&key, &object.content_hash))?;

    let error = store
        .get(&key)
        .await
        .expect_err("a manifest must not resolve without its immutable content");
    assert!(
        error
            .to_string()
            .contains("proof manifest references missing content")
    );
    Ok(())
}

#[tokio::test]
async fn proof_read_rejects_corrupted_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let object = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();
    transport.replace_bytes(
        &store.content_name(&key, &object.content_hash),
        br#"{"proof":"corrupt"}"#,
    )?;

    let error = store
        .get(&key)
        .await
        .expect_err("content that does not match the manifest hash must be rejected");
    assert!(
        error
            .to_string()
            .contains("proof manifest content hash mismatch")
    );
    Ok(())
}

#[tokio::test]
async fn exact_delete_allows_identical_republication() -> Result<()> {
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

    assert_eq!(
        store.delete_exact(&key, &first.descriptor()).await?,
        ExactDeleteResult::Removed
    );
    let second = store
        .put_if_absent(&key, proof)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    assert_ne!(first.generation, second.generation);
    assert_eq!(
        store.delete_exact(&key, &first.descriptor()).await?,
        ExactDeleteResult::Stale,
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
async fn delete_reports_removed_then_missing_through_gcs_seam() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let key = key();
    let object = store
        .put_if_absent(&key, br#"{"proof":"0x01"}"#)
        .await?
        .try_object()
        .expect("proof publication should materialize content")
        .clone();

    assert_eq!(
        store.delete_exact(&key, &object.descriptor()).await?,
        ExactDeleteResult::Removed
    );
    assert_eq!(
        store.delete_exact(&key, &object.descriptor()).await?,
        ExactDeleteResult::Missing
    );
    Ok(())
}

#[tokio::test]
async fn exact_delete_recovers_commit_then_error_by_readback() -> Result<()> {
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
        store.delete_exact(&key, &object.descriptor()).await?,
        ExactDeleteResult::Removed
    );
    assert_eq!(store.get_descriptor(&key).await?, None);
    Ok(())
}

#[tokio::test]
async fn exact_delete_retries_failure_before_commit() -> Result<()> {
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
        .delete_exact(&key, &object.descriptor())
        .await
        .expect_err("a pre-commit delete failure must remain retryable");
    assert!(error.to_string().contains("before commit"));
    assert_eq!(store.get_descriptor(&key).await?, Some(object.descriptor()));
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
async fn proof_startup_cleanup_keeps_preflight_and_immutable_content() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let proof_key = key();
    let proof = store
        .put_if_absent(&proof_key, b"proof")
        .await?
        .try_object()
        .expect("proof publication")
        .clone();
    let preflight_key = canonical_preflight_key();
    store
        .put_canonical_preflight_if_absent(&preflight_key, b"preflight")
        .await?;
    store.store_runtime_state(b"runtime", None).await?;

    let report = store
        .cleanup_before_start(StartupCleanupMask::PROOF)
        .await?;

    let proof_report = report
        .scope(StartupCleanupScope::Proof)
        .expect("proof cleanup report");
    assert_eq!(
        (
            proof_report.matched,
            proof_report.removed,
            proof_report.failed
        ),
        (2, 2, 0)
    );
    assert!(store.load_runtime_state().await?.is_none());
    assert!(store.get_descriptor(&proof_key).await?.is_none());
    assert!(
        store
            .get_canonical_preflight(&preflight_key)
            .await?
            .is_some()
    );
    assert!(transport.contains(&store.content_name(&proof_key, &proof.content_hash))?);
    assert!(transport.contains(&store.canonical_preflight_object_name(&preflight_key)?)?);
    Ok(())
}

#[tokio::test]
async fn preflight_startup_cleanup_keeps_runtime_and_proof() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let proof_key = key();
    let proof = store
        .put_if_absent(&proof_key, b"proof")
        .await?
        .try_object()
        .expect("proof publication")
        .clone();
    let preflight_key = canonical_preflight_key();
    store
        .put_canonical_preflight_if_absent(&preflight_key, b"preflight")
        .await?;
    let legacy_base = store.canonical_preflight_base_name(&preflight_key)?;
    let legacy_manifest = format!("{legacy_base}/manifest.manifest.json");
    let legacy_content = format!("{legacy_base}/content/legacy.preflight.bincode");
    assert!(matches!(
        transport
            .create(&legacy_manifest, b"legacy-manifest")
            .await?,
        GcsCreateResult::Created(_)
    ));
    assert!(matches!(
        transport.create(&legacy_content, b"legacy-content").await?,
        GcsCreateResult::Created(_)
    ));
    store.store_runtime_state(b"runtime", None).await?;

    let report = store
        .cleanup_before_start(StartupCleanupMask::PREFLIGHT)
        .await?;

    let preflight_report = report
        .scope(StartupCleanupScope::Preflight)
        .expect("preflight cleanup report");
    assert_eq!(
        (
            preflight_report.matched,
            preflight_report.removed,
            preflight_report.failed
        ),
        (3, 3, 0)
    );
    assert!(store.load_runtime_state().await?.is_some());
    assert_eq!(
        store.get_descriptor(&proof_key).await?,
        Some(proof.descriptor())
    );
    assert!(
        store
            .get_canonical_preflight(&preflight_key)
            .await?
            .is_none()
    );
    assert!(!transport.contains(&store.canonical_preflight_object_name(&preflight_key)?)?);
    assert!(!transport.contains(&legacy_manifest)?);
    assert!(!transport.contains(&legacy_content)?);
    Ok(())
}

#[tokio::test]
async fn startup_cleanup_paginates_and_bounds_manifest_deletes() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    transport.set_list_page_size(70);
    let store = store(Arc::clone(&transport))?;
    let proofs_prefix = format!("{}/proofs/", store.scope_prefix());
    for index in 0..130 {
        let name = format!("{proofs_prefix}lane/pair/proposal-{index}/manifest.manifest.json");
        assert!(matches!(
            transport.create(&name, b"manifest").await?,
            GcsCreateResult::Created(_)
        ));
    }
    let content = format!("{proofs_prefix}lane/pair/proposal-0/content/proof.json");
    assert!(matches!(
        transport.create(&content, b"proof").await?,
        GcsCreateResult::Created(_)
    ));

    let report = store
        .cleanup_before_start(StartupCleanupMask::PROOF)
        .await?;

    let proof_report = report
        .scope(StartupCleanupScope::Proof)
        .expect("proof cleanup report");
    assert_eq!((proof_report.matched, proof_report.removed), (130, 130));
    assert!(transport.max_active_deletes() > 1);
    assert!(
        transport.max_active_deletes() <= STARTUP_CLEANUP_DELETE_CONCURRENCY,
        "delete concurrency exceeded configured bound"
    );
    assert!(transport.contains(&content)?);
    Ok(())
}

#[tokio::test]
async fn proof_startup_cleanup_stops_before_manifests_when_runtime_delete_fails() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let proof_key = key();
    store.put_if_absent(&proof_key, b"proof").await?;
    store.store_runtime_state(b"runtime", None).await?;
    transport.delete_failure.store(1, Ordering::SeqCst);

    let error = store
        .cleanup_before_start(StartupCleanupMask::PROOF)
        .await
        .expect_err("runtime-state deletion failure must abort cleanup");

    assert!(error.to_string().contains("failed to delete runtime state"));
    assert!(transport.contains(&store.runtime_state_name())?);
    assert!(transport.contains(&store.manifest_name(&proof_key))?);

    let report = store
        .cleanup_before_start(StartupCleanupMask::PROOF)
        .await?;
    assert_eq!(
        report
            .scope(StartupCleanupScope::Proof)
            .expect("proof report")
            .removed,
        2
    );
    Ok(())
}

#[tokio::test]
async fn startup_cleanup_retry_is_idempotent_after_unknown_delete_outcome() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let proof_key = key();
    let proof = store
        .put_if_absent(&proof_key, b"proof")
        .await?
        .try_object()
        .expect("proof publication")
        .clone();
    transport.delete_failure.store(2, Ordering::SeqCst);

    store
        .cleanup_before_start(StartupCleanupMask::PROOF)
        .await
        .expect_err("unknown manifest delete outcome must abort cleanup");
    assert!(!transport.contains(&store.manifest_name(&proof_key))?);
    assert!(transport.contains(&store.content_name(&proof_key, &proof.content_hash))?);

    let report = store
        .cleanup_before_start(StartupCleanupMask::PROOF)
        .await?;
    let proof_report = report
        .scope(StartupCleanupScope::Proof)
        .expect("proof cleanup report");
    assert_eq!((proof_report.matched, proof_report.removed), (0, 0));
    Ok(())
}

#[tokio::test]
async fn all_startup_cleanup_removes_proof_and_preflight_scopes_in_order() -> Result<()> {
    let transport = Arc::new(FakeGcsTransport::default());
    let store = store(Arc::clone(&transport))?;
    let proof_key = key();
    store.put_if_absent(&proof_key, b"proof").await?;
    let preflight_key = canonical_preflight_key();
    store
        .put_canonical_preflight_if_absent(&preflight_key, b"preflight")
        .await?;
    store.store_runtime_state(b"runtime", None).await?;

    let report = store.cleanup_before_start(StartupCleanupMask::ALL).await?;

    assert_eq!(
        report
            .scopes
            .iter()
            .map(|entry| entry.scope)
            .collect::<Vec<_>>(),
        vec![StartupCleanupScope::Proof, StartupCleanupScope::Preflight]
    );
    assert!(!transport.contains(&store.runtime_state_name())?);
    assert!(!transport.contains(&store.manifest_name(&proof_key))?);
    assert!(!transport.contains(&store.canonical_preflight_object_name(&preflight_key)?)?);
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
