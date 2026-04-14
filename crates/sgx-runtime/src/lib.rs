//! SGX runtime helpers for the dedicated `raiko2-sgx-prover` binary.

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
mod tee;

use anyhow::Result;

pub use bootstrap::{
    BootstrapData, RegisteredInstanceIds, bootstrap, load_bootstrap_data,
    load_registered_instance_ids, public_key_to_address, save_bootstrap_data,
    save_registered_instance_ids,
};
pub use check::check;
pub use config::{
    DEFAULT_NATIVE_INSTANCE_ID, GlobalOpts, RuntimeMode, ServeOpts, ServiceConfig,
    resolve_service_config,
};

/// Run the SGX proving server.
///
/// # Errors
///
/// Returns an error when the runtime configuration cannot be resolved, the listener cannot
/// bind, or the axum server exits with an error.
pub async fn serve(global_opts: GlobalOpts, serve_opts: ServeOpts) -> Result<()> {
    let service_config = resolve_service_config(&global_opts, &serve_opts)?;
    match global_opts.mode {
        RuntimeMode::Tee => {
            let provider = tee::GramineProvider::new(global_opts.secret_dir);
            server::serve(provider, service_config).await
        }
        RuntimeMode::Native => server::serve(tee::NativeProvider, service_config).await,
    }
}
