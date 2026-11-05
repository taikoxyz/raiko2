//! Local SP1 proving on NVIDIA GPUs (`cuda` feature).
//!
//! SP1 6.x proves on CUDA through `sp1-gpu-server`, a separate process that `sp1-sdk` starts per
//! device and talks to over `/tmp/sp1-cuda-<id>.sock`. The server's proving keys live on the GPU,
//! so both the client (which owns the server process) and each program's key are cached for the
//! lifetime of the raiko2 process instead of being rebuilt per proof.
//!
//! `sp1-sdk` resolves the server binary at `$HOME/.sp1/bin/sp1-gpu-server` and downloads it from
//! the SP1 GitHub release when missing. `Dockerfile.cuda` pre-installs the pinned binary so the
//! runtime never downloads code.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use raiko2_primitives::{RaikoError, RaikoResult};
use sp1_sdk::{
    ProvingKey as _, SP1ProvingKey,
    blocking::{
        CudaProver as BlockingCudaProver, Prover as _, ProverClient as BlockingProverClient,
    },
};
use tracing::info;

use super::sp1_vk_digest;

type CudaProvingKey = <BlockingCudaProver as sp1_sdk::blocking::Prover>::ProvingKey;

/// One CUDA device: the `sp1-gpu-server` client plus the proving keys already set up on it.
pub(super) struct Sp1CudaDevice {
    device_id: u32,
    prover: BlockingCudaProver,
    /// Proving keys keyed by SP1 verifying-key digest.
    keys: Mutex<HashMap<String, CudaProvingKey>>,
}

static DEVICES: OnceLock<Mutex<HashMap<u32, Arc<Sp1CudaDevice>>>> = OnceLock::new();

/// Returns the process-wide client for `device_id`, starting `sp1-gpu-server` on first use.
///
/// Must run on a blocking thread: `sp1-sdk` drives the server through its own Tokio runtime and
/// panics when called from inside an async context. A failed start panics inside `sp1-sdk`; the
/// caller's `spawn_blocking` turns that into a task error.
pub(super) fn device(device_id: u32) -> Arc<Sp1CudaDevice> {
    let devices = DEVICES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(device) = lock(devices).get(&device_id) {
        return Arc::clone(device);
    }

    // Build outside the lock so a panic during server start does not poison the device map.
    info!(device_id, "Starting SP1 CUDA prover (sp1-gpu-server)");
    let prover = BlockingProverClient::builder()
        .cuda()
        .with_device_id(device_id)
        .build();
    let device = Arc::new(Sp1CudaDevice {
        device_id,
        prover,
        keys: Mutex::new(HashMap::new()),
    });
    Arc::clone(lock(devices).entry(device_id).or_insert(device))
}

impl Sp1CudaDevice {
    pub(super) const fn prover(&self) -> &BlockingCudaProver {
        &self.prover
    }

    /// Returns the GPU-resident proving key for `pk`, running SP1 setup on first use.
    ///
    /// The key is derived from the ELF on the GPU server; its verifying key must match the
    /// checked-in verifying key that raiko2 registers and verifies against.
    pub(super) fn proving_key(&self, pk: &SP1ProvingKey) -> RaikoResult<CudaProvingKey> {
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
        let key = self.prover.setup(pk.elf().clone()).map_err(|err| {
            RaikoError::Guest(format!(
                "SP1 CUDA setup failed on device {}: {err}",
                self.device_id
            ))
        })?;
        let actual_vk = sp1_vk_digest(key.verifying_key());
        if actual_vk != expected_vk {
            return Err(RaikoError::Guest(format!(
                "SP1 CUDA setup produced verifying key {actual_vk}, expected {expected_vk}; \
                 the guest ELF and verifying key artifacts are out of sync"
            )));
        }
        keys.insert(expected_vk, key.clone());
        Ok(key)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
