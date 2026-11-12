//! Local SP1 proving on NVIDIA GPUs (`cuda` feature).
//!
//! SP1 6.x proves on CUDA through `sp1-gpu-server`, a separate process that `sp1-sdk` starts per
//! device and talks to over `/tmp/sp1-cuda-<id>.sock`. The server's proving keys live on the GPU,
//! so the client (which owns the server process) and each program's key are cached instead of
//! being rebuilt per proof.
//!
//! A proof first runs a cycle-bounded CPU execution of its input on a shared light prover
//! (`sp1-gpu-server` never receives the cycle limit), and only then takes the device's client.
//! Holding the client during that CPU work would keep a retired connection alive and block
//! restarts.
//!
//! Client lifecycle, per device:
//! - Starts are single-flight. Before sp1-sdk is asked to start a server, the pinned
//!   `sp1-gpu-server` binary must exist and report the exact version sp1-cuda requires, so sp1-sdk
//!   never tries to download one (with no timeout). Starts have their own watchdog deadline.
//! - The pinned `sp1-cuda` client never reconnects or restarts its server, has no request
//!   timeout, and hands out its existing per-device connection while anything (including
//!   asynchronous key cleanup tasks) still references it. Each client therefore records the
//!   server process it started (via `/proc` on Linux). A start must produce a new server process;
//!   otherwise sp1-cuda reused an old connection and the client is discarded. Before a restart
//!   the previous server is killed and waited on (bounded).
//! - A client is retired, and its server killed, when: a request fails at the transport level;
//!   the server reports errors for [`MAX_DISTINCT_FAILING_INPUTS`] different inputs in a row; its
//!   server process exits; or a setup/proof exceeds `sp1.cuda_timeout_secs` at the server. GPU
//!   requests take turns per device and the deadline starts only once a request has its turn. An
//!   input whose proof timed out is remembered and its retries fail without using the GPU.
//! - A retired or failed device is reported by [`cuda_device_failures`] (surfaced by `/ready`),
//!   proofs on it fail fast, and a supervisor thread restarts it with exponential backoff,
//!   independent of proof traffic. A replacement is installed only after it set up every program
//!   the device has served. A program whose setup fails for its own reasons (a verifying-key
//!   mismatch) is dropped from that set; any server failure keeps the device restarting.
//! - [`shutdown_cuda_devices`] stops the supervisors and watchdog, kills the servers, and releases
//!   the cached clients at process shutdown; a start still running then is not installed.

mod server_process;

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use raiko2_primitives::{RaikoError, RaikoResult};
use sp1_sdk::{
    ProvingKey as _, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    blocking::{
        CudaProver as BlockingCudaProver, LightProver, ProveRequest as _, Prover as _,
        ProverClient as BlockingProverClient,
    },
};
use tokio::runtime::Handle;
use tracing::{error, info, warn};

use super::cuda_support::{
    FailingInputs, InFlight, InFlightRequests, MAX_DISTINCT_FAILING_INPUTS, TimedOutInputs,
    deadline_after, execute_within_cycle_limit, input_key, is_server_connection_failure, lock,
    panic_message, single_new_server,
};
use super::sp1_vk_digest;
use crate::sp1_config::DEFAULT_CUDA_TIMEOUT_SECS;

type CudaProvingKey = <BlockingCudaProver as sp1_sdk::blocking::Prover>::ProvingKey;

const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(2);
const RESTART_BACKOFF_MAX: Duration = Duration::from_mins(1);
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(5);
const SHUTTING_DOWN: &str = "SP1 CUDA proving is shutting down";
/// How long a restart waits for killed servers to exit (releasing their socket and GPU memory).
const PREVIOUS_SERVER_EXIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Deadline for starting `sp1-gpu-server` and connecting to it.
const START_TIMEOUT: Duration = Duration::from_mins(5);
/// Deadline for the pre-start `sp1-gpu-server --version` check.
const BINARY_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Validates the pinned local GPU server without installing or starting it.
///
/// Standalone SDK tools can call this before constructing a CUDA client, whose SDK builder
/// otherwise attempts to download a missing or mismatched server binary.
///
/// # Errors
///
/// Returns an error if the pinned server is unavailable or its bounded version check fails.
pub fn validate_cuda_server_binary() -> Result<(), String> {
    server_process::check_server_binary(sp1_primitives::SP1_CRATE_VERSION, BINARY_CHECK_TIMEOUT)
}

/// Where local SP1 GPU proofs run: a CUDA device and the per-request deadline on its server.
#[derive(Clone, Copy, Debug)]
pub(super) struct CudaTarget {
    device_id: u32,
    timeout: Duration,
}

/// One CUDA device: the `sp1-gpu-server` client plus the proving keys already set up on it.
struct Sp1CudaDevice {
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
    failing_inputs: Mutex<FailingInputs>,
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

static SLOTS: LazyLock<Mutex<HashMap<u32, DeviceSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static START_LOCKS: LazyLock<Mutex<HashMap<u32, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static IN_FLIGHT: LazyLock<Mutex<InFlightRequests>> =
    LazyLock::new(|| Mutex::new(InFlightRequests::default()));
static TIMED_OUT_INPUTS: LazyLock<Mutex<TimedOutInputs>> =
    LazyLock::new(|| Mutex::new(TimedOutInputs::default()));
static LIGHT_PROVER: OnceLock<LightProver> = OnceLock::new();
static WATCHDOG: OnceLock<()> = OnceLock::new();
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// CUDA devices whose `sp1-gpu-server` failed, hung, or died and has not been replaced yet.
///
/// Also starts a supervisor for any such device that has none (it panicked or could not be
/// spawned), so readiness probes alone keep recovery going without proof traffic.
#[must_use]
pub fn cuda_device_failures() -> Vec<(u32, String)> {
    let mut slots = lock(&SLOTS);
    let mut failures = Vec::new();
    for (device_id, slot) in slots.iter_mut() {
        if slot.current.is_some() {
            continue;
        }
        let Some(failure) = slot.failure.clone() else {
            continue;
        };
        if !slot.supervising {
            mark_failed(slot, *device_id, failure.clone());
        }
        failures.push((*device_id, failure));
    }
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

impl CudaTarget {
    pub(super) const fn new(device_id: u32, timeout: Duration) -> Self {
        Self { device_id, timeout }
    }

    /// Proves `stdin` on the GPU after a cycle-bounded CPU execution of the same input.
    ///
    /// Must run on a blocking thread inside the Tokio runtime: `sp1-sdk` drives the server
    /// through its own runtime (and panics inside an async context), while dropping a client or
    /// key spawns a cleanup task on the current runtime.
    pub(super) fn prove(
        self,
        pk: &SP1ProvingKey,
        stdin: SP1Stdin,
        proof_mode: SP1ProofMode,
        cycle_limit: u64,
    ) -> Result<SP1ProofWithPublicValues, String> {
        let vk = sp1_vk_digest(pk.verifying_key());
        let input = input_key(&vk, &stdin);
        if lock(&TIMED_OUT_INPUTS).contains(input) {
            return Err(format!(
                "SP1 CUDA proof of this input already exceeded sp1.cuda_timeout_secs ({:?}) on \
                 device {}; not retrying it on the GPU",
                self.timeout, self.device_id
            ));
        }
        // Fail fast while the device is down, before spending CPU time.
        drop(installed(self.device_id).map_err(|err| err.to_string())?);
        // Runs without holding the device's client, so a restart is never blocked by it.
        execute_within_cycle_limit(light_prover(), pk.elf().clone(), stdin.clone(), cycle_limit)
            .map_err(|err| format!("{err}; not proving on the GPU"))?;

        let device = device(self.device_id, self.timeout).map_err(|err| err.to_string())?;
        device.prove(pk, stdin, proof_mode, input)
    }
}

/// Executes on the shared CPU-only prover; `mode = "execute"` never needs the GPU.
pub(super) fn execute(
    elf: Vec<u8>,
    stdin: SP1Stdin,
) -> Result<(sp1_sdk::SP1PublicValues, sp1_sdk::ExecutionReport), String> {
    light_prover()
        .execute(elf.into(), stdin)
        .run()
        .map_err(|err| err.to_string())
}

/// Verifies on the shared CPU-only prover.
pub(super) fn verify(
    proof: &SP1ProofWithPublicValues,
    vk: &sp1_sdk::SP1VerifyingKey,
) -> Result<(), String> {
    light_prover()
        .verify(proof, vk, None)
        .map_err(|err| err.to_string())
}

/// CPU-only prover for execution and verification, shared by every request.
fn light_prover() -> &'static LightProver {
    LIGHT_PROVER.get_or_init(LightProver::new)
}

/// Returns the client for `device_id`, starting `sp1-gpu-server` on first use.
fn device(device_id: u32, timeout: Duration) -> RaikoResult<Arc<Sp1CudaDevice>> {
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
///
/// A down device without a running supervisor (it panicked, or could not be spawned) gets a new
/// one here, so it never stays down for good.
fn installed(device_id: u32) -> RaikoResult<Option<Arc<Sp1CudaDevice>>> {
    let mut slots = lock(&SLOTS);
    let Some(slot) = slots.get_mut(&device_id) else {
        return Ok(None);
    };
    if let Some(device) = &slot.current {
        return Ok(Some(Arc::clone(device)));
    }
    let Some(failure) = slot.failure.clone() else {
        return Ok(None);
    };
    if !slot.supervising {
        mark_failed(slot, device_id, failure.clone());
    }
    Err(RaikoError::Guest(format!(
        "SP1 CUDA device {device_id} is unavailable and restarting in the background: {failure}"
    )))
}

fn start_lock(device_id: u32) -> Arc<Mutex<()>> {
    Arc::clone(lock(&START_LOCKS).entry(device_id).or_default())
}

/// Starts a fresh `sp1-gpu-server` client and sets up every program the device has served.
/// Call with the device's start lock held.
fn start_device(device_id: u32, timeout: Duration) -> Result<Arc<Sp1CudaDevice>, String> {
    // Without the exact pinned binary sp1-sdk would download one with no timeout.
    server_process::check_server_binary(sp1_primitives::SP1_CRATE_VERSION, BINARY_CHECK_TIMEOUT)?;
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
        failing_inputs: Mutex::new(FailingInputs::default()),
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
            "No Tokio runtime to restart sp1-gpu-server; the next SP1 proof retries"
        );
        return;
    };
    let spawned = std::thread::Builder::new()
        .name(format!("sp1-cuda-supervisor-{device_id}"))
        .spawn(move || supervise(device_id, &handle));
    match spawned {
        Ok(_) => slot.supervising = true,
        Err(err) => warn!(
            device_id,
            %err,
            "Failed to spawn the SP1 CUDA supervisor; the next SP1 proof retries"
        ),
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
        let start_lock = start_lock(device_id);
        let _start = lock(&start_lock);
        // Shutdown, or a start in `device()` succeeded meanwhile (for example after its
        // deadline): stop without killing a healthy installed server.
        {
            let mut slots = lock(&SLOTS);
            let slot = slots.entry(device_id).or_default();
            if slot.current.is_some() || SHUTDOWN.load(Ordering::SeqCst) {
                slot.supervising = false;
                return;
            }
        }
        let timeout = lock(&SLOTS)
            .get(&device_id)
            .and_then(|slot| slot.timeout)
            .unwrap_or(Duration::from_secs(DEFAULT_CUDA_TIMEOUT_SECS));
        let result = catch_unwind(AssertUnwindSafe(|| start_device(device_id, timeout)))
            .unwrap_or_else(|panic| {
                Err(format!(
                    "SP1 CUDA restart panicked: {}",
                    panic_message(panic.as_ref())
                ))
            });

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

/// Clears `supervising` if a supervisor panics, so the next proof or readiness probe starts a
/// new one. Normal exits clear it under the same lock that installs or abandons the client.
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

/// Retires `generation` of `device_id` if it is still installed, kills its server, and starts a
/// restart. Killing it stops queued work from starting real proofs on a server presumed broken.
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
    if let Some(pid) = retired.server_pid {
        server_process::kill_server(pid, device_id);
    }
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
        let expired = lock(&IN_FLIGHT).take_expired(Instant::now());
        for request in expired {
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
    if let Some(input) = request.input {
        // The protocol has no progress signal, so a slow proof looks like a hung server. Retries
        // of this input fail without the GPU instead of taking the device down again.
        lock(&TIMED_OUT_INPUTS).record(input);
    }
    let reason = format!(
        "SP1 CUDA {} on device {device_id} exceeded sp1.cuda_timeout_secs; killed the hung sp1-gpu-server",
        request.operation
    );
    error!(device_id, "{reason}");
    // Kill exactly the server that request was waiting on (retirement does too while its client
    // is still installed). Its socket closes, so the blocked request returns; a replacement
    // server is never touched.
    retire_generation(device_id, generation, &reason);
    if let Some(pid) = request.server_pid {
        server_process::kill_server(pid, device_id);
    } else {
        warn!(
            device_id,
            "Hung sp1-gpu-server process unknown; the request may stay blocked"
        );
    }
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
    fn new(device: &Sp1CudaDevice, operation: &'static str, input: Option<u64>) -> Self {
        Self::register(InFlight {
            device_id: device.device_id,
            generation: Some(device.generation),
            server_pid: device.server_pid,
            input,
            operation,
            deadline: deadline_after(device.timeout),
        })
    }

    fn start(device_id: u32) -> Self {
        Self::register(InFlight {
            device_id,
            generation: None,
            server_pid: None,
            input: None,
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

impl Drop for WatchedRequest {
    fn drop(&mut self) {
        lock(&IN_FLIGHT).remove(self.0);
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
    /// Proves an input that already passed the cycle-bounded CPU execution.
    fn prove(
        &self,
        pk: &SP1ProvingKey,
        stdin: SP1Stdin,
        proof_mode: SP1ProofMode,
        input: u64,
    ) -> Result<SP1ProofWithPublicValues, String> {
        let cuda_pk = self.proving_key(pk)?;
        let proof = {
            let _turn = lock(&self.gpu_turn);
            let _watched = WatchedRequest::new(self, "prove", Some(input));
            self.prover.prove(&cuda_pk, stdin).mode(proof_mode).run()
        };
        match proof {
            Ok(proof) => {
                lock(&self.failing_inputs).record_success();
                Ok(proof)
            }
            Err(err) => Err(self.client_error("prove", &err.to_string(), input)),
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
                lock(&self.failing_inputs).record_success();
                // Only programs that set up successfully join the warm set used on restart.
                lock(&SLOTS)
                    .entry(self.device_id)
                    .or_default()
                    .programs
                    .entry(vk)
                    .or_insert_with(|| pk.clone());
                Ok(key)
            }
            Err(SetupError::Client(err)) => {
                let input = input_key(&vk, &SP1Stdin::new());
                Err(self.client_error("setup", &err, input))
            }
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
            let _watched = WatchedRequest::new(self, "setup", None);
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

    /// Formats a client error and retires this client when the server looks broken.
    fn client_error(&self, operation: &str, error: &str, input: u64) -> String {
        let message = format!(
            "SP1 CUDA {operation} failed on device {}: {error}",
            self.device_id
        );
        if is_server_connection_failure(error) {
            retire_generation(self.device_id, self.generation, &message);
        } else {
            // Count distinct inputs, so one input that fails deterministically and is retried
            // alone cannot retire the device by itself.
            let failing = lock(&self.failing_inputs).record_failure(input);
            if failing >= MAX_DISTINCT_FAILING_INPUTS {
                retire_generation(
                    self.device_id,
                    self.generation,
                    &format!(
                        "server errors on {failing} different inputs in a row, last: {message}"
                    ),
                );
            }
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};

    use sp1_cuda::CudaClientError;

    use super::is_server_connection_failure;

    /// Real `sp1-cuda` errors, so a change in its messages fails here.
    #[test]
    fn sp1_cuda_errors_are_classified() {
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
        for error in [
            CudaClientError::ServerError("out of memory".to_string()),
            CudaClientError::ProverError("bad witness".to_string()),
        ] {
            let message = error.to_string();
            assert!(!is_server_connection_failure(&message), "{message}");
        }
    }
}
