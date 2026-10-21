use clap::Parser;
use std::process::ExitCode;
use zkgas::{
    identity::executable_identity,
    quote::{QuoteArgs, run_quote},
};

fn main() -> ExitCode {
    init_tracing();
    let args = match QuoteArgs::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };
            let _ = error.print();
            return code;
        }
    };
    let result = executable_identity().and_then(|executable| run_quote(&args, &executable));
    match result {
        Ok(run) if run.exit_code() == 0 => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(2),
        Err(error) => {
            tracing::error!(error = %format_args!("{error:#}"), "zkgas quote failed");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
}
