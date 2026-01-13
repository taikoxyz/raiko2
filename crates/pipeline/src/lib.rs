#![allow(async_fn_in_trait)]
#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko2 Pipeline - hardfork-specific manifest builders and pipeline specs.

use raiko2_primitives::{ProofContext, RaikoError, RaikoResult};
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
    Encode,
    Prove,
    Aggregate,
}

/// Pipeline identifier for routing requests to the right engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PipelineKey {
    ShastaRisc0,
    ShastaSp1,
    ShastaNative,
}

impl PipelineKey {
    pub const fn as_str(&self) -> &'static str {
        match self {
            PipelineKey::ShastaRisc0 => "shasta-risc0",
            PipelineKey::ShastaSp1 => "shasta-sp1",
            PipelineKey::ShastaNative => "shasta-native",
        }
    }
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
    type Input;
    async fn preflight<P: Provider>(
        &self,
        ctx: &ProofContext,
        provider: &P,
    ) -> RaikoResult<Self::Input>;
}

/// Build Taiko manifests for guest execution.
#[async_trait::async_trait]
pub trait ManifestBuilder: Send + Sync {
    type Manifest: Send + Sync + 'static;
    async fn taiko_manifest(
        &self,
        ctx: &ProofContext,
        blocks: &[Block],
    ) -> RaikoResult<Self::Manifest>;
}

/// Validate a guest input for the hardfork.
pub trait Validation: Send + Sync {
    type Input;
    fn validate(&self, ctx: &ProofContext, input: &Self::Input) -> RaikoResult<()>;
}

/// No-op validation for tests or fast paths.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopValidation<I>(std::marker::PhantomData<I>);

impl<I> NoopValidation<I> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<I: Send + Sync> Validation for NoopValidation<I> {
    type Input = I;

    fn validate(&self, _ctx: &ProofContext, _input: &Self::Input) -> RaikoResult<()> {
        Ok(())
    }
}

/// No-op manifest builder for tests or fast paths.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopManifestBuilder;

#[async_trait::async_trait]
impl ManifestBuilder for NoopManifestBuilder {
    type Manifest = ();

    async fn taiko_manifest(
        &self,
        _ctx: &ProofContext,
        _blocks: &[Block],
    ) -> RaikoResult<Self::Manifest> {
        Ok(())
    }
}

/// Pipeline-specific behavior for building inputs.
pub trait PipelineSpec: Send + Sync {
    type GuestInput: Clone + Send + Sync + 'static;
    type Preflight: Preflight<Input = Self::GuestInput>;
    type Validation: Validation<Input = Self::GuestInput>;
    type ManifestBuilder: ManifestBuilder;
    type Prover: Send + Sync;
    type Backend: ProverBackend;
    type Provider: Provider;

    fn pipeline_key(&self) -> PipelineKey;
    fn prover(&self) -> &Self::Prover;
    fn backend(&self) -> &Self::Backend;
    fn provider(&self) -> &Self::Provider;
    fn preflight(&self) -> &Self::Preflight;
    fn validation(&self) -> &Self::Validation;
    fn manifest_builder(&self) -> &Self::ManifestBuilder;
}

/// Prover backend abstraction for selecting guest programs.
pub trait ProverBackend: Send + Sync {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&'static [u8]>;
}

/// Native backend placeholder (no ELF).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeBackend;

impl ProverBackend for NativeBackend {
    fn elf(&self, _stage: ProofStage) -> RaikoResult<&'static [u8]> {
        Err(RaikoError::InvalidRequestConfig(
            "native backend does not provide ELF".to_string(),
        ))
    }
}

/// Shared ELF selector for Shasta guest programs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShastaElfBackend {
    proposal_elf: &'static [u8],
    aggregation_elf: &'static [u8],
}

impl ShastaElfBackend {
    pub(crate) const fn new(proposal_elf: &'static [u8], aggregation_elf: &'static [u8]) -> Self {
        Self {
            proposal_elf,
            aggregation_elf,
        }
    }
}

impl ProverBackend for ShastaElfBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&'static [u8]> {
        Ok(match stage {
            ProofStage::Proposal => self.proposal_elf,
            ProofStage::Aggregation => self.aggregation_elf,
        })
    }
}

/// RISC0 backend for Shasta guest programs.
#[derive(Debug, Clone, Copy)]
pub struct Risc0ShastaBackend {
    elf_backend: ShastaElfBackend,
}

impl Risc0ShastaBackend {
    pub const fn new(proposal_elf: &'static [u8], aggregation_elf: &'static [u8]) -> Self {
        Self {
            elf_backend: ShastaElfBackend::new(proposal_elf, aggregation_elf),
        }
    }

    pub(crate) const fn from_elf_backend(elf_backend: ShastaElfBackend) -> Self {
        Self { elf_backend }
    }
}

impl ProverBackend for Risc0ShastaBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&'static [u8]> {
        self.elf_backend.elf(stage)
    }
}

/// SP1 backend for Shasta guest programs.
#[derive(Debug, Clone, Copy)]
pub struct Sp1ShastaBackend {
    elf_backend: ShastaElfBackend,
}

impl Sp1ShastaBackend {
    pub const fn new(proposal_elf: &'static [u8], aggregation_elf: &'static [u8]) -> Self {
        Self {
            elf_backend: ShastaElfBackend::new(proposal_elf, aggregation_elf),
        }
    }

    pub(crate) const fn from_elf_backend(elf_backend: ShastaElfBackend) -> Self {
        Self { elf_backend }
    }
}

impl ProverBackend for Sp1ShastaBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&'static [u8]> {
        self.elf_backend.elf(stage)
    }
}
