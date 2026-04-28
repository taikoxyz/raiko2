mod bench_guest;
mod build_guest;
mod latest_proposal_request;
mod register_image;
mod release_image;
mod replay_guest_input;
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

    /// Build a `/v3/proof/batch/shasta` request for the latest onchain proposal.
    LatestProposalRequest(latest_proposal_request::LatestProposalRequestArgs),

    /// Build guest ELFs, build and push the runtime image.
    ReleaseImage(release_image::ReleaseImageArgs),

    /// Register the current Shasta guest image ids on verifier contracts.
    RegisterImage(register_image::RegisterImageArgs),

    /// Replay repo-managed Shasta GuestInput fixtures.
    ReplayGuestInput(replay_guest_input::ReplayGuestInputArgs),
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Backend {
    Risc0,
    Sp1,
    All,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let root = util::repo_root();

    match args.cmd {
        Cmd::BuildGuest(args) => build_guest::run(&root, args),
        Cmd::BenchGuest(args) => bench_guest::run(&root, *args),
        Cmd::LatestProposalRequest(args) => latest_proposal_request::run(&root, args).await,
        Cmd::ReleaseImage(args) => release_image::run(&root, args),
        Cmd::RegisterImage(args) => register_image::run(&root, args).await,
        Cmd::ReplayGuestInput(args) => replay_guest_input::run(&root, args),
    }
}
