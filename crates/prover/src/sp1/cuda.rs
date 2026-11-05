//! Local SP1 proving on NVIDIA GPUs (`cuda` feature).
//!
//! SP1 6.x proves on CUDA through `sp1-gpu-server`, a separate process that `sp1-sdk` starts per
//! device and talks to over `/tmp/sp1-cuda-<id>.sock`. The server's proving keys live on the GPU,
//! so the client (which owns the server process) and each program's key are cached instead of
//! being rebuilt per proof.
//!
//! The pinned `sp1-cuda` client never reconnects or restarts its server. When a request fails at
//! the transport level (server exited, socket closed), the cached client is retired: new proofs
//! fail fast while in-flight users drain, then a background task starts a fresh server. Until that
//! succeeds the device is reported by [`cuda_device_failures`], which `/ready` surfaces.
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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use raiko2_primitives::{RaikoError, RaikoResult};
use sp1_sdk::{
    ProvingKey as _, SP1ProofMode, SP1ProofWithPublicValues, SP1ProvingKey, SP1Stdin,
    blocking::{
        CudaProver as BlockingCudaProver, ProveRequest as _, Prover as _,
        ProverClient as BlockingProverClient,
    },
};
use tracing::{info, warn};

use super::sp1_vk_digest;

type CudaProvingKey = <BlockingCudaProver as sp1_sdk::blocking::Prover>::ProvingKey;

/// How long recovery waits for in-flight users of a retired client to drop it. While any user
/// still holds it, `sp1-cuda` would hand the same broken connection to a new client.
const RETIRED_DRAIN_TIMEOUT: Duration = Duration::from_mins(10);
const RETIRED_DRAIN_POLL: Duration = Duration::from_secs(1);

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
    /// Client retired after a transport failure; recovery waits until nothing references it.
    retired: Option<Weak<Sp1CudaDevice>>,
    /// Last start or transport failure; cleared once a fresh client is installed.
    failure: Option<String>,
    recovering: bool,
}

static DEVICES: OnceLock<Mutex<HashMap<u32, DeviceSlot>>> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn devices() -> &'static Mutex<HashMap<u32, DeviceSlot>> {
    DEVICES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// CUDA devices whose `sp1-gpu-server` failed to start or died and has not been replaced yet.
#[must_use]
pub fn cuda_device_failures() -> Vec<(u32, String)> {
    let mut failures: Vec<(u32, String)> = lock(devices())
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
    {
        let devices = lock(devices());
        if let Some(slot) = devices.get(&device_id) {
            if let Some(device) = &slot.current {
                return Ok(Arc::clone(device));
            }
            if slot
                .retired
                .as_ref()
                .is_some_and(|old| old.strong_count() > 0)
            {
                return Err(RaikoError::Guest(format!(
                    "SP1 CUDA device {device_id} is recovering after a server failure ({}); retry later",
                    slot.failure.as_deref().unwrap_or("unknown failure")
                )));
            }
        }
    }

    // Start outside the lock: server start can take seconds and `sp1-sdk` panics on failure.
    info!(device_id, "Starting SP1 CUDA prover (sp1-gpu-server)");
    let started = catch_unwind(AssertUnwindSafe(|| {
        BlockingProverClient::builder()
            .cuda()
            .with_device_id(device_id)
            .build()
    }));

    let mut devices = lock(devices());
    let slot = devices.entry(device_id).or_default();
    match started {
        Ok(prover) => {
            if let Some(existing) = &slot.current {
                return Ok(Arc::clone(existing));
            }
            let device = Arc::new(Sp1CudaDevice {
                device_id,
                generation: GENERATION.fetch_add(1, Ordering::Relaxed),
                prover,
                keys: Mutex::new(HashMap::new()),
            });
            slot.current = Some(Arc::clone(&device));
            slot.retired = None;
            slot.failure = None;
            Ok(device)
        }
        Err(panic) => {
            let message = format!(
                "failed to start sp1-gpu-server on CUDA device {device_id}: {}",
                panic_message(panic.as_ref())
            );
            slot.failure = Some(message.clone());
            Err(RaikoError::Guest(message))
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
    ///
    /// The key is derived from the ELF on the GPU server; its verifying key must match the
    /// checked-in verifying key that raiko2 registers and verifies against.
    fn proving_key(&self, pk: &SP1ProvingKey) -> Result<CudaProvingKey, String> {
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
            .map_err(|err| self.client_error("setup", &err.to_string()))?;
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
        let mut devices = lock(devices());
        let slot = devices.entry(self.device_id).or_default();
        let Some(current) = slot
            .current
            .take_if(|current| current.generation == self.generation)
        else {
            return;
        };
        warn!(
            device_id = self.device_id,
            reason, "Retiring SP1 CUDA client; a fresh sp1-gpu-server will be started"
        );
        slot.retired = Some(Arc::downgrade(&current));
        slot.failure = Some(reason.to_string());
        if !slot.recovering {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                slot.recovering = true;
                let device_id = self.device_id;
                drop(handle.spawn_blocking(move || recover(device_id)));
            } else {
                warn!(
                    device_id = self.device_id,
                    "No Tokio runtime to recover the SP1 CUDA client; the next proof retries"
                );
            }
        }
        // `current` is one of several strong references (the caller holds another), so this
        // never drops the client itself.
        drop(current);
    }
}

/// Waits for the retired client to be released, then starts a fresh `sp1-gpu-server`.
fn recover(device_id: u32) {
    let mut waited = Duration::ZERO;
    loop {
        let drained = lock(devices())
            .get(&device_id)
            .and_then(|slot| slot.retired.as_ref())
            .is_none_or(|old| old.strong_count() == 0);
        if drained || waited >= RETIRED_DRAIN_TIMEOUT {
            break;
        }
        std::thread::sleep(RETIRED_DRAIN_POLL);
        waited += RETIRED_DRAIN_POLL;
    }

    match device(device_id) {
        Ok(_) => info!(device_id, "Recovered SP1 CUDA client"),
        Err(err) => warn!(
            device_id,
            %err,
            "SP1 CUDA recovery failed; the next proof retries"
        ),
    }
    if let Some(slot) = lock(devices()).get_mut(&device_id) {
        slot.recovering = false;
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
