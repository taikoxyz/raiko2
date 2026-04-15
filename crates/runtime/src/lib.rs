#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::OnceCell;

/// Runtime root managed by the host process.
#[derive(Debug, Clone)]
pub struct RuntimeManager {
    root: PathBuf,
    db_path: PathBuf,
    conn: OnceCell<tokio_rusqlite::Connection>,
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
            self.root.join("tmp"),
        ] {
            fs::create_dir_all(&dir)
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
                            updated_at INTEGER NOT NULL
                        );
                        ",
                    )?;
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
    pub fn task_dir(&self, pipeline_key: &str, task_id: &str) -> PathBuf {
        self.root.join("tasks").join(pipeline_key).join(task_id)
    }

    /// # Errors
    ///
    /// Returns an error if the task workspace or metadata cannot be created.
    pub async fn register_task(&self, registration: TaskRegistration) -> Result<RuntimeTaskRecord> {
        let task_dir = self.task_dir(&registration.pipeline_key, &registration.task_id);
        fs::create_dir_all(task_dir.join("logs"))
            .with_context(|| format!("failed to create task workspace {}", task_dir.display()))?;
        fs::write(
            task_dir.join("request.json"),
            serde_json::to_vec_pretty(&registration).context("serialize task registration")?,
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                task_dir.join("request.json").display()
            )
        })?;

        let record = RuntimeTaskRecord {
            task_id: registration.task_id,
            pipeline_key: registration.pipeline_key,
            route: registration.route,
            guest_system: registration.guest_system,
            runner: registration.runner,
            task_kind: registration.task_kind,
            proposal_id: registration.proposal_id,
            proof_ids: registration.proof_ids,
            runner_status: RunnerStatus::Allocated,
            task_dir: task_dir.display().to_string(),
            image_ref: None,
            provider_request_id: None,
            remote_tx_hash: None,
            proof_path: None,
            error: None,
            metadata: registration.metadata,
            updated_at: now_ts(),
        };
        self.upsert_task(&record).await?;
        Ok(record)
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
                    remote_tx_hash, proof_path, error, metadata_json, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
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
                    updated_at = excluded.updated_at
                ",
                params![
                    record.task_id,
                    record.pipeline_key,
                    record.route,
                    record.guest_system,
                    record.runner,
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
                        remote_tx_hash, proof_path, error, metadata_json, updated_at
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
    pub async fn find_task_by_engine_task_id(
        &self,
        engine_task_id: &str,
    ) -> Result<Option<RuntimeTaskRecord>> {
        let mut tasks = self.find_tasks_by_engine_task_id(engine_task_id).await?;
        Ok(tasks.pop())
    }

    /// # Errors
    ///
    /// Returns an error if the matching task records cannot be loaded.
    pub async fn find_tasks_by_engine_task_id(
        &self,
        engine_task_id: &str,
    ) -> Result<Vec<RuntimeTaskRecord>> {
        let conn = self.connection().await?;
        let engine_task_id = engine_task_id.to_string();
        let tasks = conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    r"
                    SELECT
                        task_id, pipeline_key, route, guest_system, runner, task_kind, proposal_id,
                        proof_ids_json, runner_status, task_dir, image_ref, provider_request_id,
                        remote_tx_hash, proof_path, error, metadata_json, updated_at
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
                let mut rows = stmt.query(params![engine_task_id])?;
                let mut matches = Vec::new();
                while let Some(row) = rows.next()? {
                    matches.push(runtime_task_record_from_row(row)?);
                }
                Ok(matches)
            })
            .await
            .context("failed to query runtime task by engine task id")?;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRegistration {
    pub task_id: String,
    pub pipeline_key: String,
    pub route: String,
    pub guest_system: String,
    pub runner: String,
    pub task_kind: String,
    pub proposal_id: Option<u64>,
    #[serde(default)]
    pub proof_ids: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskRecord {
    pub task_id: String,
    pub pipeline_key: String,
    pub route: String,
    pub guest_system: String,
    pub runner: String,
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
    pub updated_at: i64,
}

fn runtime_task_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeTaskRecord> {
    let proof_ids_json: String = row.get(7)?;
    let metadata_json: String = row.get(15)?;
    Ok(RuntimeTaskRecord {
        task_id: row.get(0)?,
        pipeline_key: row.get(1)?,
        route: row.get(2)?,
        guest_system: row.get(3)?,
        runner: row.get(4)?,
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
        updated_at: row.get(16)?,
    })
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
    use super::{RunnerStatus, RuntimeManager, TaskRegistration};
    use std::path::Path;

    #[tokio::test]
    async fn runtime_manager_registers_and_loads_task() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("raiko2-runtime-test-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        let registration = TaskRegistration {
            task_id: "task-1".to_string(),
            pipeline_key: "shasta-risc0-local".to_string(),
            route: "risc0/local".to_string(),
            guest_system: "risc0".to_string(),
            runner: "local".to_string(),
            task_kind: "proposal".to_string(),
            proposal_id: Some(7),
            proof_ids: Vec::new(),
            metadata: serde_json::json!({"proposal_id": 7}),
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
    async fn runtime_manager_finds_task_by_engine_task_id() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "raiko2-runtime-engine-lookup-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        runtime
            .register_task(TaskRegistration {
                task_id: "task-public".to_string(),
                pipeline_key: "shasta-risc0-boundless".to_string(),
                route: "risc0/boundless".to_string(),
                guest_system: "risc0".to_string(),
                runner: "boundless".to_string(),
                task_kind: "hoodi_batch".to_string(),
                proposal_id: Some(7),
                proof_ids: vec!["proposal-proof-task".to_string()],
                metadata: serde_json::json!({
                    "aggregate_task_id": "aggregate-proof-task",
                }),
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
        let root = std::env::temp_dir().join(format!(
            "raiko2-runtime-engine-lookup-all-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        let runtime = RuntimeManager::new(root.clone())?;
        for task_id in ["task-public-a", "task-public-b"] {
            runtime
                .register_task(TaskRegistration {
                    task_id: task_id.to_string(),
                    pipeline_key: "shasta-risc0-boundless".to_string(),
                    route: "risc0/boundless".to_string(),
                    guest_system: "risc0".to_string(),
                    runner: "boundless".to_string(),
                    task_kind: "hoodi_batch".to_string(),
                    proposal_id: Some(7),
                    proof_ids: vec!["shared-proposal-proof-task".to_string()],
                    metadata: serde_json::json!({
                        "aggregate_task_id": "shared-aggregate-proof-task",
                    }),
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
}
