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
//!   timeout, and hands out its existing per-device connection while anything (including
//!   asynchronous key cleanup tasks) still references it. Each client therefore records the
//!   server process it started (via `/proc` on Linux). A start must produce a new server process;
//!   otherwise sp1-cuda reused an old connection and the client is discarded. Before a restart
//!   the previous server is killed, so stale users of the old connection fail fast and release it.
//! - A client is retired when: a request fails at the transport level; the server reports errors
//!   on [`MAX_CONSECUTIVE_SERVER_ERRORS`] requests in a row; its server process exits; or a
//!   setup/proof exceeds `sp1.cuda_timeout_secs` at the server, in which case the watchdog kills
//!   that exact server process so the blocked request returns. GPU requests take turns per device
//!   and the deadline starts only once a request has its turn.
//! - A retired or failed device is reported by [`cuda_device_failures`] (surfaced by `/ready`),
//!   proofs on it fail fast, and a supervisor thread restarts it with exponential backoff,
//!   independent of proof traffic. A replacement is installed only after it set up every program
//!   the device has served. A program whose setup fails for its own reasons (a verifying-key
//!   mismatch) is dropped from that set; any server failure keeps the device restarting.
//! - Before a restart the previous server is killed and waited on (bounded), so the new client
//!   cannot reach the old listener and the old GPU memory is released. Server starts have their
//!   own watchdog deadline.
//! - [`shutdown_cuda_devices`] stops the supervisors and watchdog, kills the servers, and releases
//!   the cached clients at process shutdown; a start still running then is not installed.
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
use crate::sp1_config::{DEFAULT_CUDA_TIMEOUT_SECS, MAX_CUDA_TIMEOUT_SECS};

type CudaProvingKey = <BlockingCudaProver as sp1_sdk::blocking::Prover>::ProvingKey;

const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
const RESTART_BACKOFF_MAX: Duration = Duration::from_mins(1);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
/// Server-reported errors in a row after which the client is treated as broken.
const MAX_CONSECUTIVE_SERVER_ERRORS: u32 = 3;
const SHUTTING_DOWN: &str = "SP1 CUDA proving is shutting down";
/// How long a restart waits for killed servers to exit (releasing their socket and GPU memory).
const PREVIOUS_SERVER_EXIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Deadline for starting `sp1-gpu-server` and connecting to it.
const START_TIMEOUT: Duration = Duration::from_mins(5);

/// One CUDA device: the `sp1-gpu-server` client plus the proving keys already set up on it.
pub(super) struct Sp1CudaDevice {
    device_id: u32,
    generation: u64,
    timeout: Duration,
    /// The `sp1-gpu-server` process this client started; `None` where `/proc` is unavailable.
    server_pid: Option<i32>,
    prover: BlockingCudaProver,
    /// Proving keys keyed by SP1 verifying-key digest.
    keys: Mutex<HashMap<String, CudaProvingKey>>,
    /// One GPU request at a time. sp1-cuda serializes requests on its connection anyway; taking
    /// the turn here first lets the watchdog deadline cover only time spent at the server, never
    /// time spent queued behind other proofs.
    gpu_turn: Mutex<()>,
    consecutive_server_errors: AtomicU32,
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

/// A server start, or a setup or proof request currently being served by a server.
struct InFlight {
    device_id: u32,
    /// Client generation; `None` for a server start, which has no client yet.
    generation: Option<u64>,
    server_pid: Option<i32>,
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
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
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

/// Stops SP1 GPU proving at process shutdown, after proof workers have stopped.
///
/// Stops the supervisors and watchdog, kills every `sp1-gpu-server` this process started (so a
/// blocked request returns and the server's pooled GPU memory is released even outside a
/// container), and drops the cached clients. Run it on a blocking thread inside the runtime: the
/// SDK's client and key cleanup spawns tasks on it.
pub fn shutdown_cuda_devices() {
    SHUTDOWN.store(true, Ordering::SeqCst);
    let (device_ids, devices): (Vec<u32>, Vec<Arc<Sp1CudaDevice>>) = {
        let mut slots = lock(&SLOTS);
        let device_ids = slots.keys().copied().collect();
        let devices = slots
            .values_mut()
            .filter_map(|slot| {
                slot.failure = Some(SHUTTING_DOWN.to_string());
                slot.current.take()
            })
            .collect();
        (device_ids, devices)
    };
    for device_id in device_ids {
        let killed = server_process::kill_servers(device_id);
        if !killed.is_empty() {
            info!(device_id, ?killed, "Stopped sp1-gpu-server");
        }
    }
    drop(devices);
}

/// Returns the client for `device_id`, starting `sp1-gpu-server` on first use.
///
/// Must run on a blocking thread inside the Tokio runtime: `sp1-sdk` drives the server through
/// its own runtime (and panics inside an async context), while dropping a client or key spawns a
/// cleanup task on the current runtime.
pub(super) fn device(device_id: u32, timeout: Duration) -> RaikoResult<Arc<Sp1CudaDevice>> {
    if SHUTDOWN.load(Ordering::SeqCst) {
        return Err(RaikoError::Guest(SHUTTING_DOWN.to_string()));
    }
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
    let started = start_device(device_id, timeout);
    let mut slots = lock(&SLOTS);
    let slot = slots.entry(device_id).or_default();
    match started {
        // Shutdown began while this start was running: never install, stop its server.
        Ok(device) if SHUTDOWN.load(Ordering::SeqCst) => {
            drop(slots);
            server_process::kill_servers(device_id);
            drop(device);
            Err(RaikoError::Guest(SHUTTING_DOWN.to_string()))
        }
        Ok(device) => {
            slot.current = Some(Arc::clone(&device));
            slot.failure = None;
            Ok(device)
        }
        Err(failure) => {
            mark_failed(slot, device_id, failure.clone());
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

/// Starts a fresh `sp1-gpu-server` client and sets up every program the device has served.
/// Call with the device's start lock held.
fn start_device(device_id: u32, timeout: Duration) -> Result<Arc<Sp1CudaDevice>, String> {
    let tracking = server_process::supported();
    let previous = if tracking {
        // A previous server is dead, hung, or being replaced. Killing it makes stale users of its
        // connection fail fast and release it, so sp1-cuda starts a new server below. Wait for it
        // to exit, so the new client cannot connect to the old listener and the old server's GPU
        // memory is released first.
        let killed = server_process::kill_servers(device_id);
        let running = server_process::wait_exited(&killed, PREVIOUS_SERVER_EXIT_TIMEOUT);
        if !running.is_empty() {
            return Err(format!(
                "previous sp1-gpu-server on CUDA device {device_id} (pids {running:?}) did not exit \
                 within {PREVIOUS_SERVER_EXIT_TIMEOUT:?} after SIGKILL"
            ));
        }
        server_process::server_pids(device_id)
    } else {
        Vec::new()
    };

    info!(device_id, "Starting SP1 CUDA prover (sp1-gpu-server)");
    // `sp1-sdk` panics when the server cannot be started or reached. A start that hangs is
    // reported and its server killed by the watchdog.
    let prover = {
        let _watched = WatchedRequest::start(device_id);
        catch_unwind(AssertUnwindSafe(|| {
            BlockingProverClient::builder()
                .cuda()
                .with_device_id(device_id)
                .build()
        }))
    }
    .map_err(|panic| {
        format!(
            "failed to start sp1-gpu-server on CUDA device {device_id}: {}",
            panic_message(panic.as_ref())
        )
    })?;
    let server_pid = if tracking {
        let started = server_process::server_pids(device_id)
            .into_iter()
            .filter(|pid| !previous.contains(pid))
            .collect::<Vec<_>>();
        Some(single_new_server(device_id, &started)?)
    } else {
        None
    };
    let device = Arc::new(Sp1CudaDevice {
        device_id,
        generation: GENERATION.fetch_add(1, Ordering::Relaxed),
        timeout,
        server_pid,
        prover,
        keys: Mutex::new(HashMap::new()),
        gpu_turn: Mutex::new(()),
        consecutive_server_errors: AtomicU32::new(0),
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
            // Any server failure keeps the device restarting with the full warm set.
            Err(SetupError::Client(err)) => {
                return Err(format!(
                    "replacement sp1-gpu-server on CUDA device {device_id} failed setup: {err}"
                ));
            }
            // A mismatch specific to this program must not keep the whole device down.
            Err(SetupError::Program(err)) => {
                warn!(
                    device_id,
                    vkey_hash = %vk,
                    %err,
                    "Dropping SP1 program from the CUDA warm set after a setup mismatch"
                );
                if let Some(slot) = lock(&SLOTS).get_mut(&device_id) {
                    slot.programs.remove(vk);
                }
            }
        }
    }
    Ok(device)
}

/// The one `sp1-gpu-server` process a start produced. None means sp1-cuda handed back a
/// connection that was still referenced instead of starting a server.
fn single_new_server(device_id: u32, started: &[i32]) -> Result<i32, String> {
    match started {
        [pid] => Ok(*pid),
        [] => Err(format!(
            "no new sp1-gpu-server started on CUDA device {device_id}; sp1-cuda reused a previous \
             connection that is still referenced"
        )),
        pids => Err(format!(
            "found several new sp1-gpu-server processes for CUDA device {device_id}: {pids:?}"
        )),
    }
}

/// Records a failure and makes sure a supervisor is restarting the device. Call with the slot's
/// `SLOTS` lock held, so the failure is visible in the same step that removed the client.
fn mark_failed(slot: &mut DeviceSlot, device_id: u32, failure: String) {
    slot.failure = Some(failure);
    if slot.supervising || SHUTDOWN.load(Ordering::SeqCst) {
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
    let _supervising = SupervisorGuard(device_id);
    let mut backoff = RESTART_BACKOFF_INITIAL;
    loop {
        std::thread::sleep(backoff);
        if SHUTDOWN.load(Ordering::SeqCst) {
            return;
        }
        let start_lock = start_lock(device_id);
        let _start = lock(&start_lock);
        let timeout = lock(&SLOTS)
            .get(&device_id)
            .and_then(|slot| slot.timeout)
            .unwrap_or(Duration::from_secs(DEFAULT_CUDA_TIMEOUT_SECS));
        let result = start_device(device_id, timeout);

        let mut slots = lock(&SLOTS);
        let slot = slots.entry(device_id).or_default();
        if slot.current.is_some() || SHUTDOWN.load(Ordering::SeqCst) {
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

/// Clears `supervising` if a supervisor panics, so a later failure starts a new one.
struct SupervisorGuard(u32);

impl Drop for SupervisorGuard {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(slot) = lock(&SLOTS).get_mut(&self.0)
        {
            slot.supervising = false;
        }
    }
}

/// Retires `generation` of `device_id` if it is still installed, and starts a restart.
fn retire_generation(device_id: u32, generation: u64, reason: &str) {
    let retired = {
        let mut slots = lock(&SLOTS);
        let slot = slots.entry(device_id).or_default();
        let Some(retired) = slot
            .current
            .take_if(|current| current.generation == generation)
        else {
            return;
        };
        warn!(
            device_id,
            reason, "Retiring SP1 CUDA client; restarting sp1-gpu-server in the background"
        );
        mark_failed(slot, device_id, reason.to_string());
        retired
    };
    // Released outside the lock; callers run inside the runtime, as the SDK cleanup requires.
    drop(retired);
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

/// Kills servers whose requests passed their deadline and retires clients whose server exited.
///
/// Runs on a detached thread so it never blocks runtime shutdown.
fn watchdog(handle: &Handle) {
    let _runtime = handle.enter();
    while !SHUTDOWN.load(Ordering::SeqCst) {
        std::thread::sleep(WATCHDOG_INTERVAL);
        for request in take_expired_requests(Instant::now()) {
            handle_expired(&request);
        }
        for device in installed_devices() {
            if let Some(pid) = device.server_pid
                && !server_process::is_server(pid, device.device_id)
            {
                retire_generation(
                    device.device_id,
                    device.generation,
                    &format!(
                        "sp1-gpu-server (pid {pid}) for CUDA device {} exited",
                        device.device_id
                    ),
                );
            }
        }
    }
}

fn handle_expired(request: &InFlight) {
    let device_id = request.device_id;
    let Some(generation) = request.generation else {
        // A server start hung (no client exists yet). Report the device as down and kill this
        // device's servers so the blocked start fails; the start path then retries.
        let reason = format!(
            "starting sp1-gpu-server on CUDA device {device_id} exceeded {START_TIMEOUT:?}; killed it"
        );
        error!(device_id, "{reason}");
        {
            let mut slots = lock(&SLOTS);
            let slot = slots.entry(device_id).or_default();
            if slot.current.is_none() {
                mark_failed(slot, device_id, reason);
            }
        }
        server_process::kill_servers(device_id);
        return;
    };
    let reason = format!(
        "SP1 CUDA {} on device {device_id} exceeded sp1.cuda_timeout_secs; killed the hung sp1-gpu-server",
        request.operation
    );
    error!(device_id, "{reason}");
    retire_generation(device_id, generation, &reason);
    // Kill exactly the server that request was waiting on. Its socket closes, so the blocked
    // request returns; a replacement server is never touched.
    let killed = request
        .server_pid
        .is_some_and(|pid| server_process::kill_server(pid, device_id));
    if !killed {
        warn!(
            device_id,
            "Hung sp1-gpu-server process not found; the request may stay blocked"
        );
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
        Self::register(InFlight {
            device_id: device.device_id,
            generation: Some(device.generation),
            server_pid: device.server_pid,
            operation,
            deadline: deadline_after(device.timeout),
        })
    }

    fn start(device_id: u32) -> Self {
        Self::register(InFlight {
            device_id,
            generation: None,
            server_pid: None,
            operation: "start",
            deadline: deadline_after(START_TIMEOUT),
        })
    }

    fn register(request: InFlight) -> Self {
        let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        lock(&IN_FLIGHT).insert(id, request);
        Self(id)
    }
}

/// `now + timeout`, capped so an oversized timeout can never overflow `Instant`.
fn deadline_after(timeout: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(timeout)
        .or_else(|| now.checked_add(Duration::from_secs(MAX_CUDA_TIMEOUT_SECS)))
        .unwrap_or(now)
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
        // Gas does not affect the cycle count. Deferred-proof verification stays on: invalid
        // aggregation sub-proofs must fail here, not as server errors that count toward
        // retiring the device.
        self.prover
            .execute(pk.elf().clone(), stdin.clone())
            .cycle_limit(cycle_limit)
            .calculate_gas(false)
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
                self.consecutive_server_errors.store(0, Ordering::Relaxed);
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
        IN_FLIGHT, InFlight, deadline_after, is_server_connection_failure, lock, panic_message,
        single_new_server, take_expired_requests,
    };

    #[test]
    fn oversized_timeouts_do_not_overflow_the_deadline() {
        let before = Instant::now();
        assert!(deadline_after(Duration::MAX) >= before);
        assert!(deadline_after(Duration::from_secs(60)) > before);
    }

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
    fn a_start_must_produce_exactly_one_new_server() {
        assert_eq!(single_new_server(1, &[4242]), Ok(4242));
        let reused =
            single_new_server(1, &[]).expect_err("no new server means a reused connection");
        assert!(reused.contains("reused"), "{reused}");
        assert!(single_new_server(1, &[1, 2]).is_err());
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
                    generation: Some(1),
                    server_pid: Some(4242),
                    operation: "prove",
                    deadline: now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
                },
            );
            in_flight.insert(
                u64::MAX - 1,
                InFlight {
                    device_id: 9,
                    generation: Some(1),
                    server_pid: Some(4242),
                    operation: "setup",
                    deadline: now + Duration::from_hours(1),
                },
            );
        }

        let expired = take_expired_requests(now);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].operation, "prove");
        assert_eq!(expired[0].server_pid, Some(4242));
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
