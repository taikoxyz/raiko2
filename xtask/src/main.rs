#[cfg(feature = "guest-tools")]
mod bench_guest;
#[cfg(feature = "guest-tools")]
mod latest_proposal_request;
#[cfg(feature = "guest-tools")]
mod register_image;
mod register_tdx;
mod release_image;
#[cfg(feature = "guest-tools")]
mod release_tee_manifest;
#[cfg(feature = "guest-tools")]
mod release_tee_providers;
#[cfg(feature = "guest-tools")]
mod replay_guest_input;
#[cfg(feature = "guest-tools")]
mod tee_provider_lock;
#[cfg(feature = "guest-tools")]
mod update_tee_provider_lock;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
#[cfg(feature = "guest-tools")]
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
    #[cfg(feature = "guest-tools")]
    BuildGuest(BuildGuestArgs),

    /// Run guest benchmarks following the PR #9 workflow.
    #[cfg(feature = "guest-tools")]
    BenchGuest(Box<bench_guest::BenchGuestArgs>),

    /// Build a `/v3/proof/batch/shasta` request for the latest onchain proposal.
    #[cfg(feature = "guest-tools")]
    LatestProposalRequest(latest_proposal_request::LatestProposalRequestArgs),

    /// Build guest ELFs, build and push the runtime image.
    ReleaseImage(release_image::ReleaseImageArgs),

    /// Export the current Shasta guest registration digests.
    #[cfg(feature = "guest-tools")]
    GuestDigests(xtask_build_guest::guest_digests::GuestDigestsArgs),

    /// Register the current Shasta guest image ids on verifier contracts.
    #[cfg(feature = "guest-tools")]
    RegisterImage(register_image::RegisterImageArgs),

    /// Register a TDX prover instance on an AzureTdxVerifier contract.
    RegisterTdx(register_tdx::RegisterTdxArgs),
    /// Build, optionally push, and export TEE provider attestation metadata.
    #[cfg(feature = "guest-tools")]
    ReleaseTeeProviders(release_tee_providers::ReleaseTeeProvidersArgs),

    /// Replay repo-managed Shasta GuestInput fixtures.
    #[cfg(feature = "guest-tools")]
    ReplayGuestInput(replay_guest_input::ReplayGuestInputArgs),

    /// Update the checked-in TEE provider source pin.
    #[cfg(feature = "guest-tools")]
    UpdateTeeProviderLock(update_tee_provider_lock::UpdateTeeProviderLockArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let root = util::repo_root();

    match args.cmd {
        #[cfg(feature = "guest-tools")]
        Cmd::BuildGuest(args) => xtask_build_guest::run(&root, args),
        #[cfg(feature = "guest-tools")]
        Cmd::BenchGuest(args) => bench_guest::run(&root, *args),
        #[cfg(feature = "guest-tools")]
        Cmd::LatestProposalRequest(args) => latest_proposal_request::run(&root, args).await,
        Cmd::ReleaseImage(args) => release_image::run(&root, args),
        #[cfg(feature = "guest-tools")]
        Cmd::GuestDigests(args) => xtask_build_guest::guest_digests::run(&root, args),
        #[cfg(feature = "guest-tools")]
        Cmd::RegisterImage(args) => register_image::run(&root, args).await,
        Cmd::RegisterTdx(args) => register_tdx::run(args).await,
        #[cfg(feature = "guest-tools")]
        Cmd::ReleaseTeeProviders(args) => release_tee_providers::run(&root, args),
        #[cfg(feature = "guest-tools")]
        Cmd::ReplayGuestInput(args) => replay_guest_input::run(&root, args),
        #[cfg(feature = "guest-tools")]
        Cmd::UpdateTeeProviderLock(args) => update_tee_provider_lock::run(&root, args),
    }
}
