//! CLI entrypoint for the dedicated SGX proving runtime.

use anyhow::Result;
use clap::{Parser, Subcommand};
use raiko2_sgx_runtime::{GlobalOpts, ServeOpts};

#[derive(Debug, Parser)]
#[command(name = "raiko2-sgx-prover")]
#[command(about = "Dedicated SGX runtime service for Raiko2")]
struct App {
    #[command(flatten)]
    global_opts: GlobalOpts,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Bootstrap the SGX runtime and emit operator metadata.
    Bootstrap,
    /// Check the SGX runtime lifecycle state.
    Check,
    /// Run the SGX Shasta proving server.
    Serve(ServeOpts),
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();
    match app.command {
        Command::Bootstrap => {
            let data = raiko2_sgx_runtime::bootstrap(&app.global_opts)?;
            println!("{}", serde_json::to_string_pretty(&data)?);
            Ok(())
        }
        Command::Check => raiko2_sgx_runtime::check(&app.global_opts),
        Command::Serve(serve_opts) => raiko2_sgx_runtime::serve(app.global_opts, serve_opts).await,
    }
}
