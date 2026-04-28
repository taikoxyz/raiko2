use anyhow::Result;
use clap::Parser;
use xtask_build_guest::{BuildGuestArgs, repo_root, run};

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(flatten)]
    args: BuildGuestArgs,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(&repo_root(), cli.args)
}
