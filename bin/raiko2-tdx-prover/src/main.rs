//! CLI entrypoint for the dedicated TDX proving runtime.

use anyhow::Result;
use clap::{Parser, Subcommand};
use raiko2_sgx_runtime::{GlobalOpts, RuntimeFlavor, RuntimeMode, ServeOpts};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[derive(Debug, Parser)]
#[command(name = "raiko2-tdx-prover")]
#[command(about = "Dedicated TDX runtime service for Raiko2")]
struct App {
    #[command(flatten)]
    global_opts: TdxGlobalOpts,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, clap::Args)]
struct TdxGlobalOpts {
    #[command(flatten)]
    inner: GlobalOpts,
    /// Runtime mode used by the dedicated TDX prover.
    #[arg(long, env = "RAIKO2_TDX_MODE", value_enum, default_value_t = RuntimeMode::Tee)]
    mode: RuntimeMode,
}

impl From<TdxGlobalOpts> for GlobalOpts {
    fn from(opts: TdxGlobalOpts) -> Self {
        let mut inner = opts.inner;
        inner.mode = opts.mode;
        inner.for_flavor(RuntimeFlavor::Tdx)
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Bootstrap the TDX runtime and emit operator metadata.
    Bootstrap,
    /// Check the TDX runtime lifecycle state.
    Check,
    /// Run the TDX Shasta proving server.
    Serve(ServeOpts),
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();
    let global_opts = GlobalOpts::from(app.global_opts);
    init_logging();
    match app.command {
        Command::Bootstrap => {
            let data = raiko2_sgx_runtime::bootstrap(&global_opts)?;
            println!("{}", serde_json::to_string_pretty(&data)?);
            Ok(())
        }
        Command::Check => raiko2_sgx_runtime::check(&global_opts),
        Command::Serve(serve_opts) => raiko2_sgx_runtime::serve(global_opts, serve_opts).await,
    }
}

fn init_logging() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_ansi(false))
        .init();
}
