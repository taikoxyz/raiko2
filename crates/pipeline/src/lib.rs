#![allow(async_fn_in_trait)]
#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko2 Pipeline - hardfork-specific manifest builders and pipeline specs.

use raiko2_primitives::{ProofContext, RaikoError, RaikoResult};
use raiko2_provider::Provider;
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr, sync::Arc};

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
    ShastaRisc0Network,
    ShastaSgx,
    ShastaSgxGeth,
    ShastaTdxDcap,
}

impl PipelineKey {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            PipelineKey::ShastaRisc0 => "shasta-risc0-local",
            PipelineKey::ShastaSp1 => "shasta-sp1-local",
            PipelineKey::ShastaNative => "shasta-native-local",
            PipelineKey::ShastaRisc0Network => "shasta-risc0-network",
            PipelineKey::ShastaSgx => "shasta-sgx-remote",
            PipelineKey::ShastaSgxGeth => "shasta-sgxgeth-remote",
            PipelineKey::ShastaTdxDcap => "shasta-tdx-dcap-remote",
        }
    }

    #[must_use]
    pub const fn route(self) -> PipelineRoute {
        match self {
            Self::ShastaRisc0 => PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Local),
            Self::ShastaSp1 => PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Local),
            Self::ShastaNative => PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
            Self::ShastaSgx | Self::ShastaSgxGeth => {
                PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote)
            }
            Self::ShastaRisc0Network => PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Network),
            Self::ShastaTdxDcap => PipelineRoute::new(GuestSystem::TdxDcap, RunnerKind::Remote),
        }
    }
}

impl fmt::Display for PipelineKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PipelineKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "shasta-risc0-local" => Ok(Self::ShastaRisc0),
            "shasta-sp1-local" => Ok(Self::ShastaSp1),
            "shasta-native-local" => Ok(Self::ShastaNative),
            "shasta-sgx-remote" => Ok(Self::ShastaSgx),
            "shasta-sgxgeth-remote" => Ok(Self::ShastaSgxGeth),
            "shasta-risc0-network" | "shasta-risc0-boundless" => Ok(Self::ShastaRisc0Network),
            "shasta-tdx-dcap-remote" => Ok(Self::ShastaTdxDcap),
            _ => Err(format!("Unknown pipeline key: {s}")),
        }
    }
}

/// Guest execution system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GuestSystem {
    #[default]
    Risc0,
    Sp1,
    Native,
    Sgx,
    #[serde(rename = "tdx_dcap")]
    TdxDcap,
}

impl GuestSystem {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Risc0 => "risc0",
            Self::Sp1 => "sp1",
            Self::Native => "native",
            Self::Sgx => "sgx",
            Self::TdxDcap => "tdx_dcap",
        }
    }
}

impl fmt::Display for GuestSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GuestSystem {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "risc0" => Ok(Self::Risc0),
            "sp1" => Ok(Self::Sp1),
            "native" => Ok(Self::Native),
            "sgx" => Ok(Self::Sgx),
            "tdx_dcap" => Ok(Self::TdxDcap),
            _ => Err(format!("Unknown guest_system: {s}")),
        }
    }
}

/// Prover runner implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RunnerKind {
    #[default]
    Local,
    Network,
    Remote,
}

impl RunnerKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Network => "network",
            Self::Remote => "remote",
        }
    }
}

impl fmt::Display for RunnerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RunnerKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "network" | "boundless" => Ok(Self::Network),
            "remote" => Ok(Self::Remote),
            _ => Err(format!("Unknown runner: {s}")),
        }
    }
}

/// Canonical route for a proving request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PipelineRoute {
    pub guest_system: GuestSystem,
    pub runner: RunnerKind,
}

impl PipelineRoute {
    #[must_use]
    pub const fn new(guest_system: GuestSystem, runner: RunnerKind) -> Self {
        Self {
            guest_system,
            runner,
        }
    }

    #[must_use]
    pub const fn proof_type(self) -> raiko2_primitives::ProofType {
        match self.guest_system {
            GuestSystem::Risc0 => raiko2_primitives::ProofType::Risc0,
            GuestSystem::Sp1 => raiko2_primitives::ProofType::Sp1,
            GuestSystem::Native => raiko2_primitives::ProofType::Native,
            GuestSystem::Sgx => raiko2_primitives::ProofType::Sgx,
            GuestSystem::TdxDcap => raiko2_primitives::ProofType::TdxDcap,
        }
    }

    /// # Errors
    ///
    /// Returns an error if the guest system and runner combination is not supported by the
    /// current canonical pipeline set.
    pub fn pipeline_key(self) -> Result<PipelineKey, String> {
        match (self.guest_system, self.runner) {
            (GuestSystem::Risc0, RunnerKind::Local) => Ok(PipelineKey::ShastaRisc0),
            (GuestSystem::Risc0, RunnerKind::Network) => Ok(PipelineKey::ShastaRisc0Network),
            (GuestSystem::Sp1, RunnerKind::Local | RunnerKind::Network) => {
                Ok(PipelineKey::ShastaSp1)
            }
            (GuestSystem::Native, RunnerKind::Local) => Ok(PipelineKey::ShastaNative),
            (GuestSystem::Native, RunnerKind::Network | RunnerKind::Remote) => {
                Err("Unsupported proving route: native/network".to_string())
            }
            (GuestSystem::Sgx, RunnerKind::Remote) => Ok(PipelineKey::ShastaSgx),
            (GuestSystem::Sgx, RunnerKind::Local) => {
                Err("Unsupported proving route: sgx/local".to_string())
            }
            (GuestSystem::Sgx, RunnerKind::Network) => {
                Err("Unsupported proving route: sgx/network".to_string())
            }
            (GuestSystem::TdxDcap, RunnerKind::Remote) => Ok(PipelineKey::ShastaTdxDcap),
            (GuestSystem::TdxDcap, RunnerKind::Network) => {
                Err("Unsupported proving route: tdx_dcap/network".to_string())
            }
            (GuestSystem::TdxDcap, RunnerKind::Local) => {
                Err("Unsupported proving route: tdx_dcap/local".to_string())
            }
            (GuestSystem::Sp1, RunnerKind::Remote) => {
                Err("Unsupported proving route: sp1/remote".to_string())
            }
            (GuestSystem::Risc0, RunnerKind::Remote) => {
                Err("Unsupported proving route: risc0/remote".to_string())
            }
        }
    }

    #[must_use]
    pub const fn from_pipeline_key(pipeline_key: PipelineKey) -> Self {
        pipeline_key.route()
    }
}

impl fmt::Display for PipelineRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.guest_system, self.runner)
    }
}

impl FromStr for PipelineRoute {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (guest_system, runner) = s
            .split_once('/')
            .ok_or_else(|| format!("Invalid route '{s}', expected <guest_system>/<runner>"))?;
        Ok(Self::new(guest_system.parse()?, runner.parse()?))
    }
}

#[cfg(test)]
mod route_tests {
    use super::{GuestSystem, PipelineKey, PipelineRoute, RunnerKind};

    #[test]
    fn pipeline_route_roundtrips_with_pipeline_key() {
        let route = PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Network);
        let pipeline_key = route.pipeline_key().expect("supported route");

        assert_eq!(pipeline_key, PipelineKey::ShastaRisc0Network);
        assert_eq!(PipelineRoute::from_pipeline_key(pipeline_key), route);
        assert_eq!(
            "shasta-risc0-network"
                .parse::<PipelineKey>()
                .expect("parse pipeline key"),
            pipeline_key
        );
    }

    #[test]
    fn pipeline_route_accepts_legacy_boundless_persisted_names() {
        assert_eq!(
            "shasta-risc0-boundless"
                .parse::<PipelineKey>()
                .expect("parse legacy pipeline key"),
            PipelineKey::ShastaRisc0Network
        );
        assert_eq!(
            "boundless"
                .parse::<RunnerKind>()
                .expect("parse legacy runner"),
            RunnerKind::Network
        );
        assert_eq!(
            "risc0/boundless"
                .parse::<PipelineRoute>()
                .expect("parse legacy route"),
            PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Network)
        );
    }

    #[test]
    fn pipeline_route_rejects_unsupported_combo() {
        let route = PipelineRoute::new(GuestSystem::Native, RunnerKind::Network);
        assert_eq!(
            route.pipeline_key().expect_err("unsupported route"),
            "Unsupported proving route: native/network"
        );
    }

    #[test]
    fn sp1_network_route_uses_sp1_pipeline() {
        let route = PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network);
        assert_eq!(
            route.pipeline_key().expect("supported route"),
            PipelineKey::ShastaSp1
        );
    }

    #[test]
    fn sgx_remote_route_uses_sgx_pipeline() {
        let route = PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote);
        assert_eq!(
            route.pipeline_key().expect("supported route"),
            PipelineKey::ShastaSgx
        );
        assert_eq!(
            "shasta-sgx-remote"
                .parse::<PipelineKey>()
                .expect("parse sgx pipeline key"),
            PipelineKey::ShastaSgx
        );
        assert_eq!(
            "sgx/remote"
                .parse::<PipelineRoute>()
                .expect("parse sgx route"),
            route
        );
    }

    #[test]
    fn pipeline_key_parses_sgx_variants() {
        assert_eq!(
            "shasta-sgx-remote".parse::<PipelineKey>().expect("sgx key"),
            PipelineKey::ShastaSgx
        );
        assert_eq!(
            "shasta-sgxgeth-remote"
                .parse::<PipelineKey>()
                .expect("sgxgeth key"),
            PipelineKey::ShastaSgxGeth
        );
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
    /// # Errors
    ///
    /// Returns an error if validation fails for the provided input.
    fn validate(&self, ctx: &ProofContext, input: &Self::Input) -> RaikoResult<()>;
}

/// No-op validation for tests or fast paths.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopValidation<I>(std::marker::PhantomData<I>);

impl<I> NoopValidation<I> {
    #[must_use]
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
    /// # Errors
    ///
    /// Returns an error if the backend cannot provide an ELF for the requested stage.
    fn elf(&self, stage: ProofStage) -> RaikoResult<&[u8]>;
}

/// Native backend placeholder (no ELF).
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeBackend;

impl ProverBackend for NativeBackend {
    fn elf(&self, _stage: ProofStage) -> RaikoResult<&[u8]> {
        Err(RaikoError::InvalidRequestConfig(
            "native backend does not provide ELF".to_string(),
        ))
    }
}

/// Shared ELF selector for Shasta guest programs.
#[derive(Debug, Clone)]
pub(crate) struct ShastaElfBackend {
    proposal_elf: Arc<[u8]>,
    aggregation_elf: Arc<[u8]>,
}

impl ShastaElfBackend {
    #[must_use]
    pub(crate) const fn new(proposal_elf: Arc<[u8]>, aggregation_elf: Arc<[u8]>) -> Self {
        Self {
            proposal_elf,
            aggregation_elf,
        }
    }
}

impl ProverBackend for ShastaElfBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&[u8]> {
        Ok(match stage {
            ProofStage::Proposal => self.proposal_elf.as_ref(),
            ProofStage::Aggregation => self.aggregation_elf.as_ref(),
        })
    }
}

/// RISC0 backend for Shasta guest programs.
#[derive(Debug, Clone)]
pub struct Risc0ShastaBackend {
    elf_backend: ShastaElfBackend,
}

impl Risc0ShastaBackend {
    #[must_use]
    pub const fn new(proposal_elf: Arc<[u8]>, aggregation_elf: Arc<[u8]>) -> Self {
        Self {
            elf_backend: ShastaElfBackend::new(proposal_elf, aggregation_elf),
        }
    }

    pub(crate) const fn from_elf_backend(elf_backend: ShastaElfBackend) -> Self {
        Self { elf_backend }
    }
}

impl ProverBackend for Risc0ShastaBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&[u8]> {
        self.elf_backend.elf(stage)
    }
}

/// SP1 backend for Shasta guest programs.
#[derive(Debug, Clone)]
pub struct Sp1ShastaBackend {
    elf_backend: ShastaElfBackend,
}

impl Sp1ShastaBackend {
    #[must_use]
    pub const fn new(proposal_elf: Arc<[u8]>, aggregation_elf: Arc<[u8]>) -> Self {
        Self {
            elf_backend: ShastaElfBackend::new(proposal_elf, aggregation_elf),
        }
    }

    pub(crate) const fn from_elf_backend(elf_backend: ShastaElfBackend) -> Self {
        Self { elf_backend }
    }
}

impl ProverBackend for Sp1ShastaBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&[u8]> {
        self.elf_backend.elf(stage)
    }
}
