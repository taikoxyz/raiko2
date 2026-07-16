#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod artifact_store;

pub use artifact_store::{
    FilesystemProofArtifactStore, GcsProofArtifactStore, ProofArtifactKey, ProofArtifactObject,
    ProofArtifactPrefix, ProofArtifactPutResult, ProofArtifactStore, validate_environment_id,
};

use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fs as stdfs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::OnceCell;

/// Runtime root managed by the host process.
#[derive(Debug, Clone)]
pub struct RuntimeManager {
    root: PathBuf,
    db_path: PathBuf,
    artifact_store: Arc<dyn ProofArtifactStore>,
    conn: OnceCell<tokio_rusqlite::Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredTaskCursor {
    pub updated_at: i64,
    pub task_id: String,
}

#[derive(Debug, Clone)]
pub enum TaskRegistrationOutcome {
    Created(RuntimeTaskRecord),
    Existing(RuntimeTaskRecord),
}

#[derive(Debug, Clone)]
pub struct ProofArtifactRegistration {
    pub network_pair: String,
    pub proof_ref: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub proof_uri: String,
    pub content_hash: String,
    pub generation: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofArtifactRecord {
    pub environment_id: String,
    pub network_pair: String,
    pub proof_ref: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub proof_uri: String,
    pub content_hash: String,
    pub generation: Option<i64>,
    pub invalidated_at: Option<i64>,
    pub updated_at: i64,
}

impl RuntimeManager {
    /// # Errors
    ///
    /// Returns an error if the runtime layout cannot be created.
    pub fn new(root: PathBuf) -> Result<Self> {
        let artifact_store = Arc::new(FilesystemProofArtifactStore::new(
            "local".to_string(),
            root.join("cache").join("proofs"),
        )?);
        Self::new_with_artifact_store(root, artifact_store)
    }

    /// # Errors
    ///
    /// Returns an error if the runtime layout cannot be created.
    pub fn new_with_artifact_store(
        root: PathBuf,
        artifact_store: Arc<dyn ProofArtifactStore>,
    ) -> Result<Self> {
        let manager = Self {
            db_path: root.join("state").join("runtime.sqlite"),
            root,
            artifact_store,
            conn: OnceCell::new(),
        };
        manager.ensure_layout()?;
        Ok(manager)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn environment_id(&self) -> &str {
        self.artifact_store.environment_id()
    }

    #[must_use]
    pub fn proof_artifact_uri(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> String {
        self.artifact_store.proof_uri(&ProofArtifactKey {
            network_pair: network_pair.to_string(),
            pipeline_key,
            route,
            proof_ref: proof_ref.to_string(),
        })
    }

    /// # Errors
    ///
    /// Returns an error if the artifact cannot be conditionally published.
    pub async fn publish_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        bytes: &[u8],
    ) -> Result<ProofArtifactPutResult> {
        self.artifact_store
            .put_if_absent(
                &ProofArtifactKey {
                    network_pair: network_pair.to_string(),
                    pipeline_key,
                    route,
                    proof_ref: proof_ref.to_string(),
                },
                bytes,
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the artifact cannot be read.
    pub async fn read_proof_artifact_bytes(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactObject>> {
        self.artifact_store
            .get(&ProofArtifactKey {
                network_pair: network_pair.to_string(),
                pipeline_key,
                route,
                proof_ref: proof_ref.to_string(),
            })
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the bounded artifact prefix cannot be read.
    pub async fn read_proof_artifact_prefix(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        max_bytes: usize,
    ) -> Result<Option<ProofArtifactPrefix>> {
        self.artifact_store
            .get_prefix(
                &ProofArtifactKey {
                    network_pair: network_pair.to_string(),
                    pipeline_key,
                    route,
                    proof_ref: proof_ref.to_string(),
                },
                max_bytes,
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the artifact cannot be deleted.
    pub async fn delete_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        generation: Option<i64>,
        expected_content_hash: &str,
    ) -> Result<()> {
        self.artifact_store
            .delete(
                &ProofArtifactKey {
                    network_pair: network_pair.to_string(),
                    pipeline_key,
                    route,
                    proof_ref: proof_ref.to_string(),
                },
                generation,
                expected_content_hash,
            )
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the runtime layout cannot be created.
    pub fn ensure_layout(&self) -> Result<()> {
        for dir in [
            self.root.join("state"),
            self.root.join("tasks"),
            self.root.join("images"),
            self.root.join("cache"),
            self.root.join("cache").join("proofs"),
            self.root.join("tmp"),
        ] {
            stdfs::create_dir_all(&dir)
                .with_context(|| format!("failed to create runtime directory {}", dir.display()))?;
        }
        Ok(())
    }

    async fn connection(&self) -> Result<tokio_rusqlite::Connection> {
        let db_path = self.db_path.clone();
        let environment_id = self.environment_id().to_string();
        let conn = self
            .conn
            .get_or_try_init(|| async move {
                let conn = tokio_rusqlite::Connection::open(db_path)
                    .await
                    .context("failed to open runtime sqlite database")?;
                conn.call(move |conn| {
                    conn.execute_batch(
                        r"
                        PRAGMA journal_mode = WAL;
                        CREATE TABLE IF NOT EXISTS runtime_tasks (
                            task_id TEXT PRIMARY KEY,
                            pipeline_key TEXT NOT NULL,
                            route TEXT NOT NULL,
                            guest_system TEXT NOT NULL,
                            runner TEXT NOT NULL,
                            task_kind TEXT NOT NULL,
                            proposal_id INTEGER,
                            proof_ids_json TEXT,
                            runner_status TEXT NOT NULL,
                            task_dir TEXT NOT NULL,
                            image_ref TEXT,
                            provider_request_id TEXT,
                            remote_tx_hash TEXT,
                            proof_uri TEXT,
                            error TEXT,
                            metadata_json TEXT,
                            request_fingerprint TEXT,
                            updated_at INTEGER NOT NULL
                        );
                        ",
                    )?;
                    migrate_runtime_schema(conn, &environment_id)?;
                    Ok(())
                })
                .await
                .context("failed to initialize runtime sqlite schema")?;
                Ok::<_, anyhow::Error>(conn)
            })
            .await?;
        Ok(conn.clone())
    }

    #[must_use]
    pub fn task_dir(&self, pipeline_key: PipelineKey, task_id: &str) -> PathBuf {
        self.root
            .join("tasks")
            .join(pipeline_key.as_str())
            .join(task_id)
    }

    /// # Errors
    ///
    /// Returns an error if the task workspace or metadata cannot be created.
    pub async fn register_task(&self, registration: TaskRegistration) -> Result<RuntimeTaskRecord> {
        let record = self.build_task_record(&registration)?;
        self.write_task_workspace(&registration).await?;
        self.upsert_task(&record).await?;
        Ok(record)
    }

    /// # Errors
    ///
    /// Returns an error if the task cannot be inserted or loaded.
    pub async fn register_task_if_absent(
        &self,
        registration: TaskRegistration,
    ) -> Result<TaskRegistrationOutcome> {
        let request_fingerprint = registration
            .request_fingerprint
            .as_ref()
            .context("request_fingerprint is required for idempotent registration")?;
        let record = self.build_task_record(&registration)?;
        if self.insert_task_if_absent(&record).await? {
            if let Err(err) = self.write_task_workspace(&registration).await {
                let _ = self.delete_task_row(&record.task_id).await;
                let _ = remove_task_workspace(Path::new(&record.task_dir)).await;
                return Err(err);
            }
            return Ok(TaskRegistrationOutcome::Created(record));
        }

        let existing = match self.get_task(&record.task_id).await? {
            Some(existing) => existing,
            None => self
                .find_task_by_request_fingerprint(request_fingerprint)
                .await?
                .context("request fingerprint conflict without a matching runtime task")?,
        };
        Ok(TaskRegistrationOutcome::Existing(existing))
    }

    /// # Errors
    ///
    /// Returns an error if the task record cannot be stored.
    pub async fn upsert_task(&self, record: &RuntimeTaskRecord) -> Result<()> {
        let conn = self.connection().await?;
        let proof_ids_json =
            serde_json::to_string(&record.proof_ids).context("serialize proof_ids")?;
        let metadata_json =
            serde_json::to_string(&record.metadata).context("serialize runtime metadata")?;
        let record = record.clone();
        conn.call(move |conn| {
            conn.execute(
                r"
                INSERT INTO runtime_tasks (
                    task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                    proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                    remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                    updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                ON CONFLICT(task_id) DO UPDATE SET
                    pipeline_key = excluded.pipeline_key,
                    route = excluded.route,
                    guest_system = excluded.guest_system,
                    runner = excluded.runner,
                    task_kind = excluded.task_kind,
                    proposal_id = excluded.proposal_id,
                    proof_ids_json = excluded.proof_ids_json,
                    runner_status = excluded.runner_status,
                    task_dir = excluded.task_dir,
                    image_ref = excluded.image_ref,
                    provider_request_id = excluded.provider_request_id,
                    remote_tx_hash = excluded.remote_tx_hash,
                    proof_uri = excluded.proof_uri,
                    error = excluded.error,
                    metadata_json = excluded.metadata_json,
                    request_fingerprint = excluded.request_fingerprint,
                    updated_at = excluded.updated_at
                ",
                params![
                    record.task_id,
                    record.pipeline_key.as_str(),
                    record.route.to_string(),
                    record.route.guest_system.to_string(),
                    record.route.runner.to_string(),
                    record.task_kind,
                    record.proposal_id,
                    proof_ids_json,
                    record.runner_status.as_str(),
                    record.task_dir,
                    record.image_ref,
                    record.provider_request_id,
                    record.remote_tx_hash,
                    record.proof_uri,
                    record.error,
                    metadata_json,
                    record.request_fingerprint,
                    record.updated_at,
                ],
            )?;
            Ok(())
        })
        .await
        .context("failed to upsert runtime task")?;
        Ok(())
    }

    /// Updates only the durable task metadata without overwriting concurrent status changes.
    ///
    /// # Errors
    ///
    /// Returns an error if the task metadata cannot be serialized or stored.
    pub async fn update_task_metadata(
        &self,
        task_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<bool> {
        let conn = self.connection().await?;
        let task_id = task_id.to_string();
        let metadata_json =
            serde_json::to_string(metadata).context("serialize runtime metadata")?;
        let updated_at = now_ts();
        conn.call(move |conn| {
            Ok(conn.execute(
                "UPDATE runtime_tasks SET metadata_json = ?2, updated_at = ?3 WHERE task_id = ?1",
                params![task_id, metadata_json, updated_at],
            )?)
        })
        .await
        .context("failed to update runtime task metadata")
        .map(|updated| updated > 0)
    }

    /// # Errors
    ///
    /// Returns an error if the task record cannot be loaded.
    pub async fn get_task(&self, task_id: &str) -> Result<Option<RuntimeTaskRecord>> {
        let conn = self.connection().await?;
        let task_id = task_id.to_string();
        let row = conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r"
                    SELECT
                        task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                        proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                        remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                        updated_at
                    FROM runtime_tasks
                    WHERE task_id = ?1
                    ",
                )?;
                let mut rows = stmt.query(params![task_id])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(runtime_task_record_from_row(row)?))
            })
            .await
            .context("failed to query runtime task")?;
        Ok(row)
    }

    /// # Errors
    ///
    /// Returns an error if the task record cannot be loaded.
    pub async fn find_task_by_request_fingerprint(
        &self,
        request_fingerprint: &str,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let conn = self.connection().await?;
        let request_fingerprint = request_fingerprint.to_string();
        let row = conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r"
                    SELECT
                        task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                        proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                        remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                        updated_at
                    FROM runtime_tasks
                    WHERE request_fingerprint = ?1
                    ORDER BY updated_at DESC, task_id ASC
                    LIMIT 1
                    ",
                )?;
                let mut rows = stmt.query(params![request_fingerprint])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(runtime_task_record_from_row(row)?))
            })
            .await
            .context("failed to query runtime task by request fingerprint")?;
        Ok(row)
    }

    /// # Errors
    ///
    /// Returns an error if the task record cannot be loaded.
    pub async fn find_task_by_engine_task_id(
        &self,
        engine_task_id: &str,
    ) -> Result<Option<RuntimeTaskRecord>> {
        self.find_task_by_task_ref(engine_task_id).await
    }

    /// # Errors
    ///
    /// Returns an error if the task record cannot be loaded.
    pub async fn find_task_by_task_ref(&self, task_ref: &str) -> Result<Option<RuntimeTaskRecord>> {
        Ok(self
            .find_tasks_by_task_ref(task_ref)
            .await?
            .into_iter()
            .next())
    }

    /// # Errors
    ///
    /// Returns an error if the matching task records cannot be loaded.
    pub async fn find_tasks_by_engine_task_id(
        &self,
        engine_task_id: &str,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        self.find_tasks_by_task_ref(engine_task_id).await
    }

    /// # Errors
    ///
    /// Returns an error if the matching task records cannot be loaded.
    pub async fn find_tasks_by_task_ref(&self, task_ref: &str) -> Result<Vec<RuntimeTaskRecord>> {
        let conn = self.connection().await?;
        let task_ref = task_ref.to_string();
        let tasks = conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r"
                    SELECT
                        task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                        proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                        remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                        updated_at
                    FROM runtime_tasks
                    WHERE task_id = ?1
                        OR EXISTS (
                            SELECT 1
                            FROM json_each(runtime_tasks.proof_ids_json)
                            WHERE json_each.value = ?1
                        )
                        OR json_extract(metadata_json, '$.aggregate_task_id') = ?1
                    ORDER BY updated_at DESC, task_id ASC
                    ",
                )?;
                let mut rows = stmt.query(params![task_ref])?;
                let mut matches = Vec::new();
                while let Some(row) = rows.next()? {
                    matches.push(runtime_task_record_from_row(row)?);
                }
                Ok(matches)
            })
            .await
            .context("failed to query runtime task by task reference")?;
        Ok(tasks)
    }

    /// # Errors
    ///
    /// Returns an error if the task record cannot be updated.
    pub async fn sync_status(
        &self,
        task_id: &str,
        runner_status: RunnerStatus,
        error: Option<String>,
        proof_uri: Option<String>,
    ) -> Result<()> {
        let Some(mut record) = self.get_task(task_id).await? else {
            return Ok(());
        };
        record.runner_status = runner_status;
        record.error = error;
        if proof_uri.is_some() {
            record.proof_uri = proof_uri;
        }
        record.updated_at = now_ts();
        self.upsert_task(&record).await
    }

    /// # Errors
    ///
    /// Returns an error if the conditional runtime task cancellation cannot be persisted.
    pub async fn cancel_nonterminal_task(
        &self,
        task_id: &str,
        error: Option<String>,
    ) -> Result<bool> {
        self.cancel_nonterminal_task_matching(task_id, error, None)
            .await
    }

    /// # Errors
    ///
    /// Returns an error if the stale runtime task cancellation cannot be persisted.
    pub async fn cancel_nonterminal_task_if_stale(
        &self,
        task_id: &str,
        updated_at_or_before: i64,
        error: Option<String>,
    ) -> Result<bool> {
        self.cancel_nonterminal_task_matching(task_id, error, Some(updated_at_or_before))
            .await
    }

    async fn cancel_nonterminal_task_matching(
        &self,
        task_id: &str,
        error: Option<String>,
        updated_at_or_before: Option<i64>,
    ) -> Result<bool> {
        let conn = self.connection().await?;
        let task_id = task_id.to_string();
        let updated_at = now_ts();
        let runner_status = RunnerStatus::Cancelled.as_str();
        let updated = conn
            .call(move |conn| {
                if let Some(updated_at_or_before) = updated_at_or_before {
                    Ok(conn.execute(
                        r"
                        UPDATE runtime_tasks
                        SET runner_status = ?1,
                            error = ?2,
                            updated_at = ?3
                        WHERE task_id = ?4
                          AND runner_status IN ('allocated', 'running')
                          AND updated_at <= ?5
                        ",
                        params![
                            runner_status,
                            error,
                            updated_at,
                            task_id,
                            updated_at_or_before
                        ],
                    )?)
                } else {
                    Ok(conn.execute(
                        r"
                        UPDATE runtime_tasks
                        SET runner_status = ?1,
                            error = ?2,
                            updated_at = ?3
                        WHERE task_id = ?4
                          AND runner_status IN ('allocated', 'running')
                        ",
                        params![runner_status, error, updated_at, task_id],
                    )?)
                }
            })
            .await
            .context("failed cancel non-terminal runtime task")?;

        Ok(updated > 0)
    }

    /// # Errors
    ///
    /// Returns an error if the proof artifact record cannot be stored.
    pub async fn upsert_proof_artifact(
        &self,
        registration: ProofArtifactRegistration,
    ) -> Result<()> {
        let conn = self.connection().await?;
        let updated_at = now_ts();
        let environment_id = self.environment_id().to_string();
        conn.call(move |conn| {
            conn.execute(
                r"
                INSERT INTO proof_artifacts (
                    environment_id, network_pair, proof_ref, pipeline_key, route, proof_uri,
                    content_hash, generation, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(environment_id, network_pair, pipeline_key, route, proof_ref) DO UPDATE SET
                    proof_uri = excluded.proof_uri,
                    content_hash = excluded.content_hash,
                    generation = excluded.generation,
                    invalidated_at = CASE
                        WHEN proof_artifacts.content_hash = excluded.content_hash
                        THEN proof_artifacts.invalidated_at
                        ELSE NULL
                    END,
                    updated_at = excluded.updated_at
                ",
                params![
                    environment_id,
                    registration.network_pair,
                    registration.proof_ref,
                    registration.pipeline_key.as_str(),
                    registration.route.to_string(),
                    registration.proof_uri,
                    registration.content_hash,
                    registration.generation,
                    updated_at,
                ],
            )?;
            Ok(())
        })
        .await
        .context("failed to upsert proof artifact")?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the proof artifact record cannot be loaded.
    pub async fn get_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        let conn = self.connection().await?;
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let environment_id = self.environment_id().to_string();
        let artifact = conn
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        r"
                    SELECT environment_id, network_pair, proof_ref, pipeline_key, route, proof_uri,
                           content_hash, generation, invalidated_at, updated_at
                    FROM proof_artifacts
                    WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3
                      AND route = ?4 AND proof_ref = ?5
                      AND invalidated_at IS NULL
                    ",
                        params![environment_id, network_pair, pipeline_key, route, proof_ref],
                        proof_artifact_record_from_row,
                    )
                    .optional()?)
            })
            .await
            .context("failed to query proof artifact")?;
        Ok(artifact)
    }

    /// Loads a proof artifact record, including a locally tombstoned record.
    ///
    /// # Errors
    ///
    /// Returns an error if the proof artifact record cannot be loaded.
    pub async fn get_proof_artifact_including_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        let conn = self.connection().await?;
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let environment_id = self.environment_id().to_string();
        conn.call(move |conn| {
            Ok(conn
                .query_row(
                    r"
                    SELECT environment_id, network_pair, proof_ref, pipeline_key, route, proof_uri,
                           content_hash, generation, invalidated_at, updated_at
                    FROM proof_artifacts
                    WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3
                      AND route = ?4 AND proof_ref = ?5
                    ",
                    params![environment_id, network_pair, pipeline_key, route, proof_ref],
                    proof_artifact_record_from_row,
                )
                .optional()?)
        })
        .await
        .context("failed to query proof artifact including invalidated records")
    }

    /// # Errors
    ///
    /// Returns an error if proof artifact records cannot be listed.
    pub async fn list_proof_artifacts(&self) -> Result<Vec<ProofArtifactRecord>> {
        let conn = self.connection().await?;
        let environment_id = self.environment_id().to_string();
        let artifacts = conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r"
                    SELECT environment_id, network_pair, proof_ref, pipeline_key, route, proof_uri,
                           content_hash, generation, invalidated_at, updated_at
                    FROM proof_artifacts
                    WHERE environment_id = ?1
                    ORDER BY updated_at ASC, network_pair ASC, proof_ref ASC
                    ",
                )?;
                let mut rows = stmt.query(params![environment_id])?;
                let mut artifacts = Vec::new();
                while let Some(row) = rows.next()? {
                    artifacts.push(proof_artifact_record_from_row(row)?);
                }
                Ok(artifacts)
            })
            .await
            .context("failed to list proof artifacts")?;
        Ok(artifacts)
    }

    /// # Errors
    ///
    /// Returns an error if the proof artifact record cannot be removed.
    pub async fn remove_proof_artifact(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        let conn = self.connection().await?;
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let environment_id = self.environment_id().to_string();
        let artifact = conn
            .call(move |conn| {
                let artifact = conn
                    .query_row(
                        r"
                        SELECT environment_id, network_pair, proof_ref, pipeline_key, route,
                               proof_uri, content_hash, generation, invalidated_at, updated_at
                        FROM proof_artifacts
                        WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3
                          AND route = ?4 AND proof_ref = ?5
                        ",
                        params![environment_id, network_pair, pipeline_key, route, proof_ref],
                        proof_artifact_record_from_row,
                    )
                    .optional()?;
                if artifact.is_some() {
                    conn.execute(
                        "DELETE FROM proof_artifacts WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3 AND route = ?4 AND proof_ref = ?5",
                        params![environment_id, network_pair, pipeline_key, route, proof_ref],
                    )?;
                }
                Ok(artifact)
            })
            .await
            .context("failed to remove proof artifact")?;
        Ok(artifact)
    }

    /// Marks a proof artifact as unavailable for reuse while retaining it for deletion retries.
    ///
    /// # Errors
    ///
    /// Returns an error if the proof artifact record cannot be updated.
    pub async fn mark_proof_artifact_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        content_hash: &str,
    ) -> Result<bool> {
        let key = ProofArtifactKey {
            network_pair: network_pair.to_string(),
            pipeline_key,
            route,
            proof_ref: proof_ref.to_string(),
        };
        self.artifact_store
            .mark_invalidated(&key, content_hash)
            .await
            .context("failed to publish shared proof invalidation marker")?;
        let conn = self.connection().await?;
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let environment_id = self.environment_id().to_string();
        let content_hash = content_hash.to_string();
        let invalidated_at = now_ts();
        let updated = conn
            .call(move |conn| {
                Ok(conn.execute(
                    r"
                    UPDATE proof_artifacts
                    SET invalidated_at = COALESCE(invalidated_at, ?1), updated_at = ?1
                    WHERE environment_id = ?2 AND network_pair = ?3 AND pipeline_key = ?4
                      AND route = ?5 AND proof_ref = ?6 AND content_hash = ?7
                    ",
                    params![
                        invalidated_at,
                        environment_id,
                        network_pair,
                        pipeline_key,
                        route,
                        proof_ref,
                        content_hash
                    ],
                )?)
            })
            .await
            .context("failed to mark proof artifact invalidated")?;
        Ok(updated > 0)
    }

    /// Returns whether a proof artifact has been invalidated but not yet deleted.
    ///
    /// # Errors
    ///
    /// Returns an error if the proof artifact state cannot be loaded.
    pub async fn proof_artifact_is_invalidated(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        content_hash: &str,
    ) -> Result<bool> {
        let key = ProofArtifactKey {
            network_pair: network_pair.to_string(),
            pipeline_key,
            route,
            proof_ref: proof_ref.to_string(),
        };
        let conn = self.connection().await?;
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let environment_id = self.environment_id().to_string();
        let content_hash = content_hash.to_string();
        let queried_content_hash = content_hash.clone();
        let locally_invalidated = conn
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        r"
                    SELECT invalidated_at IS NOT NULL AND content_hash = ?6
                    FROM proof_artifacts
                    WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3
                      AND route = ?4 AND proof_ref = ?5
                    ",
                        params![
                            environment_id,
                            network_pair,
                            pipeline_key,
                            route,
                            proof_ref,
                            queried_content_hash
                        ],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(false))
            })
            .await
            .context("failed to query proof artifact invalidation state")?;
        if locally_invalidated {
            return Ok(true);
        }
        self.artifact_store
            .is_invalidated(&key, &content_hash)
            .await
            .context("failed to query shared proof invalidation marker")
    }

    /// Checkpoints a completed proof before publication begins.
    ///
    /// # Errors
    ///
    /// Returns an error if the publication outbox cannot be updated.
    pub async fn upsert_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
        proof_bytes: &[u8],
    ) -> Result<()> {
        let pending_key = pending_proof_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let pending = self
            .artifact_store
            .put_if_absent(&pending_key, proof_bytes)
            .await
            .context("failed to checkpoint pending proof in shared artifact store")?;
        let proof_bytes = pending.object().bytes.clone();
        let conn = self.connection().await?;
        let environment_id = self.environment_id().to_string();
        let network_pair = network_pair.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let proof_ref = proof_ref.to_string();
        let updated_at = now_ts();
        conn.call(move |conn| {
            conn.execute(
                r"
                INSERT INTO proof_publication_outbox (
                    environment_id, network_pair, pipeline_key, route, proof_ref,
                    proof_bytes, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                ON CONFLICT(environment_id, network_pair, pipeline_key, route, proof_ref)
                DO UPDATE SET proof_bytes = excluded.proof_bytes, updated_at = excluded.updated_at
                ",
                params![
                    environment_id,
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                    proof_bytes,
                    updated_at
                ],
            )?;
            Ok(())
        })
        .await
        .context("failed to checkpoint pending proof publication")
    }

    /// Loads a proof waiting to be published.
    ///
    /// # Errors
    ///
    /// Returns an error if the publication outbox cannot be queried.
    pub async fn get_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<Vec<u8>>> {
        let pending_key = pending_proof_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let canonical_key = ProofArtifactKey {
            network_pair: network_pair.to_string(),
            pipeline_key,
            route,
            proof_ref: proof_ref.to_string(),
        };
        if let Some(object) = self
            .artifact_store
            .get(&pending_key)
            .await
            .context("failed to load pending proof from shared artifact store")?
        {
            if self
                .artifact_store
                .is_invalidated(&canonical_key, &object.content_hash)
                .await
                .context("failed to check pending proof invalidation marker")?
            {
                self.artifact_store
                    .delete(&pending_key, object.generation, &object.content_hash)
                    .await
                    .context("failed to delete invalidated shared pending proof")?;
                let _ = self
                    .remove_local_pending_proof_publication(
                        network_pair,
                        pipeline_key,
                        route,
                        proof_ref,
                    )
                    .await?;
                return Ok(None);
            }
            return Ok(Some(object.bytes));
        }
        let local = self
            .get_local_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await?;
        let Some(bytes) = local else {
            return Ok(None);
        };
        if self
            .artifact_store
            .is_invalidated(&pending_key, PENDING_PUBLICATION_REMOVED_MARKER)
            .await
            .context("failed to check pending proof removal marker")?
        {
            let _ = self
                .remove_local_pending_proof_publication(
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                )
                .await?;
            return Ok(None);
        }
        let hash = artifact_store::content_hash(&bytes);
        if self
            .artifact_store
            .is_invalidated(&canonical_key, &hash)
            .await
            .context("failed to check local pending proof invalidation marker")?
        {
            let _ = self
                .remove_local_pending_proof_publication(
                    network_pair,
                    pipeline_key,
                    route,
                    proof_ref,
                )
                .await?;
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Removes a proof from the publication outbox after durable publication.
    ///
    /// # Errors
    ///
    /// Returns an error if the publication outbox cannot be updated.
    pub async fn remove_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let pending_key = pending_proof_artifact_key(network_pair, pipeline_key, route, proof_ref);
        self.artifact_store
            .mark_invalidated(&pending_key, PENDING_PUBLICATION_REMOVED_MARKER)
            .await
            .context("failed to mark pending proof publication removed")?;
        if let Some(object) = self
            .artifact_store
            .get(&pending_key)
            .await
            .context("failed to load shared pending proof before removal")?
        {
            self.artifact_store
                .delete(&pending_key, object.generation, &object.content_hash)
                .await
                .context("failed to remove shared pending proof")?;
        }
        self.remove_local_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await
    }

    /// Invalidates and removes a pending proof publication across replicas.
    ///
    /// # Errors
    ///
    /// Returns an error if the shared pending proof or its invalidation marker cannot be updated.
    pub async fn invalidate_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let pending_key = pending_proof_artifact_key(network_pair, pipeline_key, route, proof_ref);
        let canonical_key = ProofArtifactKey {
            network_pair: network_pair.to_string(),
            pipeline_key,
            route,
            proof_ref: proof_ref.to_string(),
        };
        self.artifact_store
            .mark_invalidated(&pending_key, PENDING_PUBLICATION_REMOVED_MARKER)
            .await
            .context("failed to mark pending proof publication removed")?;
        let mut invalidated = false;
        if let Some(object) = self
            .artifact_store
            .get(&pending_key)
            .await
            .context("failed to load shared pending proof for invalidation")?
        {
            self.artifact_store
                .mark_invalidated(&canonical_key, &object.content_hash)
                .await
                .context("failed to invalidate shared pending proof")?;
            self.artifact_store
                .delete(&pending_key, object.generation, &object.content_hash)
                .await
                .context("failed to remove invalidated shared pending proof")?;
            invalidated = true;
        }
        if let Some(bytes) = self
            .get_local_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await?
        {
            self.artifact_store
                .mark_invalidated(&canonical_key, &artifact_store::content_hash(&bytes))
                .await
                .context("failed to invalidate local pending proof")?;
            invalidated = true;
        }
        let removed = self
            .remove_local_pending_proof_publication(network_pair, pipeline_key, route, proof_ref)
            .await?;
        Ok(invalidated || removed)
    }

    async fn get_local_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<Option<Vec<u8>>> {
        let conn = self.connection().await?;
        let environment_id = self.environment_id().to_string();
        let network_pair = network_pair.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let proof_ref = proof_ref.to_string();
        conn.call(move |conn| {
            Ok(conn
                .query_row(
                    r"
                    SELECT proof_bytes
                    FROM proof_publication_outbox
                    WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3
                      AND route = ?4 AND proof_ref = ?5
                    ",
                    params![environment_id, network_pair, pipeline_key, route, proof_ref],
                    |row| row.get(0),
                )
                .optional()?)
        })
        .await
        .context("failed to load local pending proof publication")
    }

    async fn remove_local_pending_proof_publication(
        &self,
        network_pair: &str,
        pipeline_key: PipelineKey,
        route: PipelineRoute,
        proof_ref: &str,
    ) -> Result<bool> {
        let conn = self.connection().await?;
        let environment_id = self.environment_id().to_string();
        let network_pair = network_pair.to_string();
        let pipeline_key = pipeline_key.as_str().to_string();
        let route = route.to_string();
        let proof_ref = proof_ref.to_string();
        conn.call(move |conn| {
            Ok(conn.execute(
                r"
                DELETE FROM proof_publication_outbox
                WHERE environment_id = ?1 AND network_pair = ?2 AND pipeline_key = ?3
                  AND route = ?4 AND proof_ref = ?5
                ",
                params![environment_id, network_pair, pipeline_key, route, proof_ref],
            )? > 0)
        })
        .await
        .context("failed to remove pending proof publication")
    }

    /// # Errors
    ///
    /// Returns an error if runtime task records cannot be loaded.
    pub async fn list_tasks(&self) -> Result<Vec<RuntimeTaskRecord>> {
        let conn = self.connection().await?;
        let tasks = conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r"
                    SELECT
                        task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                        proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                        remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                        updated_at
                    FROM runtime_tasks
                    ORDER BY updated_at DESC, task_id ASC
                    ",
                )?;
                let mut rows = stmt.query([])?;
                let mut tasks = Vec::new();
                while let Some(row) = rows.next()? {
                    tasks.push(runtime_task_record_from_row(row)?);
                }
                Ok(tasks)
            })
            .await
            .context("failed to list runtime tasks")?;
        Ok(tasks)
    }

    /// # Errors
    ///
    /// Returns an error if expired runtime task records cannot be loaded.
    pub async fn list_expired_terminal_tasks(
        &self,
        now_ts: i64,
        ttl_secs: u64,
        after: Option<&ExpiredTaskCursor>,
        limit: usize,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        if ttl_secs == 0 || limit == 0 {
            return Ok(Vec::new());
        }

        let conn = self.connection().await?;
        let ttl_secs = i64::try_from(ttl_secs).unwrap_or(i64::MAX);
        let cutoff = now_ts.saturating_sub(ttl_secs);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let after = after.cloned();
        let tasks = conn
            .call(move |conn| {
                let mut stmt = if after.is_some() {
                    conn.prepare(
                        r"
                        SELECT
                            task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                            proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                            remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                            updated_at
                        FROM runtime_tasks
                        WHERE runner_status IN ('completed', 'failed', 'cancelled')
                            AND updated_at <= ?1
                            AND (updated_at > ?2 OR (updated_at = ?2 AND task_id > ?3))
                        ORDER BY updated_at ASC, task_id ASC
                        LIMIT ?4
                        ",
                    )?
                } else {
                    conn.prepare(
                        r"
                        SELECT
                            task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                            proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                            remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                            updated_at
                        FROM runtime_tasks
                        WHERE runner_status IN ('completed', 'failed', 'cancelled')
                            AND updated_at <= ?1
                        ORDER BY updated_at ASC, task_id ASC
                        LIMIT ?2
                        ",
                    )?
                };
                let mut rows = if let Some(after) = after {
                    stmt.query(params![cutoff, after.updated_at, after.task_id, limit])?
                } else {
                    stmt.query(params![cutoff, limit])?
                };
                let mut tasks = Vec::new();
                while let Some(row) = rows.next()? {
                    tasks.push(runtime_task_record_from_row(row)?);
                }
                Ok(tasks)
            })
            .await
            .context("failed to list expired terminal runtime tasks")?;
        Ok(tasks)
    }

    /// # Errors
    ///
    /// Returns an error if stale non-terminal runtime task records cannot be loaded.
    pub async fn list_stale_nonterminal_tasks(
        &self,
        now_ts: i64,
        ttl_secs: u64,
        after: Option<&ExpiredTaskCursor>,
        limit: usize,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        if ttl_secs == 0 || limit == 0 {
            return Ok(Vec::new());
        }

        let conn = self.connection().await?;
        let ttl_secs = i64::try_from(ttl_secs).unwrap_or(i64::MAX);
        let cutoff = now_ts.saturating_sub(ttl_secs);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let after = after.cloned();
        let tasks = conn
            .call(move |conn| {
                let mut stmt = if after.is_some() {
                    conn.prepare(
                        r"
                        SELECT
                            task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                            proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                            remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                            updated_at
                        FROM runtime_tasks
                        WHERE runner_status IN ('allocated', 'running')
                          AND updated_at <= ?1
                          AND (updated_at > ?2 OR (updated_at = ?2 AND task_id > ?3))
                        ORDER BY updated_at ASC, task_id ASC
                        LIMIT ?4
                        ",
                    )?
                } else {
                    conn.prepare(
                        r"
                        SELECT
                            task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                            proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                            remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                            updated_at
                        FROM runtime_tasks
                        WHERE runner_status IN ('allocated', 'running')
                          AND updated_at <= ?1
                        ORDER BY updated_at ASC, task_id ASC
                        LIMIT ?2
                        ",
                    )?
                };
                let mut rows = if let Some(after) = after {
                    stmt.query(params![cutoff, after.updated_at, after.task_id, limit])?
                } else {
                    stmt.query(params![cutoff, limit])?
                };
                let mut tasks = Vec::new();
                while let Some(row) = rows.next()? {
                    tasks.push(runtime_task_record_from_row(row)?);
                }
                Ok(tasks)
            })
            .await
            .context("failed to list stale non-terminal runtime tasks")?;
        Ok(tasks)
    }

    /// # Errors
    ///
    /// Returns an error if the task record or workspace cannot be deleted.
    pub async fn remove_task(&self, task_id: &str) -> Result<bool> {
        let Some(record) = self.get_task(task_id).await? else {
            return Ok(false);
        };

        let conn = self.connection().await?;
        let task_id = task_id.to_string();
        conn.call(move |conn| {
            conn.execute(
                "DELETE FROM runtime_tasks WHERE task_id = ?1",
                params![task_id],
            )?;
            Ok(())
        })
        .await
        .context("failed to delete runtime task")?;

        let task_dir = PathBuf::from(&record.task_dir);
        remove_task_workspace(&task_dir).await?;

        Ok(true)
    }

    fn build_task_record(&self, registration: &TaskRegistration) -> Result<RuntimeTaskRecord> {
        let pipeline_key = task_registration_pipeline_key(registration)?;
        let task_dir = self.task_dir(pipeline_key, &registration.task_id);
        Ok(RuntimeTaskRecord {
            task_id: registration.task_id.clone(),
            pipeline_key,
            route: registration.route,
            task_kind: registration.task_kind.clone(),
            proposal_id: registration.proposal_id,
            proof_ids: registration.proof_ids.clone(),
            runner_status: RunnerStatus::Allocated,
            task_dir: task_dir.display().to_string(),
            image_ref: None,
            provider_request_id: None,
            remote_tx_hash: None,
            proof_uri: None,
            error: None,
            metadata: registration.metadata.clone(),
            request_fingerprint: registration.request_fingerprint.clone(),
            updated_at: now_ts(),
        })
    }

    async fn write_task_workspace(&self, registration: &TaskRegistration) -> Result<()> {
        let pipeline_key = task_registration_pipeline_key(registration)?;
        let task_dir = self.task_dir(pipeline_key, &registration.task_id);
        fs::create_dir_all(task_dir.join("logs"))
            .await
            .with_context(|| format!("failed to create task workspace {}", task_dir.display()))?;
        fs::write(
            task_dir.join("request.json"),
            serde_json::to_vec_pretty(registration).context("serialize task registration")?,
        )
        .await
        .with_context(|| {
            format!(
                "failed to write {}",
                task_dir.join("request.json").display()
            )
        })?;
        Ok(())
    }

    async fn insert_task_if_absent(&self, record: &RuntimeTaskRecord) -> Result<bool> {
        let conn = self.connection().await?;
        let proof_ids_json =
            serde_json::to_string(&record.proof_ids).context("serialize proof_ids")?;
        let metadata_json =
            serde_json::to_string(&record.metadata).context("serialize runtime metadata")?;
        let record = record.clone();
        let inserted = conn
            .call(move |conn| {
            let inserted = conn.execute(
                r"
                INSERT OR IGNORE INTO runtime_tasks (
                    task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                    proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                    remote_tx_hash, proof_uri, error, metadata_json, request_fingerprint,
                    updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                ",
                params![
                    record.task_id,
                    record.pipeline_key.as_str(),
                    record.route.to_string(),
                    record.route.guest_system.to_string(),
                    record.route.runner.to_string(),
                    record.task_kind,
                    record.proposal_id,
                    proof_ids_json,
                    record.runner_status.as_str(),
                    record.task_dir,
                    record.image_ref,
                    record.provider_request_id,
                    record.remote_tx_hash,
                    record.proof_uri,
                    record.error,
                    metadata_json,
                    record.request_fingerprint,
                    record.updated_at,
                ],
            )?;
            Ok(inserted == 1)
            })
            .await
            .context("failed to insert runtime task")?;
        Ok(inserted)
    }

    async fn delete_task_row(&self, task_id: &str) -> Result<()> {
        let conn = self.connection().await?;
        let task_id = task_id.to_string();
        conn.call(move |conn| {
            conn.execute(
                "DELETE FROM runtime_tasks WHERE task_id = ?1",
                params![task_id],
            )?;
            Ok(())
        })
        .await
        .context("failed to delete runtime task row")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRegistration {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_key: Option<PipelineKey>,
    pub route: PipelineRoute,
    pub task_kind: String,
    pub proposal_id: Option<u64>,
    #[serde(default)]
    pub proof_ids: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskRecord {
    pub task_id: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub task_kind: String,
    pub proposal_id: Option<u64>,
    pub proof_ids: Vec<String>,
    pub runner_status: RunnerStatus,
    pub task_dir: String,
    pub image_ref: Option<String>,
    pub provider_request_id: Option<String>,
    pub remote_tx_hash: Option<String>,
    pub proof_uri: Option<String>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fingerprint: Option<String>,
    pub updated_at: i64,
}

fn task_registration_pipeline_key(registration: &TaskRegistration) -> Result<PipelineKey> {
    let pipeline_key = if let Some(pipeline_key) = registration.pipeline_key {
        pipeline_key
    } else {
        registration
            .route
            .pipeline_key()
            .map_err(anyhow::Error::msg)?
    };
    if !pipeline_key_matches_route(pipeline_key, registration.route) {
        anyhow::bail!(
            "pipeline_key '{}' does not match route '{}'",
            pipeline_key.as_str(),
            registration.route
        );
    }
    Ok(pipeline_key)
}

fn pipeline_key_matches_route(pipeline_key: PipelineKey, route: PipelineRoute) -> bool {
    match (pipeline_key, route) {
        (
            PipelineKey::ShastaSgx | PipelineKey::ShastaSgxGeth,
            PipelineRoute {
                guest_system: raiko2_pipeline::GuestSystem::Sgx,
                runner: raiko2_pipeline::RunnerKind::Remote,
            },
        ) => true,
        _ => route.pipeline_key() == Ok(pipeline_key),
    }
}

fn runtime_task_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeTaskRecord> {
    let proof_ids_json: String = row.get(7)?;
    let metadata_json: String = row.get(15)?;
    let pipeline_key_raw: String = row.get(1)?;
    let route_raw: String = row.get(2)?;
    let guest_system_raw: String = row.get(3)?;
    let runner_raw: String = row.get(4)?;
    let pipeline_key = pipeline_key_raw.parse::<PipelineKey>().map_err(|err| {
        invalid_runtime_task_row(
            1,
            format!("invalid pipeline_key '{pipeline_key_raw}': {err}"),
        )
    })?;
    let route = route_raw.parse::<PipelineRoute>().map_err(|err| {
        invalid_runtime_task_row(2, format!("invalid route '{route_raw}': {err}"))
    })?;
    if !pipeline_key_matches_route(pipeline_key, route) {
        return Err(invalid_runtime_task_row(
            1,
            format!("pipeline_key '{pipeline_key_raw}' does not match route '{route_raw}'"),
        ));
    }
    if guest_system_raw != route.guest_system.to_string() {
        return Err(invalid_runtime_task_row(
            3,
            format!("guest_system '{guest_system_raw}' does not match route '{route_raw}'"),
        ));
    }
    if runner_raw != route.runner.to_string() {
        return Err(invalid_runtime_task_row(
            4,
            format!("runner '{runner_raw}' does not match route '{route_raw}'"),
        ));
    }
    Ok(RuntimeTaskRecord {
        task_id: row.get(0)?,
        pipeline_key,
        route,
        task_kind: row.get(5)?,
        proposal_id: row.get(6)?,
        proof_ids: serde_json::from_str(&proof_ids_json).unwrap_or_default(),
        runner_status: RunnerStatus::from_db(row.get::<_, String>(8)?.as_str()),
        task_dir: row.get(9)?,
        image_ref: row.get(10)?,
        provider_request_id: row.get(11)?,
        remote_tx_hash: row.get(12)?,
        proof_uri: row.get(13)?,
        error: row.get(14)?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
        request_fingerprint: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn proof_artifact_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProofArtifactRecord> {
    let pipeline_key_raw: String = row.get(3)?;
    let route_raw: String = row.get(4)?;
    let pipeline_key = pipeline_key_raw.parse::<PipelineKey>().map_err(|err| {
        invalid_runtime_task_row(
            3,
            format!("invalid pipeline_key '{pipeline_key_raw}': {err}"),
        )
    })?;
    let route = route_raw.parse::<PipelineRoute>().map_err(|err| {
        invalid_runtime_task_row(4, format!("invalid route '{route_raw}': {err}"))
    })?;
    Ok(ProofArtifactRecord {
        environment_id: row.get(0)?,
        network_pair: row.get(1)?,
        proof_ref: row.get(2)?,
        pipeline_key,
        route,
        proof_uri: row.get(5)?,
        content_hash: row.get(6)?,
        generation: row.get(7)?,
        invalidated_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn migrate_runtime_schema(
    conn: &rusqlite::Connection,
    environment_id: &str,
) -> rusqlite::Result<()> {
    let transaction = conn.unchecked_transaction()?;
    bind_runtime_environment(&transaction, environment_id)?;
    if !table_column_exists(&transaction, "runtime_tasks", "request_fingerprint")? {
        transaction.execute(
            "ALTER TABLE runtime_tasks ADD COLUMN request_fingerprint TEXT",
            [],
        )?;
    }
    let runtime_proof_uri_exists = table_column_exists(&transaction, "runtime_tasks", "proof_uri")?;
    if !runtime_proof_uri_exists {
        transaction.execute("ALTER TABLE runtime_tasks ADD COLUMN proof_uri TEXT", [])?;
        if table_column_exists(&transaction, "runtime_tasks", "proof_path")? {
            transaction.execute(
                "UPDATE runtime_tasks SET proof_uri = CASE WHEN proof_path LIKE 'file://%' THEN proof_path ELSE 'file://' || proof_path END WHERE proof_path IS NOT NULL",
                [],
            )?;
        }
    }
    transaction.execute(
        r"
        CREATE UNIQUE INDEX IF NOT EXISTS runtime_tasks_request_fingerprint_uq
        ON runtime_tasks(request_fingerprint)
        WHERE request_fingerprint IS NOT NULL
        ",
        [],
    )?;
    transaction.execute(
        r"
        CREATE INDEX IF NOT EXISTS runtime_tasks_status_updated_at_idx
        ON runtime_tasks(runner_status, updated_at, task_id)
        ",
        [],
    )?;
    migrate_proof_artifact_schema(&transaction)?;
    transaction.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS proof_publication_outbox (
            environment_id TEXT NOT NULL,
            network_pair TEXT NOT NULL,
            pipeline_key TEXT NOT NULL,
            route TEXT NOT NULL,
            proof_ref TEXT NOT NULL,
            proof_bytes BLOB NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(environment_id, network_pair, pipeline_key, route, proof_ref)
        );
        CREATE INDEX IF NOT EXISTS proof_publication_outbox_updated_at_idx
        ON proof_publication_outbox(updated_at);
        ",
    )?;
    transaction.commit()
}

fn migrate_proof_artifact_schema(transaction: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let proof_artifacts_exists: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'proof_artifacts'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if proof_artifacts_exists.is_some()
        && !table_column_exists(transaction, "proof_artifacts", "environment_id")?
    {
        transaction.execute_batch(
            r"
            ALTER TABLE proof_artifacts RENAME TO proof_artifacts_legacy;
            CREATE TABLE proof_artifacts (
                environment_id TEXT NOT NULL,
                network_pair TEXT NOT NULL,
                proof_ref TEXT NOT NULL,
                pipeline_key TEXT NOT NULL,
                route TEXT NOT NULL,
                proof_uri TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                generation INTEGER,
                invalidated_at INTEGER,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(environment_id, network_pair, pipeline_key, route, proof_ref)
            );
            ",
        )?;
        // Legacy rows point at non-canonical local paths and have no verified content hash.
        // Startup recovery republishes completed runtime-task proofs into the configured store.
        transaction.execute_batch("DROP TABLE proof_artifacts_legacy;")?;
    } else {
        transaction.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS proof_artifacts (
                environment_id TEXT NOT NULL,
                network_pair TEXT NOT NULL,
                proof_ref TEXT NOT NULL,
                pipeline_key TEXT NOT NULL,
                route TEXT NOT NULL,
                proof_uri TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                generation INTEGER,
                invalidated_at INTEGER,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(environment_id, network_pair, pipeline_key, route, proof_ref)
            );
            ",
        )?;
    }
    if !table_column_exists(transaction, "proof_artifacts", "invalidated_at")? {
        transaction.execute(
            "ALTER TABLE proof_artifacts ADD COLUMN invalidated_at INTEGER",
            [],
        )?;
    }
    if !proof_artifact_route_is_identity(transaction)? {
        transaction.execute_batch(
            r"
            ALTER TABLE proof_artifacts RENAME TO proof_artifacts_without_route_identity;
            CREATE TABLE proof_artifacts (
                environment_id TEXT NOT NULL,
                network_pair TEXT NOT NULL,
                proof_ref TEXT NOT NULL,
                pipeline_key TEXT NOT NULL,
                route TEXT NOT NULL,
                proof_uri TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                generation INTEGER,
                invalidated_at INTEGER,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY(environment_id, network_pair, pipeline_key, route, proof_ref)
            );
            INSERT INTO proof_artifacts (
                environment_id, network_pair, proof_ref, pipeline_key, route, proof_uri,
                content_hash, generation, invalidated_at, updated_at
            )
            SELECT environment_id, network_pair, proof_ref, pipeline_key, route, proof_uri,
                   content_hash, generation, invalidated_at, updated_at
            FROM proof_artifacts_without_route_identity;
            DROP TABLE proof_artifacts_without_route_identity;
            ",
        )?;
    }
    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS proof_artifacts_updated_at_idx ON proof_artifacts(updated_at);",
    )?;
    Ok(())
}

fn proof_artifact_route_is_identity(
    transaction: &rusqlite::Transaction<'_>,
) -> rusqlite::Result<bool> {
    let mut statement = transaction.prepare("PRAGMA table_info(proof_artifacts)")?;
    let mut rows = statement.query([])?;
    let mut primary_key = Vec::new();
    while let Some(row) = rows.next()? {
        let position: i64 = row.get(5)?;
        if position > 0 {
            primary_key.push((position, row.get::<_, String>(1)?));
        }
    }
    primary_key.sort_by_key(|(position, _)| *position);
    Ok(primary_key.iter().map(|(_, column)| column.as_str()).eq([
        "environment_id",
        "network_pair",
        "pipeline_key",
        "route",
        "proof_ref",
    ]))
}

fn bind_runtime_environment(
    transaction: &rusqlite::Transaction<'_>,
    environment_id: &str,
) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS runtime_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    let stored: Option<String> = transaction
        .query_row(
            "SELECT value FROM runtime_metadata WHERE key = 'environment_id'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match stored {
        Some(stored) if stored != environment_id => Err(rusqlite::Error::InvalidParameterName(
            format!("runtime database belongs to environment '{stored}', not '{environment_id}'"),
        )),
        Some(_) => Ok(()),
        None => {
            transaction.execute(
                "INSERT INTO runtime_metadata (key, value) VALUES ('environment_id', ?1)",
                params![environment_id],
            )?;
            Ok(())
        }
    }
}

fn table_column_exists(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    let sql = format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1 LIMIT 1");
    Ok(conn
        .query_row(&sql, params![column], |row| row.get::<_, i64>(0))
        .optional()?
        .is_some())
}

async fn remove_task_workspace(task_dir: &Path) -> Result<()> {
    if fs::try_exists(task_dir).await? {
        fs::remove_dir_all(task_dir)
            .await
            .with_context(|| format!("failed to remove task workspace {}", task_dir.display()))?;
    }
    Ok(())
}

fn invalid_runtime_task_row(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Allocated,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl RunnerStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RunnerStatus::Allocated => "allocated",
            RunnerStatus::Running => "running",
            RunnerStatus::Completed => "completed",
            RunnerStatus::Failed => "failed",
            RunnerStatus::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn from_db(raw: &str) -> Self {
        match raw {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Allocated,
        }
    }
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed()
}

fn pending_proof_artifact_key(
    network_pair: &str,
    pipeline_key: PipelineKey,
    route: PipelineRoute,
    proof_ref: &str,
) -> ProofArtifactKey {
    ProofArtifactKey {
        network_pair: network_pair.to_string(),
        pipeline_key,
        route,
        proof_ref: format!("pending:{proof_ref}"),
    }
}

const PENDING_PUBLICATION_REMOVED_MARKER: &str = "publication-removed";

#[cfg(test)]
mod tests {
    use super::{
        ExpiredTaskCursor, FilesystemProofArtifactStore, ProofArtifactRegistration,
        ProofArtifactStore, RunnerStatus, RuntimeManager, TaskRegistration,
        TaskRegistrationOutcome,
    };
    use raiko2_pipeline::PipelineRoute;
    use rusqlite::OptionalExtension;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_root(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn runtime_database_rejects_a_different_environment() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-environment-test");
        let first = RuntimeManager::new_with_artifact_store(
            root.clone(),
            Arc::new(FilesystemProofArtifactStore::new(
                "devnet-a".to_string(),
                root.join("artifacts"),
            )?),
        )?;
        first.list_tasks().await?;
        drop(first);

        let second = RuntimeManager::new_with_artifact_store(
            root.clone(),
            Arc::new(FilesystemProofArtifactStore::new(
                "devnet-b".to_string(),
                root.join("artifacts"),
            )?),
        )?;
        let error = second
            .list_tasks()
            .await
            .expect_err("database environment must be immutable");
        assert!(format!("{error:#}").contains("devnet-a"), "{error:#}");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_registers_and_loads_task() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-test");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        let registration = TaskRegistration {
            task_id: "task-1".to_string(),
            pipeline_key: None,
            route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
            task_kind: "proposal".to_string(),
            proposal_id: Some(7),
            proof_ids: Vec::new(),
            metadata: serde_json::json!({"proposal_id": 7}),
            request_fingerprint: None,
        };
        runtime.register_task(registration).await?;

        let task = runtime.get_task("task-1").await?.expect("task present");
        assert_eq!(task.runner_status, RunnerStatus::Allocated);
        assert_eq!(task.proposal_id, Some(7));
        assert!(Path::new(&task.task_dir).exists());
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_persists_proof_artifact_uris() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-proof-artifact");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        let pipeline_key = raiko2_pipeline::PipelineKey::ShastaSp1;
        let route = "sp1/local".parse::<PipelineRoute>().expect("parse route");
        let proof_uri =
            runtime.proof_artifact_uri("taiko_dev/ethereum", pipeline_key, route, "proposal_0xabc");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: "proposal_0xabc".to_string(),
                pipeline_key,
                route,
                proof_uri: proof_uri.clone(),
                content_hash: "hash".to_string(),
                generation: None,
            })
            .await?;

        let artifact = runtime
            .get_proof_artifact("taiko_dev/ethereum", pipeline_key, route, "proposal_0xabc")
            .await?
            .expect("proof artifact");
        assert_eq!(artifact.proof_uri, proof_uri);
        assert_eq!(artifact.network_pair, "taiko_dev/ethereum");
        assert_eq!(artifact.proof_ref, "proposal_0xabc");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn proof_artifact_tombstone_survives_reregistration() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-proof-artifact-tombstone");
        let runtime = RuntimeManager::new(root.clone())?;
        let pipeline_key = raiko2_pipeline::PipelineKey::ShastaSp1;
        let registration = ProofArtifactRegistration {
            network_pair: "taiko_dev/ethereum".to_string(),
            proof_ref: "proposal_0xabc".to_string(),
            pipeline_key,
            route: "sp1/network".parse::<PipelineRoute>().expect("parse route"),
            proof_uri: "file:///proof.json".to_string(),
            content_hash: "hash".to_string(),
            generation: Some(7),
        };
        runtime.upsert_proof_artifact(registration.clone()).await?;
        assert!(
            runtime
                .mark_proof_artifact_invalidated(
                    &registration.network_pair,
                    pipeline_key,
                    registration.route,
                    &registration.proof_ref,
                    &registration.content_hash,
                )
                .await?
        );

        runtime.upsert_proof_artifact(registration.clone()).await?;

        assert!(
            runtime
                .get_proof_artifact(
                    &registration.network_pair,
                    pipeline_key,
                    registration.route,
                    &registration.proof_ref,
                )
                .await?
                .is_none(),
            "re-registration cleared the tombstone"
        );
        let artifacts = runtime.list_proof_artifacts().await?;
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].invalidated_at.is_some());

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn shared_invalidation_marker_is_visible_across_runtime_replicas() -> anyhow::Result<()> {
        let artifact_root = unique_root("raiko2-runtime-shared-artifacts");
        let store: Arc<dyn ProofArtifactStore> = Arc::new(FilesystemProofArtifactStore::new(
            "shared-environment".to_string(),
            artifact_root.clone(),
        )?);
        let first = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-replica-a"),
            Arc::clone(&store),
        )?;
        let second = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-replica-b"),
            store,
        )?;
        let pipeline = raiko2_pipeline::PipelineKey::ShastaSp1;
        let route = "sp1/network".parse::<PipelineRoute>().expect("route");
        let publication = first
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                pipeline,
                route,
                "proposal_0xabc",
                b"proof",
            )
            .await?;
        let object = publication.object();
        first
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: "proposal_0xabc".to_string(),
                pipeline_key: pipeline,
                route,
                proof_uri: object.proof_uri.clone(),
                content_hash: object.content_hash.clone(),
                generation: object.generation,
            })
            .await?;
        first
            .mark_proof_artifact_invalidated(
                "taiko_dev/ethereum",
                pipeline,
                route,
                "proposal_0xabc",
                &object.content_hash,
            )
            .await?;

        assert!(
            second
                .proof_artifact_is_invalidated(
                    "taiko_dev/ethereum",
                    pipeline,
                    route,
                    "proposal_0xabc",
                    &object.content_hash,
                )
                .await?
        );
        assert!(second.list_proof_artifacts().await?.is_empty());
        std::fs::remove_dir_all(artifact_root)?;
        Ok(())
    }

    #[tokio::test]
    async fn shared_pending_publication_invalidation_is_visible_across_replicas()
    -> anyhow::Result<()> {
        let artifact_root = unique_root("raiko2-runtime-shared-pending-artifacts");
        let store: Arc<dyn ProofArtifactStore> = Arc::new(FilesystemProofArtifactStore::new(
            "shared-environment".to_string(),
            artifact_root.clone(),
        )?);
        let first = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-pending-replica-a"),
            Arc::clone(&store),
        )?;
        let second = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-pending-replica-b"),
            store,
        )?;
        let pipeline = raiko2_pipeline::PipelineKey::ShastaSp1;
        let route = "sp1/network".parse::<PipelineRoute>().expect("route");

        first
            .upsert_pending_proof_publication(
                "taiko_dev/ethereum",
                pipeline,
                route,
                "proposal_0xpending",
                b"pending-proof",
            )
            .await?;
        assert!(
            second
                .get_pending_proof_publication(
                    "taiko_dev/ethereum",
                    pipeline,
                    route,
                    "proposal_0xpending",
                )
                .await?
                .is_some()
        );

        second
            .invalidate_pending_proof_publication(
                "taiko_dev/ethereum",
                pipeline,
                route,
                "proposal_0xpending",
            )
            .await?;

        assert!(
            first
                .get_pending_proof_publication(
                    "taiko_dev/ethereum",
                    pipeline,
                    route,
                    "proposal_0xpending",
                )
                .await?
                .is_none()
        );
        std::fs::remove_dir_all(artifact_root)?;
        Ok(())
    }

    #[tokio::test]
    async fn removed_shared_pending_publication_blocks_replica_local_fallback() -> anyhow::Result<()>
    {
        let artifact_root = unique_root("raiko2-runtime-removed-pending-artifacts");
        let store: Arc<dyn ProofArtifactStore> = Arc::new(FilesystemProofArtifactStore::new(
            "shared-environment".to_string(),
            artifact_root.clone(),
        )?);
        let first = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-removed-pending-a"),
            Arc::clone(&store),
        )?;
        let second = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-removed-pending-b"),
            store,
        )?;
        let pipeline = raiko2_pipeline::PipelineKey::ShastaSp1;
        let route = "sp1/network".parse::<PipelineRoute>().expect("route");
        let network_pair = "taiko_dev/ethereum";
        let proof_ref = "proposal_0xremoved";

        first
            .upsert_pending_proof_publication(
                network_pair,
                pipeline,
                route,
                proof_ref,
                b"pending-proof",
            )
            .await?;
        second
            .upsert_pending_proof_publication(
                network_pair,
                pipeline,
                route,
                proof_ref,
                b"pending-proof",
            )
            .await?;

        first
            .remove_pending_proof_publication(network_pair, pipeline, route, proof_ref)
            .await?;

        assert!(
            second
                .get_pending_proof_publication(network_pair, pipeline, route, proof_ref)
                .await?
                .is_none(),
            "a replica-local checkpoint survived deliberate shared removal"
        );
        std::fs::remove_dir_all(artifact_root)?;
        Ok(())
    }

    #[tokio::test]
    async fn local_tombstone_does_not_reject_new_content_hash() -> anyhow::Result<()> {
        let artifact_root = unique_root("raiko2-runtime-content-scoped-artifacts");
        let store: Arc<dyn ProofArtifactStore> = Arc::new(FilesystemProofArtifactStore::new(
            "shared-environment".to_string(),
            artifact_root.clone(),
        )?);
        let first = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-content-replica-a"),
            Arc::clone(&store),
        )?;
        let second = RuntimeManager::new_with_artifact_store(
            unique_root("raiko2-runtime-content-replica-b"),
            store,
        )?;
        let pipeline = raiko2_pipeline::PipelineKey::ShastaNative;
        let route = pipeline.route();
        let network_pair = "taiko_dev/ethereum";
        let proof_ref = "proposal_0xcontent";
        let old = first
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"old-proof")
            .await?;
        let old_object = old.object();
        first
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key: pipeline,
                route,
                proof_uri: old_object.proof_uri.clone(),
                content_hash: old_object.content_hash.clone(),
                generation: old_object.generation,
            })
            .await?;
        first
            .mark_proof_artifact_invalidated(
                network_pair,
                pipeline,
                route,
                proof_ref,
                &old_object.content_hash,
            )
            .await?;
        second
            .delete_proof_artifact(
                network_pair,
                pipeline,
                route,
                proof_ref,
                old_object.generation,
                &old_object.content_hash,
            )
            .await?;

        let new = second
            .publish_proof_artifact_bytes(network_pair, pipeline, route, proof_ref, b"new-proof")
            .await?;
        let new_object = new.object();
        assert!(
            !first
                .proof_artifact_is_invalidated(
                    network_pair,
                    pipeline,
                    route,
                    proof_ref,
                    &new_object.content_hash,
                )
                .await?
        );
        first
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: network_pair.to_string(),
                proof_ref: proof_ref.to_string(),
                pipeline_key: pipeline,
                route,
                proof_uri: new_object.proof_uri.clone(),
                content_hash: new_object.content_hash.clone(),
                generation: new_object.generation,
            })
            .await?;
        let record = first
            .get_proof_artifact(network_pair, pipeline, route, proof_ref)
            .await?
            .expect("new content must be active");
        assert!(record.invalidated_at.is_none());
        std::fs::remove_dir_all(artifact_root)?;
        Ok(())
    }

    #[tokio::test]
    async fn proof_artifact_publication_does_not_overwrite_existing_content() -> anyhow::Result<()>
    {
        let root = unique_root("raiko2-runtime-proof-create-only");
        let runtime = RuntimeManager::new(root.clone())?;

        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                raiko2_pipeline::PipelineKey::ShastaSp1,
                raiko2_pipeline::PipelineKey::ShastaSp1.route(),
                "proposal_0xabc",
                b"first",
            )
            .await?;
        runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                raiko2_pipeline::PipelineKey::ShastaSp1,
                raiko2_pipeline::PipelineKey::ShastaSp1.route(),
                "proposal_0xabc",
                b"second",
            )
            .await?;

        let artifact = runtime
            .read_proof_artifact_bytes(
                "taiko_dev/ethereum",
                raiko2_pipeline::PipelineKey::ShastaSp1,
                raiko2_pipeline::PipelineKey::ShastaSp1.route(),
                "proposal_0xabc",
            )
            .await?
            .expect("artifact");
        assert_eq!(artifact.bytes, b"first");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn proof_artifact_identity_includes_execution_route() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-proof-route-identity");
        let runtime = RuntimeManager::new(root.clone())?;
        let pipeline = raiko2_pipeline::PipelineKey::ShastaSp1;
        let local = PipelineRoute::new(
            raiko2_pipeline::GuestSystem::Sp1,
            raiko2_pipeline::RunnerKind::Local,
        );
        let network = PipelineRoute::new(
            raiko2_pipeline::GuestSystem::Sp1,
            raiko2_pipeline::RunnerKind::Network,
        );

        let local_put = runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                pipeline,
                local,
                "proposal_0xabc",
                b"local",
            )
            .await?;
        let network_put = runtime
            .publish_proof_artifact_bytes(
                "taiko_dev/ethereum",
                pipeline,
                network,
                "proposal_0xabc",
                b"network",
            )
            .await?;
        for (route, artifact) in [(local, local_put), (network, network_put)] {
            let artifact = artifact.object();
            runtime
                .upsert_proof_artifact(ProofArtifactRegistration {
                    network_pair: "taiko_dev/ethereum".to_string(),
                    proof_ref: "proposal_0xabc".to_string(),
                    pipeline_key: pipeline,
                    route,
                    proof_uri: artifact.proof_uri.clone(),
                    content_hash: artifact.content_hash.clone(),
                    generation: artifact.generation,
                })
                .await?;
        }

        let local_artifact = runtime
            .read_proof_artifact_bytes("taiko_dev/ethereum", pipeline, local, "proposal_0xabc")
            .await?
            .expect("local artifact");
        let network_artifact = runtime
            .read_proof_artifact_bytes("taiko_dev/ethereum", pipeline, network, "proposal_0xabc")
            .await?
            .expect("network artifact");

        assert_eq!(local_artifact.bytes, b"local");
        assert_eq!(network_artifact.bytes, b"network");
        assert_ne!(local_artifact.proof_uri, network_artifact.proof_uri);
        assert_eq!(
            runtime
                .get_proof_artifact("taiko_dev/ethereum", pipeline, local, "proposal_0xabc")
                .await?
                .expect("local registration")
                .route,
            local
        );
        assert_eq!(
            runtime
                .get_proof_artifact("taiko_dev/ethereum", pipeline, network, "proposal_0xabc")
                .await?
                .expect("network registration")
                .route,
            network
        );
        assert_eq!(runtime.list_proof_artifacts().await?.len(), 2);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_finds_task_by_engine_task_id() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-engine-lookup");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        runtime
            .register_task(TaskRegistration {
                task_id: "task-public".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(7),
                proof_ids: vec!["proposal-proof-task".to_string()],
                metadata: serde_json::json!({
                    "aggregate_task_id": "aggregate-proof-task",
                }),
                request_fingerprint: None,
            })
            .await?;

        let proposal = runtime
            .find_task_by_engine_task_id("proposal-proof-task")
            .await?
            .expect("proposal proof lookup");
        assert_eq!(proposal.task_id, "task-public");

        let aggregate = runtime
            .find_task_by_engine_task_id("aggregate-proof-task")
            .await?
            .expect("aggregate proof lookup");
        assert_eq!(aggregate.task_id, "task-public");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_lists_all_tasks_by_engine_task_id() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-engine-lookup-all");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        for task_id in ["task-public-a", "task-public-b"] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: None,
                    route: "risc0/network"
                        .parse::<PipelineRoute>()
                        .expect("parse route"),
                    task_kind: "hoodi_batch".to_string(),
                    proposal_id: Some(7),
                    proof_ids: vec!["shared-proposal-proof-task".to_string()],
                    metadata: serde_json::json!({
                        "aggregate_task_id": "shared-aggregate-proof-task",
                    }),
                    request_fingerprint: None,
                })
                .await?;
        }

        let shared = runtime
            .find_tasks_by_engine_task_id("shared-proposal-proof-task")
            .await?;
        assert_eq!(shared.len(), 2);
        let mut task_ids = shared
            .iter()
            .map(|record| record.task_id.as_str())
            .collect::<Vec<_>>();
        task_ids.sort_unstable();
        assert_eq!(task_ids, vec!["task-public-a", "task-public-b"]);

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_rejects_inconsistent_route_identity() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-invalid-route");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        let conn = runtime.connection().await?;
        conn.call(|conn| {
            conn.execute(
                r"
                INSERT INTO runtime_tasks (
                    task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                    proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                    remote_tx_hash, proof_uri, error, metadata_json, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                ",
                rusqlite::params![
                    "task-invalid",
                    "shasta-native-local",
                    "risc0/local",
                    "native",
                    "local",
                    "hoodi_batch",
                    Option::<u64>::None,
                    "[]",
                    "allocated",
                    "/tmp/runtime-task",
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    "{}",
                    1_i64,
                ],
            )?;
            Ok(())
        })
        .await?;

        let err = runtime
            .get_task("task-invalid")
            .await
            .expect_err("mismatched runtime row should fail");
        assert!(
            err.chain().any(|source| source
                .to_string()
                .contains("does not match route 'risc0/local'")),
            "{err:?}"
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_lists_only_expired_terminal_tasks() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-expired");
        let runtime = RuntimeManager::new(root.clone())?;
        for task_id in ["completed-task", "running-task", "cancelled-task"] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: None,
                    route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
                    task_kind: "hoodi_batch".to_string(),
                    proposal_id: Some(7),
                    proof_ids: Vec::new(),
                    metadata: serde_json::json!({}),
                    request_fingerprint: None,
                })
                .await?;
        }

        for (task_id, status, updated_at) in [
            ("completed-task", RunnerStatus::Completed, 10_i64),
            ("running-task", RunnerStatus::Running, 10_i64),
            ("cancelled-task", RunnerStatus::Cancelled, 20_i64),
        ] {
            let mut record = runtime.get_task(task_id).await?.expect("task present");
            record.runner_status = status;
            record.updated_at = updated_at;
            runtime.upsert_task(&record).await?;
        }

        let expired = runtime
            .list_expired_terminal_tasks(7_220, 7_200, None, 8)
            .await?;

        assert_eq!(expired.len(), 2);
        assert_eq!(expired[0].task_id, "completed-task");
        assert_eq!(expired[1].task_id, "cancelled-task");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_lists_only_stale_nonterminal_tasks() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-stale-nonterminal");
        let runtime = RuntimeManager::new(root.clone())?;
        for task_id in [
            "allocated-task",
            "running-task",
            "fresh-task",
            "failed-task",
        ] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: None,
                    route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
                    task_kind: "hoodi_batch".to_string(),
                    proposal_id: Some(7),
                    proof_ids: Vec::new(),
                    metadata: serde_json::json!({}),
                    request_fingerprint: None,
                })
                .await?;
        }

        for (task_id, status, updated_at) in [
            ("allocated-task", RunnerStatus::Allocated, 10_i64),
            ("running-task", RunnerStatus::Running, 20_i64),
            ("fresh-task", RunnerStatus::Running, 40_i64),
            ("failed-task", RunnerStatus::Failed, 10_i64),
        ] {
            let mut record = runtime.get_task(task_id).await?.expect("task present");
            record.runner_status = status;
            record.updated_at = updated_at;
            runtime.upsert_task(&record).await?;
        }

        let stale = runtime
            .list_stale_nonterminal_tasks(7_230, 7_200, None, 8)
            .await?;
        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].task_id, "allocated-task");
        assert_eq!(stale[1].task_id, "running-task");

        let after = ExpiredTaskCursor {
            updated_at: stale[0].updated_at,
            task_id: stale[0].task_id.clone(),
        };
        let stale_after = runtime
            .list_stale_nonterminal_tasks(7_230, 7_200, Some(&after), 8)
            .await?;
        assert_eq!(stale_after.len(), 1);
        assert_eq!(stale_after[0].task_id, "running-task");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_cancels_only_matching_nonterminal_task() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-cancel-stale-nonterminal");
        let runtime = RuntimeManager::new(root.clone())?;
        for (task_id, status, updated_at) in [
            ("stale-running", RunnerStatus::Running, 10_i64),
            ("fresh-running", RunnerStatus::Running, 30_i64),
            ("failed-task", RunnerStatus::Failed, 10_i64),
        ] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: None,
                    route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
                    task_kind: "hoodi_batch".to_string(),
                    proposal_id: Some(7),
                    proof_ids: Vec::new(),
                    metadata: serde_json::json!({}),
                    request_fingerprint: None,
                })
                .await?;
            let mut record = runtime.get_task(task_id).await?.expect("task present");
            record.runner_status = status;
            record.updated_at = updated_at;
            runtime.upsert_task(&record).await?;
        }

        assert!(
            runtime
                .cancel_nonterminal_task_if_stale("stale-running", 20, Some("stale".to_string()))
                .await?
        );
        assert!(
            !runtime
                .cancel_nonterminal_task_if_stale("fresh-running", 20, None)
                .await?
        );
        assert!(
            !runtime
                .cancel_nonterminal_task_if_stale("failed-task", 20, None)
                .await?
        );

        let stale = runtime
            .get_task("stale-running")
            .await?
            .expect("stale task");
        assert_eq!(stale.runner_status, RunnerStatus::Cancelled);
        assert_eq!(stale.error.as_deref(), Some("stale"));
        let fresh = runtime
            .get_task("fresh-running")
            .await?
            .expect("fresh task");
        assert_eq!(fresh.runner_status, RunnerStatus::Running);
        let failed = runtime.get_task("failed-task").await?.expect("failed task");
        assert_eq!(failed.runner_status, RunnerStatus::Failed);

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_pages_expired_terminal_tasks_after_cursor() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-expired-page");
        let runtime = RuntimeManager::new(root.clone())?;
        for (task_id, status, updated_at) in [
            ("task-a", RunnerStatus::Completed, 10_i64),
            ("task-b", RunnerStatus::Failed, 20_i64),
            ("task-c", RunnerStatus::Cancelled, 30_i64),
        ] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: None,
                    route: "risc0/local".parse::<PipelineRoute>().expect("parse route"),
                    task_kind: "hoodi_batch".to_string(),
                    proposal_id: Some(7),
                    proof_ids: Vec::new(),
                    metadata: serde_json::json!({}),
                    request_fingerprint: None,
                })
                .await?;
            let mut record = runtime.get_task(task_id).await?.expect("task present");
            record.runner_status = status;
            record.updated_at = updated_at;
            runtime.upsert_task(&record).await?;
        }

        let expired = runtime
            .list_expired_terminal_tasks(
                7_230,
                7_200,
                Some(&ExpiredTaskCursor {
                    updated_at: 20,
                    task_id: "task-b".to_string(),
                }),
                8,
            )
            .await?;

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].task_id, "task-c");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_drops_legacy_artifact_rows_for_republication() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-migration");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        tokio::fs::create_dir_all(root.join("state")).await?;
        let conn =
            tokio_rusqlite::Connection::open(root.join("state").join("runtime.sqlite")).await?;
        conn.call(|conn| {
            conn.execute_batch(
                r"
                CREATE TABLE runtime_tasks (
                    task_id TEXT PRIMARY KEY,
                    pipeline_key TEXT NOT NULL,
                    route TEXT NOT NULL,
                    guest_system TEXT NOT NULL,
                    runner TEXT NOT NULL,
                    task_kind TEXT NOT NULL,
                    proposal_id INTEGER,
                    proof_ids_json TEXT,
                    runner_status TEXT NOT NULL,
                    task_dir TEXT NOT NULL,
                    image_ref TEXT,
                    provider_request_id TEXT,
                    remote_tx_hash TEXT,
                    proof_uri TEXT,
                    error TEXT,
                    metadata_json TEXT,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE proof_artifacts (
                    network_pair TEXT NOT NULL,
                    proof_ref TEXT NOT NULL,
                    pipeline_key TEXT NOT NULL,
                    route TEXT NOT NULL,
                    proof_path TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY(network_pair, proof_ref)
                );
                INSERT INTO proof_artifacts (
                    network_pair, proof_ref, pipeline_key, route, proof_path, updated_at
                ) VALUES (
                    'taiko_dev/ethereum', 'legacy-proof', 'shasta-sp1-local', 'sp1/local',
                    '/tmp/legacy-proof.json', 1
                );
                ",
            )?;
            Ok(())
        })
        .await?;
        drop(conn);

        let runtime = RuntimeManager::new(root.clone())?;
        let _ = runtime.list_tasks().await?;
        let artifact = runtime
            .get_proof_artifact(
                "taiko_dev/ethereum",
                raiko2_pipeline::PipelineKey::ShastaSp1,
                raiko2_pipeline::PipelineKey::ShastaSp1.route(),
                "legacy-proof",
            )
            .await?;
        assert!(
            artifact.is_none(),
            "legacy locations must not masquerade as canonical artifacts"
        );

        let conn =
            tokio_rusqlite::Connection::open(root.join("state").join("runtime.sqlite")).await?;
        let (request_fingerprint_exists, index_exists): (Option<i64>, Option<i64>) = conn
            .call(|conn| {
                let request_fingerprint_exists = conn
                    .query_row(
                        "SELECT 1 FROM pragma_table_info('runtime_tasks') WHERE name = 'request_fingerprint' LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                let index_exists = conn
                    .query_row(
                        "SELECT 1 FROM pragma_index_list('runtime_tasks') WHERE name = 'runtime_tasks_request_fingerprint_uq' LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok((request_fingerprint_exists, index_exists))
            })
            .await?;
        assert_eq!(request_fingerprint_exists, Some(1));
        assert_eq!(index_exists, Some(1));

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_finds_task_by_request_fingerprint() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-fingerprint-lookup");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        runtime
            .register_task(TaskRegistration {
                task_id: "task-public".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(7),
                proof_ids: vec!["proposal-proof-task".to_string()],
                metadata: serde_json::json!({}),
                request_fingerprint: Some("0xfingerprint".to_string()),
            })
            .await?;

        let record = runtime
            .find_task_by_request_fingerprint("0xfingerprint")
            .await?
            .expect("task present");
        assert_eq!(record.task_id, "task-public");

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_register_task_if_absent_reuses_existing_task() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-fingerprint-idempotent");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;

        let created = runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "task-created".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(7),
                proof_ids: vec!["proposal-proof-task".to_string()],
                metadata: serde_json::json!({}),
                request_fingerprint: Some("0xfingerprint".to_string()),
            })
            .await?;
        let TaskRegistrationOutcome::Created(created) = created else {
            panic!("first registration should create a task");
        };

        let existing = runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "task-duplicate".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(7),
                proof_ids: vec!["proposal-proof-task".to_string()],
                metadata: serde_json::json!({}),
                request_fingerprint: Some("0xfingerprint".to_string()),
            })
            .await?;
        let TaskRegistrationOutcome::Existing(existing) = existing else {
            panic!("duplicate registration should reuse the existing task");
        };

        assert_eq!(existing.task_id, created.task_id);
        assert_eq!(runtime.list_tasks().await?.len(), 1);

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_register_task_if_absent_returns_existing_task_id_conflict()
    -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-task-id-conflict");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;

        let created = runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "task-shared".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(7),
                proof_ids: vec!["proposal-proof-task".to_string()],
                metadata: serde_json::json!({}),
                request_fingerprint: Some("0xfingerprint-a".to_string()),
            })
            .await?;
        let TaskRegistrationOutcome::Created(created) = created else {
            panic!("first registration should create a task");
        };

        let existing = runtime
            .register_task_if_absent(TaskRegistration {
                task_id: "task-shared".to_string(),
                pipeline_key: None,
                route: "risc0/network"
                    .parse::<PipelineRoute>()
                    .expect("parse route"),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(8),
                proof_ids: vec!["other-proposal-proof-task".to_string()],
                metadata: serde_json::json!({}),
                request_fingerprint: Some("0xfingerprint-b".to_string()),
            })
            .await?;
        let TaskRegistrationOutcome::Existing(existing) = existing else {
            panic!("task id conflict should return the existing task");
        };

        assert_eq!(existing.task_id, created.task_id);
        assert_eq!(
            existing.request_fingerprint.as_deref(),
            Some("0xfingerprint-a")
        );
        assert_eq!(runtime.list_tasks().await?.len(), 1);

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_manager_register_task_if_absent_is_safe_under_concurrency()
    -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-fingerprint-concurrent");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = Arc::new(RuntimeManager::new(root.clone())?);
        let mut handles = Vec::new();

        for index in 0..8 {
            let runtime = Arc::clone(&runtime);
            handles.push(tokio::spawn(async move {
                runtime
                    .register_task_if_absent(TaskRegistration {
                        task_id: format!("task-{index}"),
                        pipeline_key: None,
                        route: "risc0/network"
                            .parse::<PipelineRoute>()
                            .expect("parse route"),
                        task_kind: "hoodi_batch".to_string(),
                        proposal_id: Some(7),
                        proof_ids: vec!["proposal-proof-task".to_string()],
                        metadata: serde_json::json!({}),
                        request_fingerprint: Some("0xfingerprint".to_string()),
                    })
                    .await
            }));
        }

        let mut task_ids = Vec::new();
        for handle in handles {
            let outcome = handle.await.expect("join task")?;
            let task_id = match outcome {
                TaskRegistrationOutcome::Created(record)
                | TaskRegistrationOutcome::Existing(record) => record.task_id,
            };
            task_ids.push(task_id);
        }

        let first_task_id = task_ids.first().cloned().expect("at least one task id");
        assert!(task_ids.iter().all(|task_id| task_id == &first_task_id));
        assert_eq!(runtime.list_tasks().await?.len(), 1);

        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
