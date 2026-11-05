//! Local SP1 proving on NVIDIA GPUs (`cuda` feature).
//!
//! SP1 6.x proves on CUDA through `sp1-gpu-server`, a separate process that `sp1-sdk` starts per
//! device and talks to over `/tmp/sp1-cuda-<id>.sock`. The server's proving keys live on the GPU,
//! so the client (which owns the server process) and each program's key are cached instead of
//! being rebuilt per proof.
//!
//! Client lifecycle, per device:
//! - Starts are single-flight: the first proof starts the server while concurrent first users wait.
//! - The pinned `sp1-cuda` client never reconnects or restarts its server, has no request
//!   timeout, and shares one process-wide connection per device while anything (including
//!   asynchronous key cleanup tasks) still references it. A client is therefore retired when:
//!   a request fails at the transport level; the server reports errors on
//!   [`MAX_CONSECUTIVE_SERVER_ERRORS`] requests in a row; the watchdog finds its server process
//!   gone between proofs; or a setup/proof exceeds `sp1.cuda_timeout_secs`, in which case the
//!   watchdog also kills the hung server so the blocked request returns.
//! - A retired or failed device is reported by [`cuda_device_failures`] (surfaced by `/ready`),
//!   proofs on it fail fast, and a supervisor thread restarts it with exponential backoff,
//!   independent of proof traffic. A replacement is installed only after it set up every program
//!   the device has served, which warms the keys and proves it talks to a live server rather than
//!   a reused dead connection. A program whose setup fails for its own reasons (for example a
//!   verifying-key mismatch) is dropped from that warm set instead of blocking the device.
//!
//! `sp1-gpu-server` does not receive the SP1 cycle limit (its protocol carries only the proof
//! nonce), so every GPU proof is preceded by a cycle-bounded CPU execution of the same input.
//!
//! `sp1-sdk` resolves the server binary at `$HOME/.sp1/bin/sp1-gpu-server`. `Dockerfile.cuda`
//! pre-installs the pinned binary; a missing or mismatched binary would make `sp1-sdk` try to
//! stop servers through `systemctl` and download one, which fails in the image.

mod server_process;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use raiko2_primitives::{RaikoError, RaikoResult};
use sp1_sdk::{
    ProvingKey as _, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    blocking::{
        CudaProver as BlockingCudaProver, ProveRequest as _, Prover as _,
        ProverClient as BlockingProverClient,
    },
};
use tokio::runtime::Handle;
use tracing::{error, info, warn};

use super::sp1_vk_digest;
use crate::sp1_config::DEFAULT_CUDA_TIMEOUT_SECS;

type CudaProvingKey = <BlockingCudaProver as sp1_sdk::blocking::Prover>::ProvingKey;

const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
const RESTART_BACKOFF_MAX: Duration = Duration::from_mins(1);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
/// Server-reported errors in a row after which the client is treated as broken.
const MAX_CONSECUTIVE_SERVER_ERRORS: u32 = 3;

/// One CUDA device: the `sp1-gpu-server` client plus the proving keys already set up on it.
pub(super) struct Sp1CudaDevice {
    device_id: u32,
    generation: u64,
    timeout: Duration,
    prover: BlockingCudaProver,
    /// Proving keys keyed by SP1 verifying-key digest.
    keys: Mutex<HashMap<String, CudaProvingKey>>,
    /// One GPU request at a time. sp1-cuda serializes requests on its connection anyway; taking
    /// the turn here first lets the watchdog deadline cover only time spent at the server, never
    /// time spent queued behind other proofs.
    gpu_turn: Mutex<()>,
    consecutive_server_errors: AtomicU32,
    /// Whether the watchdog has seen this client's server process. A server is only reported as
    /// exited after it was seen, so a process the `/proc` lookup cannot match never causes a
    /// restart loop.
    server_seen: AtomicBool,
}

#[derive(Default)]
struct DeviceSlot {
    current: Option<Arc<Sp1CudaDevice>>,
    /// Start or runtime failure; set while no healthy client is installed.
    failure: Option<String>,
    supervising: bool,
    /// Programs set up successfully on this device; set up again on every replacement client.
    programs: HashMap<String, SP1ProvingKey>,
    /// Latest configured per-request deadline, used for replacement setup.
    timeout: Option<Duration>,
}

/// A setup or proof request currently waiting on a server.
struct InFlight {
    device_id: u32,
    generation: u64,
    operation: &'static str,
    deadline: Instant,
}

static SLOTS: LazyLock<Mutex<HashMap<u32, DeviceSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static START_LOCKS: LazyLock<Mutex<HashMap<u32, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static IN_FLIGHT: LazyLock<Mutex<HashMap<u64, InFlight>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static WATCHDOG: OnceLock<()> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// CUDA devices whose `sp1-gpu-server` failed, hung, or died and has not been replaced yet.
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
pub(super) fn device(device_id: u32, timeout: Duration) -> RaikoResult<Arc<Sp1CudaDevice>> {
    ensure_watchdog();
    lock(&SLOTS).entry(device_id).or_default().timeout = Some(timeout);
    if let Some(device) = installed(device_id)? {
        return Ok(device);
    }

    let start_lock = start_lock(device_id);
    let _start = lock(&start_lock);
    if let Some(device) = installed(device_id)? {
        return Ok(device);
    }
    match start_device(device_id, timeout) {
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
fn start_device(device_id: u32, timeout: Duration) -> Result<Arc<Sp1CudaDevice>, String> {
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
        timeout,
        prover,
        keys: Mutex::new(HashMap::new()),
        gpu_turn: Mutex::new(()),
        consecutive_server_errors: AtomicU32::new(0),
        server_seen: AtomicBool::new(false),
    });

    let programs: Vec<(String, SP1ProvingKey)> = lock(&SLOTS)
        .get(&device_id)
        .map(|slot| {
            slot.programs
                .iter()
                .map(|(vk, pk)| (vk.clone(), pk.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (vk, pk) in &programs {
        match device.setup_key(pk) {
            Ok(_) => {}
            Err(SetupError::Client(err)) if is_server_connection_failure(&err) => {
                return Err(format!(
                    "replacement sp1-gpu-server on CUDA device {device_id} is not usable: {err}"
                ));
            }
            // A failure specific to this program must not keep the whole device down.
            Err(SetupError::Client(err) | SetupError::Program(err)) => {
                warn!(
                    device_id,
                    vkey_hash = %vk,
                    %err,
                    "Dropping SP1 program from the CUDA warm set after a setup failure"
                );
                if let Some(slot) = lock(&SLOTS).get_mut(&device_id) {
                    slot.programs.remove(vk);
                }
            }
        }
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
        let timeout = lock(&SLOTS)
            .get(&device_id)
            .and_then(|slot| slot.timeout)
            .unwrap_or(Duration::from_secs(DEFAULT_CUDA_TIMEOUT_SECS));
        let result = start_device(device_id, timeout);

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

/// Retires `generation` of `device_id` if it is still installed, and starts a restart.
fn retire_generation(device_id: u32, generation: u64, reason: &str) {
    {
        let mut slots = lock(&SLOTS);
        let slot = slots.entry(device_id).or_default();
        let Some(retired) = slot
            .current
            .take_if(|current| current.generation == generation)
        else {
            return;
        };
        // Callers run inside the runtime and usually still hold the client, so releasing the
        // slot's reference here is safe.
        drop(retired);
    }
    warn!(
        device_id,
        reason, "Retiring SP1 CUDA client; restarting sp1-gpu-server in the background"
    );
    fail_and_supervise(device_id, reason.to_string());
}

/// Starts the watchdog thread once, from a thread that is inside the Tokio runtime.
fn ensure_watchdog() {
    WATCHDOG.get_or_init(|| {
        let Ok(handle) = Handle::try_current() else {
            warn!("No Tokio runtime for the SP1 CUDA watchdog; hung servers are not detected");
            return;
        };
        if let Err(err) = std::thread::Builder::new()
            .name("sp1-cuda-watchdog".to_string())
            .spawn(move || watchdog(&handle))
        {
            error!(%err, "Failed to spawn the SP1 CUDA watchdog");
        }
    });
}

/// Kills servers whose requests passed their deadline and retires servers that exited.
///
/// Runs on a detached thread so it never blocks runtime shutdown.
fn watchdog(handle: &Handle) {
    let _runtime = handle.enter();
    loop {
        std::thread::sleep(WATCHDOG_INTERVAL);
        for request in take_expired_requests(Instant::now()) {
            let reason = format!(
                "SP1 CUDA {} on device {} exceeded sp1.cuda_timeout_secs; killed the hung sp1-gpu-server",
                request.operation, request.device_id
            );
            error!(device_id = request.device_id, "{reason}");
            retire_generation(request.device_id, request.generation, &reason);
            // Killing the server closes its socket, so the blocked request returns an error.
            if server_process::kill_servers(request.device_id) == 0 {
                warn!(
                    device_id = request.device_id,
                    "No sp1-gpu-server process found to kill; the hung request may stay blocked"
                );
            }
        }
        if server_process::supported() {
            for device in installed_devices() {
                let running = !server_process::server_pids(device.device_id).is_empty();
                if running {
                    device.server_seen.store(true, Ordering::Relaxed);
                } else if device.server_seen.load(Ordering::Relaxed) {
                    retire_generation(
                        device.device_id,
                        device.generation,
                        &format!("sp1-gpu-server for CUDA device {} exited", device.device_id),
                    );
                }
            }
        }
    }
}

fn take_expired_requests(now: Instant) -> Vec<InFlight> {
    let mut in_flight = lock(&IN_FLIGHT);
    let expired: Vec<u64> = in_flight
        .iter()
        .filter(|(_, request)| request.deadline <= now)
        .map(|(id, _)| *id)
        .collect();
    expired
        .into_iter()
        .filter_map(|id| in_flight.remove(&id))
        .collect()
}

fn installed_devices() -> Vec<Arc<Sp1CudaDevice>> {
    lock(&SLOTS)
        .values()
        .filter_map(|slot| slot.current.clone())
        .collect()
}

/// Registers a request with the watchdog until dropped.
struct WatchedRequest(u64);

impl WatchedRequest {
    fn new(device: &Sp1CudaDevice, operation: &'static str) -> Self {
        let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        lock(&IN_FLIGHT).insert(
            id,
            InFlight {
                device_id: device.device_id,
                generation: device.generation,
                operation,
                deadline: Instant::now() + device.timeout,
            },
        );
        Self(id)
    }
}

impl Drop for WatchedRequest {
    fn drop(&mut self) {
        lock(&IN_FLIGHT).remove(&self.0);
    }
}

/// Why setting up a program on the GPU server failed.
enum SetupError {
    /// The `sp1-cuda` client failed: transport or server-reported.
    Client(String),
    /// The server answered, but the key does not match the checked-in verifying key.
    Program(String),
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
        // Gas and deferred-proof checks do not affect the cycle count; the server checks
        // deferred proofs while proving.
        self.prover
            .execute(pk.elf().clone(), stdin.clone())
            .cycle_limit(cycle_limit)
            .calculate_gas(false)
            .deferred_proof_verification(false)
            .run()
            .map_err(|err| {
                format!(
                    "SP1 execution before CUDA proving failed (cycle limit {cycle_limit}): {err}"
                )
            })?;

        let cuda_pk = self.proving_key(pk)?;
        let proof = {
            let _turn = lock(&self.gpu_turn);
            let _watched = WatchedRequest::new(self, "prove");
            self.prover.prove(&cuda_pk, stdin).mode(proof_mode).run()
        };
        match proof {
            Ok(proof) => {
                self.consecutive_server_errors.store(0, Ordering::Relaxed);
                Ok(proof)
            }
            Err(err) => Err(self.client_error("prove", &err.to_string())),
        }
    }

    /// Returns the GPU-resident proving key for `pk`, running SP1 setup on first use.
    fn proving_key(&self, pk: &SP1ProvingKey) -> Result<CudaProvingKey, String> {
        let vk = sp1_vk_digest(pk.verifying_key());
        if let Some(key) = lock(&self.keys).get(&vk) {
            return Ok(key.clone());
        }
        match self.setup_key(pk) {
            Ok(key) => {
                // Only programs that set up successfully join the warm set used on restart.
                lock(&SLOTS)
                    .entry(self.device_id)
                    .or_default()
                    .programs
                    .entry(vk)
                    .or_insert_with(|| pk.clone());
                Ok(key)
            }
            Err(SetupError::Client(err)) => Err(self.client_error("setup", &err)),
            // A local verifying-key check, not a server failure: fail only this proof.
            Err(SetupError::Program(err)) => Err(err),
        }
    }

    /// Sets up `pk` on the GPU server and caches the key. Does not retire the client.
    ///
    /// The key is derived from the ELF on the GPU server; its verifying key must match the
    /// checked-in verifying key that raiko2 registers and verifies against.
    fn setup_key(&self, pk: &SP1ProvingKey) -> Result<CudaProvingKey, SetupError> {
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
        let key = {
            let _turn = lock(&self.gpu_turn);
            let _watched = WatchedRequest::new(self, "setup");
            self.prover.setup(pk.elf().clone())
        }
        .map_err(|err| SetupError::Client(err.to_string()))?;
        let actual_vk = sp1_vk_digest(key.verifying_key());
        if actual_vk != expected_vk {
            return Err(SetupError::Program(format!(
                "SP1 CUDA setup produced verifying key {actual_vk}, expected {expected_vk}; \
                 the guest ELF and verifying key artifacts are out of sync"
            )));
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

    /// Formats a client error and retires this client when the server looks broken.
    fn client_error(&self, operation: &str, error: &str) -> String {
        let message = format!(
            "SP1 CUDA {operation} failed on device {}: {error}",
            self.device_id
        );
        if is_server_connection_failure(error) {
            retire_generation(self.device_id, self.generation, &message);
        } else {
            let in_a_row = self
                .consecutive_server_errors
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            if in_a_row >= MAX_CONSECUTIVE_SERVER_ERRORS {
                retire_generation(
                    self.device_id,
                    self.generation,
                    &format!("{in_a_row} consecutive server errors, last: {message}"),
                );
            }
        }
        message
    }
}

/// Whether an `sp1-cuda` client error means the server connection is unusable.
///
/// `CudaClientError` does not expose its variant, so this matches its 6.8.1 `Display` text.
/// sp1-cuda reports both server `InternalError` and `ProverError` responses as
/// "The server returned an internal error"; those keep the client (the server is alive), and
/// every other error, including unknown ones, retires it so a dead server is never reused.
fn is_server_connection_failure(error: &str) -> bool {
    const SERVER_REPORTED: [&str; 2] = [
        "The server returned an internal error",
        "The server returned a prover error",
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
    use std::io::{Error as IoError, ErrorKind};
    use std::time::{Duration, Instant};

    use sp1_cuda::CudaClientError;

    use super::{
        IN_FLIGHT, InFlight, is_server_connection_failure, lock, panic_message,
        take_expired_requests,
    };

    #[test]
    fn transport_failures_retire_the_client() {
        for error in [
            CudaClientError::Connect(IoError::from(ErrorKind::NotFound)),
            CudaClientError::Read(IoError::from(ErrorKind::UnexpectedEof)),
            CudaClientError::Write(IoError::from(ErrorKind::BrokenPipe)),
            CudaClientError::UnexpectedResponse("Setup"),
            CudaClientError::Unexpected("something new".to_string()),
        ] {
            let message = error.to_string();
            assert!(is_server_connection_failure(&message), "{message}");
        }
    }

    #[test]
    fn server_reported_failures_keep_the_client() {
        for error in [
            CudaClientError::ServerError("out of memory".to_string()),
            CudaClientError::ProverError("bad witness".to_string()),
        ] {
            let message = error.to_string();
            assert!(!is_server_connection_failure(&message), "{message}");
        }
    }

    #[test]
    fn expired_requests_are_taken_once() {
        let now = Instant::now();
        {
            let mut in_flight = lock(&IN_FLIGHT);
            in_flight.insert(
                u64::MAX,
                InFlight {
                    device_id: 9,
                    generation: 1,
                    operation: "prove",
                    deadline: now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
                },
            );
            in_flight.insert(
                u64::MAX - 1,
                InFlight {
                    device_id: 9,
                    generation: 1,
                    operation: "setup",
                    deadline: now + Duration::from_hours(1),
                },
            );
        }

        let expired = take_expired_requests(now);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].operation, "prove");
        assert!(take_expired_requests(now).is_empty());
        lock(&IN_FLIGHT).remove(&(u64::MAX - 1));
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
