mod bench_guest;
mod fixture;
mod latest_proposal_request;
mod register_image;
mod release_image;
mod release_tee_manifest;
mod release_tee_providers;
mod replay_guest_input;
mod tee_provider_lock;
mod update_tee_provider_lock;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use xtask_build_guest::BuildGuestArgs;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build guest ELF binaries using official docker tooling.
    BuildGuest(BuildGuestArgs),

    /// Run guest benchmarks following the PR #9 workflow.
    BenchGuest(Box<bench_guest::BenchGuestArgs>),

    /// Build a `/v3/proof/batch/shasta` request for the latest onchain proposal.
    LatestProposalRequest(latest_proposal_request::LatestProposalRequestArgs),

    /// Build guest ELFs, build and push the runtime image.
    ReleaseImage(release_image::ReleaseImageArgs),

    /// Export the current Shasta guest registration digests.
    GuestDigests(xtask_build_guest::guest_digests::GuestDigestsArgs),

    /// Register the current Shasta guest image ids on verifier contracts.
    RegisterImage(register_image::RegisterImageArgs),

    /// Build, optionally push, and export TEE provider attestation metadata.
    ReleaseTeeProviders(release_tee_providers::ReleaseTeeProvidersArgs),

    /// Replay repo-managed Shasta GuestInput fixtures.
    ReplayGuestInput(replay_guest_input::ReplayGuestInputArgs),

    /// Check repo-managed fixture envelopes.
    Fixture(fixture::FixtureArgs),

    /// Update the checked-in TEE provider source pin.
    UpdateTeeProviderLock(update_tee_provider_lock::UpdateTeeProviderLockArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let root = util::repo_root();

    match args.cmd {
        Cmd::BuildGuest(args) => xtask_build_guest::run(&root, args),
        Cmd::BenchGuest(args) => bench_guest::run(&root, *args),
        Cmd::LatestProposalRequest(args) => latest_proposal_request::run(&root, args).await,
        Cmd::ReleaseImage(args) => release_image::run(&root, args),
        Cmd::GuestDigests(args) => xtask_build_guest::guest_digests::run(&root, args),
        Cmd::RegisterImage(args) => register_image::run(&root, args).await,
        Cmd::ReleaseTeeProviders(args) => release_tee_providers::run(&root, args),
        Cmd::ReplayGuestInput(args) => replay_guest_input::run(&root, args),
        Cmd::Fixture(args) => fixture::run(&root, args),
        Cmd::UpdateTeeProviderLock(args) => update_tee_provider_lock::run(&root, args),
    }
}
