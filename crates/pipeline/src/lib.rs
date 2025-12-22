#![allow(async_fn_in_trait)]
#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko2 Pipeline - hardfork-specific manifest builders and pipeline specs.

use raiko2_primitives::{GuestInput, ProofContext, RaikoResult, TaikoManifest};
use raiko2_provider::Provider;
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};

pub mod forks;
mod pipeline;

pub use pipeline::Pipeline;

/// Proof stage selector for hardfork-specific programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofStage {
    Proposal,
    Aggregation,
}

/// Pipeline stage selector for tracking progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStage {
    Preflight,
    Validation,
    Prove,
    Aggregate,
}

/// Pipeline stage output wrapper for status tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStageResult<T> {
    pub stage: PipelineStage,
    pub output: T,
}

impl<T> PipelineStageResult<T> {
    pub const fn new(stage: PipelineStage, output: T) -> Self {
        Self { stage, output }
    }
}

/// Build and validate a guest input for the hardfork.
#[async_trait::async_trait]
pub trait Preflight: Send + Sync {
    async fn preflight<P: Provider>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<GuestInput>;
}

/// Build Taiko manifests for guest execution.
#[async_trait::async_trait]
pub trait ManifestBuilder: Send + Sync {
    async fn taiko_manifest(
        &self,
        ctx: &ProofContext,
        blocks: &[Block],
    ) -> RaikoResult<TaikoManifest>;
}

/// Validate a guest input for the hardfork.
pub trait Validation: Send + Sync {
    fn validate(&self, ctx: &ProofContext, input: &GuestInput) -> RaikoResult<()>;
}

/// No-op validation for tests or fast paths.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopValidation;

impl Validation for NoopValidation {
    fn validate(&self, _ctx: &ProofContext, _input: &GuestInput) -> RaikoResult<()> {
        Ok(())
    }
}

/// No-op manifest builder for tests or fast paths.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopManifestBuilder;

#[async_trait::async_trait]
impl ManifestBuilder for NoopManifestBuilder {
    async fn taiko_manifest(
        &self,
        _ctx: &ProofContext,
        _blocks: &[Block],
    ) -> RaikoResult<TaikoManifest> {
        Ok(TaikoManifest::default())
    }
}

/// Pipeline-specific behavior for building inputs.
pub trait PipelineSpec<B>: Send + Sync {
    type Preflight: Preflight;
    type Validation: Validation;
    type ManifestBuilder: ManifestBuilder;

    fn preflight(&self) -> &Self::Preflight;
    fn validation(&self) -> &Self::Validation;
    fn manifest_builder(&self) -> &Self::ManifestBuilder;
}

/// Prover backend abstraction for selecting guest programs.
pub trait ProverBackend<S>: Send + Sync {
    fn elf(&self, spec: &S, stage: ProofStage) -> RaikoResult<&'static [u8]>;
}

/// RISC0 backend marker type.
#[derive(Debug, Clone, Copy, Default)]
pub struct Risc0Backend;

/// SP1 backend marker type.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sp1Backend;
