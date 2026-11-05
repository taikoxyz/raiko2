//! Local SP1 proving on NVIDIA GPUs (`cuda` feature).
//!
//! SP1 6.x proves on CUDA through `sp1-gpu-server`, a separate process that `sp1-sdk` starts per
//! device and talks to over `/tmp/sp1-cuda-<id>.sock`. The server's proving keys live on the GPU,
//! so the client (which owns the server process) and each program's key are cached instead of
//! being rebuilt per proof.
//!
//! Client lifecycle, per device:
//! - Starts are single-flight: the first proof starts the server while concurrent first users wait.
//! - The pinned `sp1-cuda` client never reconnects or restarts its server, and it shares one
//!   process-wide connection per device for as long as anything (including asynchronous key
//!   cleanup tasks) still references it. On a transport failure the client is retired, the device
//!   is reported by [`cuda_device_failures`] (surfaced by `/ready`), and proofs on it fail fast.
//! - A supervisor thread then restarts it with exponential backoff, independent of proof traffic.
//!   A replacement is installed only after it set up every program the device has served, which
//!   both warms the keys and proves it talks to a live server: a replacement that silently reused
//!   the dead connection fails that setup and is discarded, and the next attempt retries.
//! - A failed first start is retried by the same supervisor.
//!
//! `sp1-gpu-server` does not receive the SP1 cycle limit (its protocol carries only the proof
//! nonce), so every GPU proof is preceded by a cycle-bounded CPU execution of the same input.
//!
//! `sp1-sdk` resolves the server binary at `$HOME/.sp1/bin/sp1-gpu-server` and downloads it from
//! the SP1 GitHub release when missing. `Dockerfile.cuda` pre-installs the pinned binary so the
//! runtime never downloads code.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use raiko2_primitives::{RaikoError, RaikoResult};
use sp1_sdk::{
    ProvingKey as _, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    blocking::{
        CudaProver as BlockingCudaProver, ProveRequest as _, Prover as _,
        ProverClient as BlockingProverClient,
    },
};
use tokio::runtime::Handle;
use tracing::{info, warn};

use super::sp1_vk_digest;

type CudaProvingKey = <BlockingCudaProver as sp1_sdk::blocking::Prover>::ProvingKey;

const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
const RESTART_BACKOFF_MAX: Duration = Duration::from_mins(1);

/// One CUDA device: the `sp1-gpu-server` client plus the proving keys already set up on it.
pub(super) struct Sp1CudaDevice {
    device_id: u32,
    generation: u64,
    prover: BlockingCudaProver,
    /// Proving keys keyed by SP1 verifying-key digest.
    keys: Mutex<HashMap<String, CudaProvingKey>>,
}

#[derive(Default)]
struct DeviceSlot {
    current: Option<Arc<Sp1CudaDevice>>,
    /// Start or transport failure; set while no healthy client is installed.
    failure: Option<String>,
    supervising: bool,
    /// Programs requested on this device, set up again on every replacement client.
    programs: HashMap<String, SP1ProvingKey>,
}

static SLOTS: LazyLock<Mutex<HashMap<u32, DeviceSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static START_LOCKS: LazyLock<Mutex<HashMap<u32, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// CUDA devices whose `sp1-gpu-server` failed to start or died and has not been replaced yet.
#[must_use]
pub fn cuda_device_failures() -> Vec<(u32, String)> {
    let mut failures: Vec<(u32, String)> = lock(&SLOTS)
        .iter()
        .filter(|(_, slot)| slot.current.is_none())
        .filter_map(|(id, slot)| slot.failure.clone().map(|failure| (*id, failure)))
        .collect();
    failures.sort_unstable_by_key(|(id, _)| *id);
    failures
}

/// Returns the client for `device_id`, starting `sp1-gpu-server` on first use.
///
/// Must run on a blocking thread inside the Tokio runtime: `sp1-sdk` drives the server through
/// its own runtime (and panics inside an async context), while dropping a client or key spawns a
/// cleanup task on the current runtime.
pub(super) fn device(device_id: u32) -> RaikoResult<Arc<Sp1CudaDevice>> {
    if let Some(device) = installed(device_id)? {
        return Ok(device);
    }

    let start_lock = start_lock(device_id);
    let _start = lock(&start_lock);
    if let Some(device) = installed(device_id)? {
        return Ok(device);
    }
    match start_device(device_id) {
        Ok(device) => {
            let mut slots = lock(&SLOTS);
            let slot = slots.entry(device_id).or_default();
            slot.current = Some(Arc::clone(&device));
            slot.failure = None;
            Ok(device)
        }
        Err(failure) => {
            fail_and_supervise(device_id, failure.clone());
            Err(RaikoError::Guest(failure))
        }
    }
}

/// The installed client, an error while the device is down, or `None` before its first start.
fn installed(device_id: u32) -> RaikoResult<Option<Arc<Sp1CudaDevice>>> {
    let slots = lock(&SLOTS);
    let Some(slot) = slots.get(&device_id) else {
        return Ok(None);
    };
    if let Some(device) = &slot.current {
        return Ok(Some(Arc::clone(device)));
    }
    slot.failure.as_ref().map_or(Ok(None), |failure| {
        Err(RaikoError::Guest(format!(
            "SP1 CUDA device {device_id} is unavailable and restarting in the background: {failure}"
        )))
    })
}

fn start_lock(device_id: u32) -> Arc<Mutex<()>> {
    Arc::clone(lock(&START_LOCKS).entry(device_id).or_default())
}

/// Starts a client and sets up every program the device has served. Call with the start lock.
fn start_device(device_id: u32) -> Result<Arc<Sp1CudaDevice>, String> {
    info!(device_id, "Starting SP1 CUDA prover (sp1-gpu-server)");
    // `sp1-sdk` panics when the server cannot be started or reached.
    let prover = catch_unwind(AssertUnwindSafe(|| {
        BlockingProverClient::builder()
            .cuda()
            .with_device_id(device_id)
            .build()
    }))
    .map_err(|panic| {
        format!(
            "failed to start sp1-gpu-server on CUDA device {device_id}: {}",
            panic_message(panic.as_ref())
        )
    })?;
    let device = Arc::new(Sp1CudaDevice {
        device_id,
        generation: GENERATION.fetch_add(1, Ordering::Relaxed),
        prover,
        keys: Mutex::new(HashMap::new()),
    });

    let programs: Vec<SP1ProvingKey> = lock(&SLOTS)
        .get(&device_id)
        .map(|slot| slot.programs.values().cloned().collect())
        .unwrap_or_default();
    for pk in &programs {
        device.setup_key(pk).map_err(|err| {
            format!("replacement sp1-gpu-server on CUDA device {device_id} is not usable: {err}")
        })?;
    }
    Ok(device)
}

/// Records a failure and makes sure a supervisor is restarting the device.
fn fail_and_supervise(device_id: u32, failure: String) {
    let mut slots = lock(&SLOTS);
    let slot = slots.entry(device_id).or_default();
    slot.failure = Some(failure);
    if slot.supervising {
        return;
    }
    // The supervisor drops failed clients, whose cleanup spawns onto the Tokio runtime.
    let Ok(handle) = Handle::try_current() else {
        warn!(
            device_id,
            "No Tokio runtime to restart sp1-gpu-server; the device stays unavailable"
        );
        return;
    };
    let spawned = std::thread::Builder::new()
        .name(format!("sp1-cuda-supervisor-{device_id}"))
        .spawn(move || supervise(device_id, &handle));
    match spawned {
        Ok(_) => slot.supervising = true,
        Err(err) => warn!(device_id, %err, "Failed to spawn the SP1 CUDA supervisor"),
    }
}

/// Restarts the device with exponential backoff until a usable client is installed.
///
/// Runs on a detached thread so it never blocks runtime shutdown. The first delay also gives
/// in-flight users of a retired client time to release it.
fn supervise(device_id: u32, handle: &Handle) {
    let _runtime = handle.enter();
    let mut backoff = RESTART_BACKOFF_INITIAL;
    loop {
        std::thread::sleep(backoff);
        let start_lock = start_lock(device_id);
        let _start = lock(&start_lock);
        let result = start_device(device_id);

        let mut slots = lock(&SLOTS);
        let slot = slots.entry(device_id).or_default();
        if slot.current.is_some() {
            slot.supervising = false;
            return;
        }
        match result {
            Ok(device) => {
                info!(device_id, "Restarted SP1 CUDA prover");
                slot.current = Some(device);
                slot.failure = None;
                // Cleared together with the install, so a later retirement starts a new supervisor.
                slot.supervising = false;
                return;
            }
            Err(failure) => {
                warn!(device_id, %failure, retry_in = ?backoff, "SP1 CUDA restart failed");
                slot.failure = Some(failure);
                backoff = (backoff * 2).min(RESTART_BACKOFF_MAX);
            }
        }
    }
}

impl Sp1CudaDevice {
    /// Proves `stdin` on this GPU after a cycle-bounded CPU execution of the same input.
    pub(super) fn prove(
        &self,
        pk: &SP1ProvingKey,
        stdin: SP1Stdin,
        proof_mode: SP1ProofMode,
        cycle_limit: u64,
    ) -> Result<SP1ProofWithPublicValues, String> {
        // sp1-gpu-server ignores the request cycle limit, so enforce it before using the GPU.
        self.prover
            .execute(pk.elf().clone(), stdin.clone())
            .cycle_limit(cycle_limit)
            .run()
            .map_err(|err| {
                format!(
                    "SP1 execution before CUDA proving failed (cycle limit {cycle_limit}): {err}"
                )
            })?;

        let cuda_pk = self.proving_key(pk)?;
        self.prover
            .prove(&cuda_pk, stdin)
            .mode(proof_mode)
            .run()
            .map_err(|err| self.client_error("prove", &err.to_string()))
    }

    /// Returns the GPU-resident proving key for `pk`, running SP1 setup on first use.
    fn proving_key(&self, pk: &SP1ProvingKey) -> Result<CudaProvingKey, String> {
        let vk = sp1_vk_digest(pk.verifying_key());
        if let Some(key) = lock(&self.keys).get(&vk) {
            return Ok(key.clone());
        }
        lock(&SLOTS)
            .entry(self.device_id)
            .or_default()
            .programs
            .entry(vk)
            .or_insert_with(|| pk.clone());
        self.setup_key(pk)
            .map_err(|err| self.client_error("setup", &err))
    }

    /// Sets up `pk` on the GPU server and caches the key. Does not retire the client.
    ///
    /// The key is derived from the ELF on the GPU server; its verifying key must match the
    /// checked-in verifying key that raiko2 registers and verifies against.
    fn setup_key(&self, pk: &SP1ProvingKey) -> Result<CudaProvingKey, String> {
        let expected_vk = sp1_vk_digest(pk.verifying_key());
        let mut keys = lock(&self.keys);
        if let Some(key) = keys.get(&expected_vk) {
            return Ok(key.clone());
        }

        info!(
            device_id = self.device_id,
            vkey_hash = %expected_vk,
            "Setting up SP1 program on CUDA device"
        );
        let key = self
            .prover
            .setup(pk.elf().clone())
            .map_err(|err| err.to_string())?;
        let actual_vk = sp1_vk_digest(key.verifying_key());
        if actual_vk != expected_vk {
            return Err(format!(
                "SP1 CUDA setup produced verifying key {actual_vk}, expected {expected_vk}; \
                 the guest ELF and verifying key artifacts are out of sync"
            ));
        }
        keys.insert(expected_vk, key.clone());
        Ok(key)
    }

    pub(super) fn execute(
        &self,
        elf: Vec<u8>,
        stdin: SP1Stdin,
    ) -> Result<(sp1_sdk::SP1PublicValues, sp1_sdk::ExecutionReport), String> {
        self.prover
            .execute(elf.into(), stdin)
            .run()
            .map_err(|err| err.to_string())
    }

    pub(super) fn verify(
        &self,
        proof: &SP1ProofWithPublicValues,
        vk: &sp1_sdk::SP1VerifyingKey,
    ) -> Result<(), String> {
        self.prover
            .verify(proof, vk, None)
            .map_err(|err| err.to_string())
    }

    /// Formats a client error and retires this client when the server connection is broken.
    fn client_error(&self, operation: &str, error: &str) -> String {
        let message = format!(
            "SP1 CUDA {operation} failed on device {}: {error}",
            self.device_id
        );
        if is_server_connection_failure(error) {
            self.retire(&message);
        }
        message
    }

    fn retire(&self, reason: &str) {
        {
            let mut slots = lock(&SLOTS);
            let slot = slots.entry(self.device_id).or_default();
            let Some(retired) = slot
                .current
                .take_if(|current| current.generation == self.generation)
            else {
                return;
            };
            // The caller still holds this client, so releasing the slot's reference here never
            // drops it while the lock is held.
            drop(retired);
        }
        warn!(
            device_id = self.device_id,
            reason, "Retiring SP1 CUDA client; restarting sp1-gpu-server in the background"
        );
        fail_and_supervise(self.device_id, reason.to_string());
    }
}

/// Whether an `sp1-cuda` client error means the server connection is unusable.
///
/// `CudaClientError` does not expose its variant, so this matches the pinned 6.8.1 messages.
/// Errors the live server reported itself (prover/internal errors) keep the client; every other
/// error, including unknown ones, retires it so a dead server is never reused.
fn is_server_connection_failure(error: &str) -> bool {
    const SERVER_REPORTED: [&str; 2] = [
        "The server returned a prover error",
        "The server returned an internal error",
    ];
    !SERVER_REPORTED
        .iter()
        .any(|reported| error.contains(reported))
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "unknown panic".to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{is_server_connection_failure, panic_message};

    #[test]
    fn transport_failures_retire_the_client() {
        for error in [
            "CudaClientError: Failed to connect to the server: Os { code: 2 }",
            "CudaClientError: Failed to read the response: Kind(UnexpectedEof)",
            "CudaClientError: Failed to write the request: Os { code: 32 }",
            "CudaClientError: Failed to deserialize the response: Io(..)",
            "CudaClientError: The server returned an unexpected response: \"Setup\"",
            "something new",
        ] {
            assert!(is_server_connection_failure(error), "{error}");
        }
    }

    #[test]
    fn server_reported_failures_keep_the_client() {
        for error in [
            "CudaClientError: The server returned a prover error: \"cycle limit\"",
            "CudaClientError: The server returned an internal error \n oom",
        ] {
            assert!(!is_server_connection_failure(error), "{error}");
        }
    }

    #[test]
    fn panic_message_reads_string_payloads() {
        let owned: Box<dyn std::any::Any + Send> = Box::new("boom".to_string());
        let borrowed: Box<dyn std::any::Any + Send> = Box::new("bang");
        let other: Box<dyn std::any::Any + Send> = Box::new(7_u8);

        assert_eq!(panic_message(owned.as_ref()), "boom");
        assert_eq!(panic_message(borrowed.as_ref()), "bang");
        assert_eq!(panic_message(other.as_ref()), "unknown panic");
    }
}
