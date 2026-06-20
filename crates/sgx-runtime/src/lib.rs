//! Shared TEE runtime helpers for dedicated SGX and TDX prover binaries.

#![allow(clippy::redundant_pub_crate)]

mod aggregation;
/// Bootstrap lifecycle support.
pub mod bootstrap;
/// Lifecycle validation support.
pub mod check;
/// Runtime configuration support.
pub mod config;
mod proposal;
mod protocol;
mod server;
mod startup;
mod tee;

use anyhow::Result;

pub use bootstrap::{
    BootstrapData, RegisteredInstanceIds, bootstrap, load_bootstrap_data,
    load_registered_instance_ids, public_key_to_address, save_bootstrap_data,
    save_registered_instance_ids,
};
pub use check::check;
pub use config::{
    DEFAULT_NATIVE_INSTANCE_ID, DEFAULT_NATIVE_TDX_INSTANCE_ID, GlobalOpts, RuntimeFlavor,
    RuntimeMode, ServeOpts, ServiceConfig, resolve_service_config,
};

/// Run the TEE proving server.
///
/// # Errors
///
/// Returns an error when the runtime configuration cannot be resolved, the listener cannot
/// bind, or the axum server exits with an error.
pub async fn serve(global_opts: GlobalOpts, serve_opts: ServeOpts) -> Result<()> {
    let service_config = resolve_service_config(&global_opts, &serve_opts)?;
    startup::log_startup_summary(&global_opts, &service_config);
    match (global_opts.flavor, global_opts.mode) {
        (RuntimeFlavor::Sgx, RuntimeMode::Tee) => {
            let provider = tee::GramineProvider::new(global_opts.secret_dir);
            server::serve(provider, service_config).await
        }
        (RuntimeFlavor::Tdx, RuntimeMode::Tee) => {
            anyhow::bail!("TDX tee mode is not implemented")
        }
        (_, RuntimeMode::Native) => {
            server::serve(tee::NativeProvider::new(global_opts.flavor), service_config).await
        }
    }
}
