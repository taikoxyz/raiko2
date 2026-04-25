#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

use anyhow::{Context, Result};
use raiko2_pipeline::{PipelineKey, PipelineRoute};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::fs as stdfs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;

/// Runtime root managed by the host process.
#[derive(Debug, Clone)]
pub struct RuntimeManager {
    root: PathBuf,
    db_path: PathBuf,
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
    pub proof_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofArtifactRecord {
    pub network_pair: String,
    pub proof_ref: String,
    pub pipeline_key: PipelineKey,
    pub route: PipelineRoute,
    pub proof_path: String,
    pub updated_at: i64,
}

impl RuntimeManager {
    /// # Errors
    ///
    /// Returns an error if the runtime layout cannot be created.
    pub fn new(root: PathBuf) -> Result<Self> {
        let manager = Self {
            db_path: root.join("state").join("runtime.sqlite"),
            root,
            conn: OnceCell::new(),
        };
        manager.ensure_layout()?;
        Ok(manager)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
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
        let conn = self
            .conn
            .get_or_try_init(|| async move {
                let conn = tokio_rusqlite::Connection::open(db_path)
                    .await
                    .context("failed to open runtime sqlite database")?;
                conn.call(|conn| {
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
                            proof_path TEXT,
                            error TEXT,
                            metadata_json TEXT,
                            request_fingerprint TEXT,
                            updated_at INTEGER NOT NULL
                        );
                        ",
                    )?;
                    migrate_runtime_schema(conn)?;
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

    #[must_use]
    pub fn proof_artifact_path(&self, network_pair: &str, proof_ref: &str) -> PathBuf {
        self.root
            .join("cache")
            .join("proofs")
            .join(safe_path_component(network_pair))
            .join(format!("{}.json", safe_path_component(proof_ref)))
    }

    /// # Errors
    ///
    /// Returns an error if the proof artifact cannot be atomically published.
    pub async fn write_proof_artifact_bytes(
        &self,
        network_pair: &str,
        proof_ref: &str,
        bytes: &[u8],
    ) -> Result<PathBuf> {
        let path = self.proof_artifact_path(network_pair, proof_ref);
        write_file_atomic(&path, bytes).await?;
        Ok(path)
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
            .clone()
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

        let existing = self
            .find_task_by_request_fingerprint(&request_fingerprint)
            .await?
            .context("request fingerprint conflict without a matching runtime task")?;
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
                    remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
                    proof_path = excluded.proof_path,
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
                    record.proof_path,
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
                        remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
                        remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
                        remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
        proof_path: Option<String>,
    ) -> Result<()> {
        let Some(mut record) = self.get_task(task_id).await? else {
            return Ok(());
        };
        record.runner_status = runner_status;
        record.error = error;
        if proof_path.is_some() {
            record.proof_path = proof_path;
        }
        record.updated_at = now_ts();
        self.upsert_task(&record).await
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
        conn.call(move |conn| {
            conn.execute(
                r"
                INSERT INTO proof_artifacts (
                    network_pair, proof_ref, pipeline_key, route, proof_path, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(network_pair, proof_ref) DO UPDATE SET
                    pipeline_key = excluded.pipeline_key,
                    route = excluded.route,
                    proof_path = excluded.proof_path,
                    updated_at = excluded.updated_at
                ",
                params![
                    registration.network_pair,
                    registration.proof_ref,
                    registration.pipeline_key.as_str(),
                    registration.route.to_string(),
                    registration.proof_path,
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
        proof_ref: &str,
    ) -> Result<Option<ProofArtifactRecord>> {
        let conn = self.connection().await?;
        let network_pair = network_pair.to_string();
        let proof_ref = proof_ref.to_string();
        let artifact = conn
            .call(move |conn| {
                Ok(conn
                    .query_row(
                        r"
                    SELECT network_pair, proof_ref, pipeline_key, route, proof_path, updated_at
                    FROM proof_artifacts
                    WHERE network_pair = ?1 AND proof_ref = ?2
                    ",
                        params![network_pair, proof_ref],
                        proof_artifact_record_from_row,
                    )
                    .optional()?)
            })
            .await
            .context("failed to query proof artifact")?;
        Ok(artifact)
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
                        remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
                            remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
                            remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
        let pipeline_key = registration
            .route
            .pipeline_key()
            .map_err(anyhow::Error::msg)?;
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
            proof_path: None,
            error: None,
            metadata: registration.metadata.clone(),
            request_fingerprint: registration.request_fingerprint.clone(),
            updated_at: now_ts(),
        })
    }

    async fn write_task_workspace(&self, registration: &TaskRegistration) -> Result<()> {
        let pipeline_key = registration
            .route
            .pipeline_key()
            .map_err(anyhow::Error::msg)?;
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
                    remote_tx_hash, proof_path, error, metadata_json, request_fingerprint,
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
                    record.proof_path,
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
    pub proof_path: Option<String>,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_fingerprint: Option<String>,
    pub updated_at: i64,
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
    let expected_pipeline_key = route.pipeline_key().map_err(|err| {
        invalid_runtime_task_row(
            2,
            format!("route '{route_raw}' does not map to a supported pipeline: {err}"),
        )
    })?;
    if pipeline_key != expected_pipeline_key {
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
        proof_path: row.get(13)?,
        error: row.get(14)?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
        request_fingerprint: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn proof_artifact_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProofArtifactRecord> {
    let pipeline_key_raw: String = row.get(2)?;
    let route_raw: String = row.get(3)?;
    let pipeline_key = pipeline_key_raw.parse::<PipelineKey>().map_err(|err| {
        invalid_runtime_task_row(
            2,
            format!("invalid pipeline_key '{pipeline_key_raw}': {err}"),
        )
    })?;
    let route = route_raw.parse::<PipelineRoute>().map_err(|err| {
        invalid_runtime_task_row(3, format!("invalid route '{route_raw}': {err}"))
    })?;
    Ok(ProofArtifactRecord {
        network_pair: row.get(0)?,
        proof_ref: row.get(1)?,
        pipeline_key,
        route,
        proof_path: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn migrate_runtime_schema(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let request_fingerprint_exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info('runtime_tasks') WHERE name = 'request_fingerprint' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if request_fingerprint_exists.is_none() {
        conn.execute(
            "ALTER TABLE runtime_tasks ADD COLUMN request_fingerprint TEXT",
            [],
        )?;
    }
    conn.execute(
        r"
        CREATE UNIQUE INDEX IF NOT EXISTS runtime_tasks_request_fingerprint_uq
        ON runtime_tasks(request_fingerprint)
        WHERE request_fingerprint IS NOT NULL
        ",
        [],
    )?;
    conn.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS proof_artifacts (
            network_pair TEXT NOT NULL,
            proof_ref TEXT NOT NULL,
            pipeline_key TEXT NOT NULL,
            route TEXT NOT NULL,
            proof_path TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(network_pair, proof_ref)
        );
        CREATE INDEX IF NOT EXISTS proof_artifacts_updated_at_idx
        ON proof_artifacts(updated_at);
        ",
    )?;
    Ok(())
}

async fn remove_task_workspace(task_dir: &Path) -> Result<()> {
    if fs::try_exists(task_dir).await? {
        fs::remove_dir_all(task_dir)
            .await
            .with_context(|| format!("failed to remove task workspace {}", task_dir.display()))?;
    }
    Ok(())
}

async fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path {} has no parent", path.display()))?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("failed to create directory {}", parent.display()))?;

    let temp_path = atomic_temp_path(path);
    let result = async {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await
            .with_context(|| format!("failed to create temp file {}", temp_path.display()))?;
        file.write_all(bytes)
            .await
            .with_context(|| format!("failed to write temp file {}", temp_path.display()))?;
        file.sync_all()
            .await
            .with_context(|| format!("failed to sync temp file {}", temp_path.display()))?;
        drop(file);
        fs::rename(&temp_path, path).await.with_context(|| {
            format!(
                "failed to publish temp file {} to {}",
                temp_path.display(),
                path.display()
            )
        })?;
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if result.is_err() {
        let _ = fs::remove_file(&temp_path).await;
    }
    result
}

fn atomic_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    path.with_file_name(format!(".{file_name}.tmp.{}.{}", process::id(), unique))
}

fn safe_path_component(raw: &str) -> String {
    let mut component = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                component.push(char::from(byte));
            }
            _ => {
                write!(&mut component, "%{byte:02x}").expect("writing to String should not fail");
            }
        }
    }
    component
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

#[cfg(test)]
mod tests {
    use super::{
        ExpiredTaskCursor, ProofArtifactRegistration, RunnerStatus, RuntimeManager,
        TaskRegistration, TaskRegistrationOutcome,
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
    async fn runtime_manager_registers_and_loads_task() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-test");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        let registration = TaskRegistration {
            task_id: "task-1".to_string(),
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
    async fn runtime_manager_persists_proof_artifact_paths() -> anyhow::Result<()> {
        let root = unique_root("raiko2-runtime-proof-artifact");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        let proof_path = runtime.proof_artifact_path("taiko_dev/ethereum", "proposal_0xabc");
        runtime
            .upsert_proof_artifact(ProofArtifactRegistration {
                network_pair: "taiko_dev/ethereum".to_string(),
                proof_ref: "proposal_0xabc".to_string(),
                pipeline_key: raiko2_pipeline::PipelineKey::ShastaSp1,
                route: "sp1/local".parse::<PipelineRoute>().expect("parse route"),
                proof_path: proof_path.display().to_string(),
            })
            .await?;

        let artifact = runtime
            .get_proof_artifact("taiko_dev/ethereum", "proposal_0xabc")
            .await?
            .expect("proof artifact");
        assert_eq!(artifact.proof_path, proof_path.display().to_string());
        assert_eq!(artifact.network_pair, "taiko_dev/ethereum");
        assert_eq!(artifact.proof_ref, "proposal_0xabc");

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
                route: "risc0/boundless"
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
                    route: "risc0/boundless"
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
        assert_eq!(shared[0].task_id, "task-public-a");
        assert_eq!(shared[1].task_id, "task-public-b");

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
                    remote_tx_hash, proof_path, error, metadata_json, updated_at
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
    async fn runtime_manager_migrates_request_fingerprint_column() -> anyhow::Result<()> {
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
                    proof_path TEXT,
                    error TEXT,
                    metadata_json TEXT,
                    updated_at INTEGER NOT NULL
                );
                ",
            )?;
            Ok(())
        })
        .await?;
        drop(conn);

        let runtime = RuntimeManager::new(root.clone())?;
        let _ = runtime.list_tasks().await?;

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
                route: "risc0/boundless"
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
                route: "risc0/boundless"
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
                route: "risc0/boundless"
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
                        route: "risc0/boundless"
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
