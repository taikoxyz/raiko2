use anyhow::Result;
use clap::Parser;
use xtask_build_guest::guest_digests::{GuestDigestsArgs, run};
use xtask_build_guest::repo_root;

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(flatten)]
    args: GuestDigestsArgs,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(&repo_root(), cli.args)
}
