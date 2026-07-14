# Proof Artifact TTL Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically remove expired SQLite-backed proof artifacts and their files without deleting artifacts used by non-terminal roots or breaking aggregate reuse.

**Architecture:** `crates/runtime` provides bounded cursor queries and compare-and-delete storage primitives. `bin/raiko2` owns reference derivation, cleanup policy, structured statistics, and a shared read/write guard that orders cleanup against artifact-consuming submissions. Root-task and artifact cleanup share one maintenance task but use independent TTLs and cursors.

**Tech Stack:** Rust 2024, Tokio, tokio-rusqlite/rusqlite, Serde/TOML, Axum, tracing, Cargo tests and Clippy.

## Global Constraints

- `runtime.proof_artifact_ttl_secs` defaults to exactly `604800` seconds (seven days).
- `runtime.proof_artifact_ttl_secs = 0` disables only automatic artifact cleanup.
- Retain artifacts referenced by roots whose status is `allocated` or `running`.
- Include canonical and legacy proposal refs, canonical and legacy aggregate refs, and external aggregate-input refs.
- Delete the SQLite row before best-effort file removal; missing files are not failures.
- Keep `POST /v4/prover/invalidate-artifacts`, artifact key derivation, and root-task TTL semantics unchanged.
- Process artifacts in batches of 64 and emit one compact structured log for each non-idle pass.
- Follow the approved design in `docs/superpowers/specs/2026-07-14-proof-artifact-ttl-cleanup-design.md`.

## File Structure

- `crates/runtime/src/lib.rs`: artifact cursor, bounded expiration query, conditional row deletion, and storage tests.
- `bin/raiko2/src/config/runtime.rs`: new TTL setting and seven-day default.
- `bin/raiko2/src/config/mod.rs`: configuration default/disablement tests.
- `bin/raiko2/src/server/task_metadata.rs`: single source of truth for every artifact ref held by a root.
- `bin/raiko2/src/server/task_cleanup.rs`: artifact cleanup pass, statistics/logging, cursor lifecycle, file deletion, and cleanup tests.
- `bin/raiko2/src/server/state/mod.rs`: shared artifact cleanup guard and maintenance-loop wiring.
- `bin/raiko2/src/server/handlers/proof_api/v3.rs`: shared guard around legacy batch and external aggregate submission.
- `bin/raiko2/src/server/handlers/proof_api/v4.rs`: shared guard around v4 proposal submission/recovery.
- `config.example.toml`: operator-facing setting and comments.
- `docs/API.md`: runtime retention semantics.

---

### Task 1: Add bounded artifact storage primitives

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Test: `crates/runtime/src/lib.rs`

**Interfaces:**
- Consumes: existing `ProofArtifactRecord`, `RuntimeManager::connection`, and `proof_artifacts(updated_at)` index.
- Produces: `ProofArtifactCursor`; `RuntimeManager::list_expired_proof_artifacts(now_ts, ttl_secs, after, limit)`; `RuntimeManager::remove_proof_artifact_if_unchanged(network_pair, proof_ref, expected_updated_at)`.

- [ ] **Step 1: Write failing cursor and conditional-delete tests**

Add `ProofArtifactCursor` to the test imports and add these helpers/tests inside `crates/runtime/src/lib.rs`'s existing `tests` module:

```rust
async fn set_artifact_updated_at(
    runtime: &RuntimeManager,
    network_pair: &str,
    proof_ref: &str,
    updated_at: i64,
) -> anyhow::Result<()> {
    let conn = runtime.connection().await?;
    let network_pair = network_pair.to_string();
    let proof_ref = proof_ref.to_string();
    conn.call(move |conn| {
        conn.execute(
            "UPDATE proof_artifacts SET updated_at = ?1 WHERE network_pair = ?2 AND proof_ref = ?3",
            params![updated_at, network_pair, proof_ref],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn register_test_artifact(
    runtime: &RuntimeManager,
    proof_ref: &str,
) -> anyhow::Result<()> {
    runtime
        .upsert_proof_artifact(ProofArtifactRegistration {
            network_pair: "taiko_dev/ethereum".to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key: raiko2_pipeline::PipelineKey::ShastaRisc0,
            route: "risc0/local".parse().expect("parse route"),
            proof_path: runtime
                .proof_artifact_path("taiko_dev/ethereum", proof_ref)
                .display()
                .to_string(),
        })
        .await
}

#[tokio::test]
async fn runtime_manager_pages_expired_proof_artifacts_by_stable_cursor() -> anyhow::Result<()> {
    let root = unique_root("raiko2-runtime-expired-artifacts");
    let runtime = RuntimeManager::new(root.clone())?;
    for proof_ref in ["proposal-a", "proposal-b", "proposal-fresh"] {
        register_test_artifact(&runtime, proof_ref).await?;
    }
    set_artifact_updated_at(&runtime, "taiko_dev/ethereum", "proposal-a", 10).await?;
    set_artifact_updated_at(&runtime, "taiko_dev/ethereum", "proposal-b", 10).await?;
    set_artifact_updated_at(&runtime, "taiko_dev/ethereum", "proposal-fresh", 21).await?;

    let first = runtime
        .list_expired_proof_artifacts(100, 80, None, 1)
        .await?;
    assert_eq!(first.iter().map(|row| row.proof_ref.as_str()).collect::<Vec<_>>(), ["proposal-a"]);
    let cursor = ProofArtifactCursor::from(&first[0]);
    let second = runtime
        .list_expired_proof_artifacts(100, 80, Some(&cursor), 1)
        .await?;
    assert_eq!(second.iter().map(|row| row.proof_ref.as_str()).collect::<Vec<_>>(), ["proposal-b"]);
    let cursor = ProofArtifactCursor::from(&second[0]);
    assert!(runtime
        .list_expired_proof_artifacts(100, 80, Some(&cursor), 1)
        .await?
        .is_empty());
    assert!(runtime
        .list_expired_proof_artifacts(100, 0, None, 10)
        .await?
        .is_empty());
    assert!(runtime
        .list_expired_proof_artifacts(100, 80, None, 0)
        .await?
        .is_empty());

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn runtime_manager_conditionally_removes_unchanged_artifact_rows() -> anyhow::Result<()> {
    let root = unique_root("raiko2-runtime-conditional-artifact-remove");
    let runtime = RuntimeManager::new(root.clone())?;
    register_test_artifact(&runtime, "proposal-a").await?;
    set_artifact_updated_at(&runtime, "taiko_dev/ethereum", "proposal-a", 10).await?;

    register_test_artifact(&runtime, "proposal-a").await?;
    let refreshed = runtime
        .get_proof_artifact("taiko_dev/ethereum", "proposal-a")
        .await?
        .expect("refreshed artifact");
    assert!(refreshed.updated_at > 10);
    assert!(!runtime
        .remove_proof_artifact_if_unchanged("taiko_dev/ethereum", "proposal-a", 10)
        .await?);
    assert!(runtime
        .get_proof_artifact("taiko_dev/ethereum", "proposal-a")
        .await?
        .is_some());

    assert!(runtime
        .remove_proof_artifact_if_unchanged(
            "taiko_dev/ethereum",
            "proposal-a",
            refreshed.updated_at,
        )
        .await?);
    assert!(runtime
        .get_proof_artifact("taiko_dev/ethereum", "proposal-a")
        .await?
        .is_none());

    std::fs::remove_dir_all(root)?;
    Ok(())
}
```

- [ ] **Step 2: Run the storage tests and verify they fail**

Run:

```bash
cargo test -p raiko2-runtime expired_proof_artifacts -- --nocapture
cargo test -p raiko2-runtime conditionally_removes_unchanged -- --nocapture
```

Expected: compilation fails because `ProofArtifactCursor`, `list_expired_proof_artifacts`, and `remove_proof_artifact_if_unchanged` do not exist.

- [ ] **Step 3: Implement the cursor and bounded query**

Add beside `ExpiredTaskCursor`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofArtifactCursor {
    pub updated_at: i64,
    pub network_pair: String,
    pub proof_ref: String,
}

impl From<&ProofArtifactRecord> for ProofArtifactCursor {
    fn from(record: &ProofArtifactRecord) -> Self {
        Self {
            updated_at: record.updated_at,
            network_pair: record.network_pair.clone(),
            proof_ref: record.proof_ref.clone(),
        }
    }
}
```

Add after `list_proof_artifacts`:

```rust
pub async fn list_expired_proof_artifacts(
    &self,
    now_ts: i64,
    ttl_secs: u64,
    after: Option<&ProofArtifactCursor>,
    limit: usize,
) -> Result<Vec<ProofArtifactRecord>> {
    if ttl_secs == 0 || limit == 0 {
        return Ok(Vec::new());
    }

    let conn = self.connection().await?;
    let cutoff = now_ts.saturating_sub(i64::try_from(ttl_secs).unwrap_or(i64::MAX));
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let after = after.cloned();
    conn.call(move |conn| {
        let mut stmt = if after.is_some() {
            conn.prepare(
                r"
                SELECT network_pair, proof_ref, pipeline_key, route, proof_path, updated_at
                FROM proof_artifacts
                WHERE updated_at <= ?1
                  AND (
                    updated_at > ?2
                    OR (updated_at = ?2 AND network_pair > ?3)
                    OR (updated_at = ?2 AND network_pair = ?3 AND proof_ref > ?4)
                  )
                ORDER BY updated_at ASC, network_pair ASC, proof_ref ASC
                LIMIT ?5
                ",
            )?
        } else {
            conn.prepare(
                r"
                SELECT network_pair, proof_ref, pipeline_key, route, proof_path, updated_at
                FROM proof_artifacts
                WHERE updated_at <= ?1
                ORDER BY updated_at ASC, network_pair ASC, proof_ref ASC
                LIMIT ?2
                ",
            )?
        };
        let mut rows = if let Some(after) = after {
            stmt.query(params![
                cutoff,
                after.updated_at,
                after.network_pair,
                after.proof_ref,
                limit
            ])?
        } else {
            stmt.query(params![cutoff, limit])?
        };
        let mut artifacts = Vec::new();
        while let Some(row) = rows.next()? {
            artifacts.push(proof_artifact_record_from_row(row)?);
        }
        Ok(artifacts)
    })
    .await
    .context("failed to list expired proof artifacts")
}
```

- [ ] **Step 4: Make artifact upserts monotonic and implement compare-and-delete**

In `upsert_proof_artifact`, make every conflict refresh produce a distinct version even when two
updates occur in the same wall-clock second:

```rust
updated_at = CASE
    WHEN excluded.updated_at <= proof_artifacts.updated_at
        THEN proof_artifacts.updated_at + 1
    ELSE excluded.updated_at
END
```

This replaces `updated_at = excluded.updated_at` in the existing conflict clause. It ensures the
conditional delete below cannot remove a row concurrently refreshed by the runtime observer.

Add beside `remove_proof_artifact`:

```rust
pub async fn remove_proof_artifact_if_unchanged(
    &self,
    network_pair: &str,
    proof_ref: &str,
    expected_updated_at: i64,
) -> Result<bool> {
    let conn = self.connection().await?;
    let network_pair = network_pair.to_string();
    let proof_ref = proof_ref.to_string();
    let removed = conn
        .call(move |conn| {
            Ok(conn.execute(
                r"
                DELETE FROM proof_artifacts
                WHERE network_pair = ?1 AND proof_ref = ?2 AND updated_at = ?3
                ",
                params![network_pair, proof_ref, expected_updated_at],
            )?)
        })
        .await
        .context("failed to conditionally remove proof artifact")?;
    Ok(removed == 1)
}
```

- [ ] **Step 5: Run focused and full runtime tests**

Run:

```bash
cargo test -p raiko2-runtime expired_proof_artifacts -- --nocapture
cargo test -p raiko2-runtime conditionally_removes_unchanged -- --nocapture
cargo test -p raiko2-runtime
```

Expected: all commands pass.

- [ ] **Step 6: Commit the storage primitives**

```bash
git add crates/runtime/src/lib.rs
git commit -m "feat(runtime): add artifact expiry queries"
```

---

### Task 2: Add independent artifact TTL configuration

**Files:**
- Modify: `bin/raiko2/src/config/runtime.rs`
- Modify: `bin/raiko2/src/config/mod.rs`

**Interfaces:**
- Consumes: existing `RuntimeConfig` Serde/default behavior.
- Produces: `RuntimeConfig::proof_artifact_ttl_secs: u64` with a default of `604_800`.

- [ ] **Step 1: Write failing configuration tests**

Replace the current inactive-TTL default test and add explicit disablement coverage:

```rust
#[test]
fn test_runtime_config_defaults_cleanup_ttls() {
    let config = RuntimeConfig::default();
    assert_eq!(config.inactive_ttl_secs, 7_200);
    assert_eq!(config.proof_artifact_ttl_secs, 604_800);
}

#[test]
fn test_runtime_config_accepts_disabled_artifact_cleanup() {
    let config: RuntimeConfig = toml::from_str(
        r#"
        root = "./data/runtime"
        proof_artifact_ttl_secs = 0
        "#,
    )
    .expect("deserialize runtime config");
    assert_eq!(config.proof_artifact_ttl_secs, 0);
    assert_eq!(config.inactive_ttl_secs, 7_200);
}
```

- [ ] **Step 2: Run the config tests and verify failure**

Run: `cargo test -p raiko2 runtime_config -- --nocapture`

Expected: compilation fails because `proof_artifact_ttl_secs` is missing.

- [ ] **Step 3: Implement the setting and default**

Update `RuntimeConfig` and its default:

```rust
pub struct RuntimeConfig {
    pub root: PathBuf,
    #[serde(default = "default_inactive_ttl_secs")]
    pub inactive_ttl_secs: u64,
    #[serde(default = "default_proof_artifact_ttl_secs")]
    pub proof_artifact_ttl_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("./data/runtime"),
            inactive_ttl_secs: default_inactive_ttl_secs(),
            proof_artifact_ttl_secs: default_proof_artifact_ttl_secs(),
        }
    }
}

const fn default_proof_artifact_ttl_secs() -> u64 {
    604_800
}
```

- [ ] **Step 4: Run config tests**

Run: `cargo test -p raiko2 runtime_config -- --nocapture`

Expected: both runtime configuration tests pass.

- [ ] **Step 5: Commit the configuration contract**

```bash
git add bin/raiko2/src/config/runtime.rs bin/raiko2/src/config/mod.rs
git commit -m "feat(config): add proof artifact TTL"
```

---

### Task 3: Centralize live-root artifact references

**Files:**
- Modify: `bin/raiko2/src/server/task_metadata.rs`
- Test: `bin/raiko2/src/server/task_metadata.rs`

**Interfaces:**
- Consumes: `proposal_proof_artifact_refs`, `root_proof_artifact_refs`, and `TaskMetadata::aggregate_input_artifacts`.
- Produces: `referenced_proof_artifact_refs(metadata, pipeline_key) -> BTreeSet<String>`.

- [ ] **Step 1: Write a failing all-reference-kinds test**

Add this test to `task_metadata.rs`:

```rust
#[test]
fn referenced_artifact_refs_include_canonical_legacy_and_external_inputs() {
    let proposal_request = ProposalTaskRequest {
        proposal_id: 7,
        l2_block_range: None,
        l1_inclusion_block_number: 11,
        last_anchor_block_number: 6,
        checkpoint: None,
        blob_proof_type: None,
        prover: None,
        graffiti: None,
        prover_config: raiko2_engine::ProverTaskConfig::default(),
    };
    let aggregate_request = AggregationTaskRequest {
        request_id: "aggregate-request".to_string(),
        proposal_ids: vec![7],
        prover_config: raiko2_engine::ProverTaskConfig::default(),
    };
    let metadata = TaskMetadata {
        network_pair: "taiko_dev/ethereum".to_string(),
        network: "taiko_dev".to_string(),
        l1_network: "ethereum".to_string(),
        proof_type: ProofType::Risc0,
        requested_proof_type: None,
        prover_type: None,
        execution_mode: None,
        aggregate_requested: true,
        proposals: vec![ProposalTask {
            proposal_id: 7,
            checkpoint: None,
            l1_inclusion_block_number: 11,
            l2_block_numbers: vec![7],
            last_anchor_block_number: 6,
            task_id: "legacy-proposal-ref".to_string(),
            request: Some(proposal_request.clone()),
        }],
        aggregate_task_id: Some("legacy-aggregate-ref".to_string()),
        aggregate_request: Some(aggregate_request.clone()),
        aggregate_input_artifacts: vec![AggregateInputProofArtifact {
            proof_ref: "external-input-ref".to_string(),
            proof_path: "cache/proofs/external.json".to_string(),
        }],
        runtime: RuntimeMetadata::default(),
    };

    let refs = referenced_proof_artifact_refs(&metadata, PipelineKey::ShastaRisc0);
    assert_eq!(
        refs,
        BTreeSet::from([
            proposal_task_ref(PipelineKey::ShastaRisc0, &proposal_request),
            "legacy-proposal-ref".to_string(),
            aggregate_task_ref(PipelineKey::ShastaRisc0, &aggregate_request),
            "legacy-aggregate-ref".to_string(),
            "external-input-ref".to_string(),
        ])
    );
}
```

- [ ] **Step 2: Run the test and verify failure**

Run: `cargo test -p raiko2 referenced_artifact_refs -- --nocapture`

Expected: compilation fails because `referenced_proof_artifact_refs` does not exist.

- [ ] **Step 3: Implement the reference helper**

Import `BTreeSet` and add after `root_proof_artifact_refs`:

```rust
pub(crate) fn referenced_proof_artifact_refs(
    metadata: &TaskMetadata,
    pipeline_key: PipelineKey,
) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for proposal in &metadata.proposals {
        refs.extend(proposal_proof_artifact_refs(pipeline_key, proposal));
    }
    if let Some(root_refs) = root_proof_artifact_refs(metadata, pipeline_key) {
        refs.extend(root_refs.refs);
    }
    refs.extend(
        metadata
            .aggregate_input_artifacts
            .iter()
            .map(|artifact| artifact.proof_ref.clone()),
    );
    refs
}
```

- [ ] **Step 4: Run task metadata tests**

Run:

```bash
cargo test -p raiko2 referenced_artifact_refs -- --nocapture
cargo test -p raiko2 task_metadata -- --nocapture
```

Expected: all matching tests pass.

- [ ] **Step 5: Commit the reference policy**

```bash
git add bin/raiko2/src/server/task_metadata.rs
git commit -m "refactor(server): centralize artifact references"
```

---

### Task 4: Implement artifact cleanup and maintenance wiring

**Files:**
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Interfaces:**
- Consumes: Tasks 1-3 APIs, `RuntimeManager::list_tasks`, Tokio filesystem APIs, and queue maintenance interval.
- Produces: `ProofArtifactCleanupStats`; `run_proof_artifact_cleanup_pass(runtime, guard, now_ts, ttl_secs, cursor)`; independent root/artifact maintenance execution; `AppState::artifact_cleanup_guard`.

- [ ] **Step 1: Add failing cleanup behavior tests**

Add test imports for `ProofArtifactCursor`, `ProofArtifactRegistration`, `ProofArtifactRecord`, `RwLock`, and `Path`. Add this helper:

```rust
async fn register_artifact(
    runtime: &RuntimeManager,
    proof_ref: &str,
    file: bool,
) -> Result<ProofArtifactRecord> {
    let proof_path = runtime.proof_artifact_path("taiko_dev/ethereum", proof_ref);
    if file {
        tokio::fs::create_dir_all(proof_path.parent().context("artifact parent")?).await?;
        tokio::fs::write(&proof_path, b"{}").await?;
    }
    runtime
        .upsert_proof_artifact(ProofArtifactRegistration {
            network_pair: "taiko_dev/ethereum".to_string(),
            proof_ref: proof_ref.to_string(),
            pipeline_key: PipelineKey::ShastaRisc0,
            route: "risc0/local".parse().expect("parse route"),
            proof_path: proof_path.display().to_string(),
        })
        .await?;
    runtime
        .get_proof_artifact("taiko_dev/ethereum", proof_ref)
        .await?
        .context("registered artifact")
}
```

Add these tests, using `now_ts = artifact.updated_at + 10` and `ttl_secs = 10` so no sleeps are needed:

```rust
#[tokio::test]
async fn artifact_cleanup_removes_expired_row_and_file() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-expired"))?);
    let artifact = register_artifact(runtime.as_ref(), "expired", true).await?;
    let mut cursor = None;
    let stats = run_proof_artifact_cleanup_pass(
        runtime.clone(),
        Arc::new(RwLock::new(())),
        artifact.updated_at + 10,
        10,
        &mut cursor,
    )
    .await?;
    assert_eq!(stats.removed_rows, 1);
    assert_eq!(stats.files_removed, 1);
    assert!(runtime.get_proof_artifact(&artifact.network_pair, &artifact.proof_ref).await?.is_none());
    assert!(!tokio::fs::try_exists(&artifact.proof_path).await?);
    Ok(())
}

#[tokio::test]
async fn artifact_cleanup_retains_nonterminal_references() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-active"))?);
    let proposal_ref = encoded_proposal_task_id(44)?;
    let artifact = register_artifact(runtime.as_ref(), &proposal_ref, true).await?;
    register_runtime_task(
        runtime.as_ref(),
        "active-root",
        &proposal_ref,
        RunnerStatus::Running,
        now_ts(),
    )
    .await?;
    let mut cursor = None;
    let stats = run_proof_artifact_cleanup_pass(
        runtime.clone(),
        Arc::new(RwLock::new(())),
        artifact.updated_at + 10,
        10,
        &mut cursor,
    )
    .await?;
    assert_eq!(stats.retained_active, 1);
    assert!(runtime.get_proof_artifact(&artifact.network_pair, &artifact.proof_ref).await?.is_some());
    assert!(tokio::fs::try_exists(&artifact.proof_path).await?);
    Ok(())
}

#[tokio::test]
async fn artifact_cleanup_does_not_retain_terminal_references() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-terminal"))?);
    let proposal_ref = encoded_proposal_task_id(45)?;
    let artifact = register_artifact(runtime.as_ref(), &proposal_ref, true).await?;
    register_runtime_task(
        runtime.as_ref(),
        "terminal-root",
        &proposal_ref,
        RunnerStatus::Completed,
        now_ts(),
    )
    .await?;
    let mut cursor = None;
    let stats = run_proof_artifact_cleanup_pass(
        runtime.clone(),
        Arc::new(RwLock::new(())),
        artifact.updated_at + 10,
        10,
        &mut cursor,
    )
    .await?;
    assert_eq!(stats.removed_rows, 1);
    assert_eq!(stats.files_removed, 1);
    Ok(())
}

#[tokio::test]
async fn artifact_cleanup_removes_missing_file_rows_and_honors_zero() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-missing"))?);
    let artifact = register_artifact(runtime.as_ref(), "missing", false).await?;
    let guard = Arc::new(RwLock::new(()));
    let mut cursor = None;
    assert_eq!(
        run_proof_artifact_cleanup_pass(
            runtime.clone(),
            guard.clone(),
            artifact.updated_at + 10,
            0,
            &mut cursor,
        )
        .await?,
        ProofArtifactCleanupStats::default()
    );
    let stats = run_proof_artifact_cleanup_pass(
        runtime.clone(),
        guard,
        artifact.updated_at + 10,
        10,
        &mut cursor,
    )
    .await?;
    assert_eq!(stats.removed_rows, 1);
    assert_eq!(stats.files_missing, 1);
    Ok(())
}

#[tokio::test]
async fn artifact_cleanup_removes_row_when_file_delete_fails() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-file-failure"))?);
    let artifact = register_artifact(runtime.as_ref(), "delete-fails", false).await?;
    tokio::fs::create_dir_all(&artifact.proof_path).await?;
    let mut cursor = None;
    let stats = run_proof_artifact_cleanup_pass(
        runtime.clone(),
        Arc::new(RwLock::new(())),
        artifact.updated_at + 10,
        10,
        &mut cursor,
    )
    .await?;
    assert_eq!(stats.removed_rows, 1);
    assert_eq!(stats.file_delete_failures, 1);
    assert!(runtime.get_proof_artifact(&artifact.network_pair, &artifact.proof_ref).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn artifact_cleanup_fails_closed_on_invalid_live_metadata() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-invalid-metadata"))?);
    let artifact = register_artifact(runtime.as_ref(), "retained", true).await?;
    runtime
        .register_task(TaskRegistration {
            task_id: "invalid-live-root".to_string(),
            pipeline_key: Some(PipelineKey::ShastaRisc0),
            route: "risc0/local".parse().expect("parse route"),
            task_kind: "hoodi_batch".to_string(),
            proposal_id: None,
            proof_ids: Vec::new(),
            metadata: serde_json::json!({"invalid": true}),
            request_fingerprint: None,
        })
        .await?;
    let mut cursor = None;
    let err = run_proof_artifact_cleanup_pass(
        runtime.clone(),
        Arc::new(RwLock::new(())),
        artifact.updated_at + 10,
        10,
        &mut cursor,
    )
    .await
    .expect_err("invalid live metadata must fail closed");
    assert!(err.to_string().contains("invalid-live-root"));
    assert!(runtime.get_proof_artifact(&artifact.network_pair, &artifact.proof_ref).await?.is_some());
    Ok(())
}
```

- [ ] **Step 2: Run cleanup tests and verify failure**

Run: `cargo test -p raiko2 artifact_cleanup -- --nocapture`

Expected: compilation fails because the cleanup stats/pass and app-state guard do not exist.

- [ ] **Step 3: Implement artifact identity, stats, and live refs**

Add these definitions in `task_cleanup.rs` and import `referenced_proof_artifact_refs`:

```rust
const PROOF_ARTIFACT_CLEANUP_BATCH_SIZE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProofArtifactIdentity {
    network_pair: String,
    proof_ref: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProofArtifactCleanupStats {
    pub scanned: usize,
    pub removed_rows: usize,
    pub retained_active: usize,
    pub retained_changed: usize,
    pub files_removed: usize,
    pub files_missing: usize,
    pub file_delete_failures: usize,
    pub record_delete_failures: usize,
}

impl ProofArtifactCleanupStats {
    const fn is_idle(self) -> bool {
        self.scanned == 0
            && self.removed_rows == 0
            && self.retained_active == 0
            && self.retained_changed == 0
            && self.files_removed == 0
            && self.files_missing == 0
            && self.file_delete_failures == 0
            && self.record_delete_failures == 0
    }
}

async fn live_proof_artifact_refs(
    runtime: &RuntimeManager,
) -> Result<HashSet<ProofArtifactIdentity>> {
    let mut refs = HashSet::new();
    for record in runtime.list_tasks().await? {
        if !matches!(record.runner_status, RunnerStatus::Allocated | RunnerStatus::Running) {
            continue;
        }
        let metadata: TaskMetadata = serde_json::from_value(record.metadata.clone())
            .with_context(|| format!("failed to parse non-terminal task metadata {}", record.task_id))?;
        refs.extend(
            referenced_proof_artifact_refs(&metadata, record.pipeline_key)
                .into_iter()
                .map(|proof_ref| ProofArtifactIdentity {
                    network_pair: metadata.network_pair.clone(),
                    proof_ref,
                }),
        );
    }
    Ok(refs)
}
```

- [ ] **Step 4: Implement row-first cleanup and structured logging**

Add the cleanup pass and logger:

```rust
pub(crate) async fn run_proof_artifact_cleanup_pass(
    runtime: Arc<RuntimeManager>,
    artifact_cleanup_guard: Arc<RwLock<()>>,
    now_ts: i64,
    ttl_secs: u64,
    cursor: &mut Option<ProofArtifactCursor>,
) -> Result<ProofArtifactCleanupStats> {
    if ttl_secs == 0 {
        return Ok(ProofArtifactCleanupStats::default());
    }
    let _guard = artifact_cleanup_guard.write().await;
    let artifacts = runtime
        .list_expired_proof_artifacts(
            now_ts,
            ttl_secs,
            cursor.as_ref(),
            PROOF_ARTIFACT_CLEANUP_BATCH_SIZE,
        )
        .await?;
    *cursor = artifacts.last().map(ProofArtifactCursor::from);
    if artifacts.is_empty() {
        return Ok(ProofArtifactCleanupStats::default());
    }
    let live_refs = live_proof_artifact_refs(runtime.as_ref()).await?;
    let mut stats = ProofArtifactCleanupStats {
        scanned: artifacts.len(),
        ..ProofArtifactCleanupStats::default()
    };

    for artifact in artifacts {
        let identity = ProofArtifactIdentity {
            network_pair: artifact.network_pair.clone(),
            proof_ref: artifact.proof_ref.clone(),
        };
        if live_refs.contains(&identity) {
            stats.retained_active += 1;
            continue;
        }
        match runtime
            .remove_proof_artifact_if_unchanged(
                &artifact.network_pair,
                &artifact.proof_ref,
                artifact.updated_at,
            )
            .await
        {
            Ok(false) => {
                stats.retained_changed += 1;
                continue;
            }
            Ok(true) => stats.removed_rows += 1,
            Err(err) => {
                stats.record_delete_failures += 1;
                warn!(
                    network_pair = artifact.network_pair,
                    proof_ref = artifact.proof_ref,
                    error = %err,
                    "failed to remove expired proof artifact record"
                );
                continue;
            }
        }
        match fs::remove_file(&artifact.proof_path).await {
            Ok(()) => stats.files_removed += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => stats.files_missing += 1,
            Err(err) => {
                stats.file_delete_failures += 1;
                warn!(
                    network_pair = artifact.network_pair,
                    proof_ref = artifact.proof_ref,
                    proof_path = artifact.proof_path,
                    error = %err,
                    "failed to remove expired proof artifact file after removing cache record"
                );
            }
        }
    }
    Ok(stats)
}

fn log_proof_artifact_cleanup_stats(result: Result<ProofArtifactCleanupStats>) {
    match result {
        Ok(stats) if !stats.is_idle() => info!(
            scanned = stats.scanned,
            removed_rows = stats.removed_rows,
            retained_active = stats.retained_active,
            retained_changed = stats.retained_changed,
            files_removed = stats.files_removed,
            files_missing = stats.files_missing,
            file_delete_failures = stats.file_delete_failures,
            record_delete_failures = stats.record_delete_failures,
            "proof artifact cleanup tick completed"
        ),
        Ok(_) => {}
        Err(err) => warn!(error = %err, "proof artifact cleanup tick failed"),
    }
}
```

- [ ] **Step 5: Add the shared guard to application state**

Import `tokio::sync::RwLock`, add the field, initialize it in `from_parts`, and pass it to cleanup startup:

```rust
pub struct AppState {
    pub config: Arc<Config>,
    pub pipelines: Arc<dyn PipelineFactory>,
    pub runtime: Arc<RuntimeManager>,
    pub zk_any_sampler: Arc<Mutex<ZkAnySampler>>,
    pub(crate) artifact_cleanup_guard: Arc<RwLock<()>>,
    pub(crate) acl_rate_limiter: Arc<AclRateLimiter>,
}
```

```rust
Self {
    config,
    pipelines,
    runtime,
    zk_any_sampler,
    artifact_cleanup_guard: Arc::new(RwLock::new(())),
    acl_rate_limiter: Arc::new(AclRateLimiter::default()),
}
```

```rust
spawn_runtime_cleanup_loop(
    Arc::clone(&state.config),
    Arc::clone(&state.runtime),
    Arc::clone(&state.pipelines),
    Arc::clone(&state.artifact_cleanup_guard),
);
```

- [ ] **Step 6: Make the maintenance loop run both cleanup policies independently**

Change `spawn_runtime_cleanup_loop` to accept `Arc<RwLock<()>>`, return only when both TTLs are zero, keep independent `artifact_cursor`, `orphan_cursor`, and `terminal_cursor`, and run these two guarded blocks both before interval creation and on every tick:

```rust
if config.runtime.inactive_ttl_secs != 0 {
    log_runtime_cleanup_stats(
        run_runtime_cleanup_pass(
            Arc::clone(&runtime),
            Arc::clone(&pipelines),
            config.runtime.inactive_ttl_secs,
            &mut orphan_cursor,
            &mut terminal_cursor,
        )
        .await,
    );
}
if config.runtime.proof_artifact_ttl_secs != 0 {
    log_proof_artifact_cleanup_stats(
        run_proof_artifact_cleanup_pass(
            Arc::clone(&runtime),
            Arc::clone(&artifact_cleanup_guard),
            now_ts(),
            config.runtime.proof_artifact_ttl_secs,
            &mut artifact_cursor,
        )
        .await,
    );
}
```

Do not return early merely because `inactive_ttl_secs == 0`.

- [ ] **Step 7: Run cleanup and existing root-cleanup tests**

Run:

```bash
cargo test -p raiko2 artifact_cleanup -- --nocapture
cargo test -p raiko2 runtime_cleanup -- --nocapture
```

Expected: all artifact and root cleanup tests pass; existing root TTL behavior is unchanged.

- [ ] **Step 8: Commit cleanup mechanics**

```bash
git add bin/raiko2/src/server/task_cleanup.rs bin/raiko2/src/server/state/mod.rs
git commit -m "feat(server): clean expired proof artifacts"
```

---

### Task 5: Order artifact-consuming submissions against cleanup

**Files:**
- Modify: `bin/raiko2/src/server/handlers/proof_api/v3.rs`
- Modify: `bin/raiko2/src/server/handlers/proof_api/v4.rs`
- Modify: `bin/raiko2/src/server/task_cleanup.rs`
- Test: `bin/raiko2/src/server/task_cleanup.rs`

**Interfaces:**
- Consumes: `AppState::artifact_cleanup_guard` and Task 4 cleanup pass.
- Produces: shared/read guard coverage from artifact lookup through runtime root registration for v3 batch, v3 external aggregation, v4 submission, replacement, and recovery flows.

- [ ] **Step 1: Add a cleanup-guard serialization regression test**

Add this test to `task_cleanup.rs`:

```rust
#[tokio::test]
async fn artifact_cleanup_waits_for_submission_read_guard() -> Result<()> {
    let runtime = Arc::new(RuntimeManager::new(unique_runtime_root("artifact-guard"))?);
    let artifact = register_artifact(runtime.as_ref(), "guarded", true).await?;
    let guard = Arc::new(RwLock::new(()));
    let read_guard = guard.read().await;
    let mut cleanup = tokio::spawn({
        let runtime = runtime.clone();
        let guard = guard.clone();
        async move {
            let mut cursor = None;
            run_proof_artifact_cleanup_pass(
                runtime,
                guard,
                artifact.updated_at + 10,
                10,
                &mut cursor,
            )
            .await
        }
    });

    assert!(tokio::time::timeout(Duration::from_millis(25), &mut cleanup)
        .await
        .is_err());
    drop(read_guard);
    let stats = cleanup.await??;
    assert_eq!(stats.removed_rows, 1);
    Ok(())
}
```

- [ ] **Step 2: Run the guard test**

Run: `cargo test -p raiko2 artifact_cleanup_waits_for_submission_read_guard -- --nocapture`

Expected: the primitive guard test passes, proving the cleanup side takes the exclusive guard.
Continue to the handler changes; the existing submission/recovery tests then protect their behavior
while code review verifies the three shared-guard acquisition points.

- [ ] **Step 3: Guard v3 batch submission**

In `request_batch_shasta_proof_inner`, acquire the guard immediately before `build_submission_plan` and keep the binding in scope through the registration match:

```rust
let _artifact_guard = state.artifact_cleanup_guard.read().await;
let plan =
    build_submission_plan(state.runtime.as_ref(), &submission, &request_fingerprint).await?;
```

The guard is intentionally held through `register_batch_task` and the existing created/existing handler call.

- [ ] **Step 4: Guard v3 external aggregate submission**

In `request_aggregation_proof_inner`, acquire the guard before external input artifacts are created and keep it through the registration match:

```rust
let _artifact_guard = state.artifact_cleanup_guard.read().await;
let submission = build_external_aggregate_submission(&state, req).await?;
```

- [ ] **Step 5: Guard all v4 submission, replacement, and recovery branches**

At the start of `submit_submission`, before the existing-task lookup, add:

```rust
let _artifact_guard = state.artifact_cleanup_guard.read().await;
```

Because replacement and recovery are called from `submit_submission`, this one scope covers cached lookup, terminal-root reset, replacement registration, new root registration, and enqueue.

- [ ] **Step 6: Run submission, aggregation, and guard tests**

Run:

```bash
cargo test -p raiko2 artifact_cleanup_waits_for_submission_read_guard -- --nocapture
cargo test -p raiko2 aggregate_plan_uses_cached_artifact_refs -- --nocapture
cargo test -p raiko2 aggregate_recovery -- --nocapture
cargo test -p raiko2 external_aggregate_inputs_are_persisted -- --nocapture
```

Expected: all commands pass.

- [ ] **Step 7: Commit the concurrency boundary**

```bash
git add bin/raiko2/src/server/handlers/proof_api/v3.rs \
  bin/raiko2/src/server/handlers/proof_api/v4.rs \
  bin/raiko2/src/server/task_cleanup.rs
git commit -m "fix(server): retain artifacts during aggregation"
```

---

### Task 6: Document behavior and perform release-quality verification

**Files:**
- Modify: `config.example.toml`
- Modify: `docs/API.md`
- Verify: all files changed by Tasks 1-5

**Interfaces:**
- Consumes: the final configuration and runtime behavior.
- Produces: operator documentation and evidence suitable for the pull request description.

- [ ] **Step 1: Update the example configuration**

Make the `[runtime]` block read:

```toml
[runtime]
root = "./data/runtime"
# Terminal root tasks expire independently after two hours of inactivity. Set to 0 to disable.
inactive_ttl_secs = 7200
# Completed proof artifact rows/files expire after seven days unless a non-terminal root still
# references them. Set to 0 to disable automatic artifact cleanup.
proof_artifact_ttl_secs = 604800
```

- [ ] **Step 2: Update API runtime semantics**

Replace the existing two cleanup bullets/paragraphs in `docs/API.md`'s `Runtime Semantics` section with:

```markdown
- Terminal root tasks may be automatically removed from `runtime.sqlite` and `tasks/...` after
  `runtime.inactive_ttl_secs` of inactivity. Active root tasks are never removed by root TTL
  cleanup. Set this value to `0` to disable automatic root-task cleanup.
- Completed proof artifacts are stored independently under `cache/proofs/...` and indexed by
  stable proof refs so aggregation can reuse them after engine task cleanup or process restart.
  Artifact rows and files expire after `runtime.proof_artifact_ttl_secs` (default `604800`, seven
  days). Artifacts referenced by `allocated` or `running` roots are retained, missing files do not
  block row cleanup, and other file deletion errors are logged after the row is removed. Set this
  value to `0` to disable automatic artifact cleanup without disabling root-task cleanup.
```

- [ ] **Step 3: Check formatting and documentation diffs**

Run:

```bash
cargo fmt --all -- --check
git diff --check
git diff -- config.example.toml docs/API.md
```

Expected: both checks pass; the diff states the seven-day default, zero disablement, independence, active retention, and missing-file semantics.

- [ ] **Step 4: Run focused test suites**

Run:

```bash
cargo test -p raiko2-runtime
cargo test -p raiko2 task_cleanup -- --nocapture
cargo test -p raiko2 config -- --nocapture
cargo test -p raiko2 task_metadata -- --nocapture
```

Expected: all commands pass.

- [ ] **Step 5: Run repository-policy checks**

Run:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test -p raiko2-queue -p raiko2-runtime
cargo test -p raiko2
```

Expected: all commands pass with no warnings. If an environment-only dependency prevents a command from running, preserve the exact command/output for the PR and run every unaffected check.

- [ ] **Step 6: Review the complete diff against issue #154**

Run:

```bash
git diff origin/main...HEAD --check
git status --short
git log --oneline origin/main..HEAD
```

Expected: only the design/plan docs and issue-related implementation/docs are present; no generated ELF files or unrelated changes appear.

- [ ] **Step 7: Commit documentation and any formatting changes**

```bash
git add config.example.toml docs/API.md
git commit -m "docs: document proof artifact retention"
```

- [ ] **Step 8: Push and open the pull request**

```bash
git push -u origin codex/issue-154-proof-artifact-ttl-cleanup
gh pr create \
  --repo taikoxyz/raiko2 \
  --base main \
  --head codex/issue-154-proof-artifact-ttl-cleanup \
  --title "feat(runtime): clean expired proof artifacts" \
  --body "$(printf '%s\n' \
    'Closes #154' \
    '' \
    '## Summary' \
    '- add independent seven-day proof artifact retention with 0 as the opt-out' \
    '- retain artifacts referenced by non-terminal roots and serialize cleanup against aggregation reuse' \
    '- delete expired SQLite rows first, then best-effort remove files with compact structured counters' \
    '' \
    '## Verification' \
    '- cargo fmt --all' \
    '- cargo clippy --workspace -- -D warnings' \
    '- cargo test -p raiko2-runtime' \
    '- cargo test -p raiko2-queue -p raiko2-runtime' \
    '- cargo test -p raiko2')"
```

Before running the command, remove any verification line whose command did not pass and replace it
with the exact failure/blocker and the unaffected checks that passed. Never claim an unrun check.
