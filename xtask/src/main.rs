mod bench_guest;
mod build_guest;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build guest ELF binaries using official docker tooling.
    BuildGuest(build_guest::BuildGuestArgs),

    /// Run guest benchmarks following the PR #9 workflow.
    BenchGuest(Box<bench_guest::BenchGuestArgs>),
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Backend {
    Risc0,
    Sp1,
    All,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = util::repo_root();

    match args.cmd {
        Cmd::BuildGuest(args) => build_guest::run(&root, args),
        Cmd::BenchGuest(args) => bench_guest::run(&root, *args),
    }
}
