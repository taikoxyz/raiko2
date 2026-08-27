#![allow(async_fn_in_trait)]
#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko2 Pipeline - hardfork-specific manifest builders and pipeline specs.

use raiko2_primitives::{ProofContext, ProofType, RaikoError, RaikoResult};
use raiko2_provider::Provider;
use reth_ethereum_primitives::Block;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr, sync::Arc};

mod pipeline;
pub mod proposal;

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
///
/// The `serde` names and [`PipelineKey::as_str`] values are frozen wire identity, not cosmetics.
/// The `serde` names appear in persisted `ProofArtifactRecord` JSON, and the `as_str` values are
/// path components of live GCS proof URIs. Renaming either orphans every stored artifact and
/// re-pays for proofs already produced. See the frozen identifier list in `CONCEPTS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PipelineKey {
    #[serde(rename = "ShastaRisc0")]
    Risc0Local,
    #[serde(rename = "ShastaSp1")]
    Sp1Local,
    #[serde(rename = "ShastaNative")]
    NativeLocal,
    #[serde(rename = "ShastaRisc0Network")]
    Risc0Network,
    #[serde(rename = "ShastaSgx")]
    SgxRemote,
    #[serde(rename = "ShastaSgxGeth")]
    SgxGethRemote,
}

impl PipelineKey {
    pub const ALL: [Self; 6] = [
        Self::Risc0Local,
        Self::Sp1Local,
        Self::NativeLocal,
        Self::Risc0Network,
        Self::SgxRemote,
        Self::SgxGethRemote,
    ];

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            PipelineKey::Risc0Local => "shasta-risc0-local",
            PipelineKey::Sp1Local => "shasta-sp1-local",
            PipelineKey::NativeLocal => "shasta-native-local",
            PipelineKey::Risc0Network => "shasta-risc0-network",
            PipelineKey::SgxRemote => "shasta-sgx-remote",
            PipelineKey::SgxGethRemote => "shasta-sgxgeth-remote",
        }
    }

    #[must_use]
    pub const fn route(self) -> PipelineRoute {
        match self {
            Self::Risc0Local => PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Local),
            Self::Sp1Local => PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Local),
            Self::NativeLocal => PipelineRoute::new(GuestSystem::Native, RunnerKind::Local),
            Self::SgxRemote => PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote),
            Self::SgxGethRemote => PipelineRoute::new(GuestSystem::SgxGeth, RunnerKind::Remote),
            Self::Risc0Network => PipelineRoute::new(GuestSystem::Risc0, RunnerKind::Network),
        }
    }

    #[must_use]
    pub const fn proof_type(self) -> ProofType {
        match self {
            Self::Risc0Local | Self::Risc0Network => ProofType::Risc0,
            Self::Sp1Local => ProofType::Sp1,
            Self::NativeLocal => ProofType::Native,
            Self::SgxRemote => ProofType::Sgx,
            Self::SgxGethRemote => ProofType::SgxGeth,
        }
    }

    #[must_use]
    pub const fn supports_route(self, route: PipelineRoute) -> bool {
        matches!(
            (self, route),
            (
                Self::Risc0Local,
                PipelineRoute {
                    guest_system: GuestSystem::Risc0,
                    runner: RunnerKind::Local,
                }
            ) | (
                Self::Risc0Network,
                PipelineRoute {
                    guest_system: GuestSystem::Risc0,
                    runner: RunnerKind::Network,
                }
            ) | (
                Self::Sp1Local,
                PipelineRoute {
                    guest_system: GuestSystem::Sp1,
                    runner: RunnerKind::Local | RunnerKind::Network,
                }
            ) | (
                Self::NativeLocal,
                PipelineRoute {
                    guest_system: GuestSystem::Native,
                    runner: RunnerKind::Local,
                }
            ) | (
                Self::SgxRemote,
                PipelineRoute {
                    guest_system: GuestSystem::Sgx,
                    runner: RunnerKind::Remote,
                }
            ) | (
                Self::SgxGethRemote,
                PipelineRoute {
                    guest_system: GuestSystem::SgxGeth,
                    runner: RunnerKind::Remote,
                }
            )
        )
    }

    /// Canonicalizes route identity read from persisted state.
    #[must_use]
    pub const fn canonicalize_persisted_route(self, route: PipelineRoute) -> Option<PipelineRoute> {
        if self.supports_route(route) {
            return Some(route);
        }
        match (self, route) {
            (
                Self::SgxGethRemote,
                PipelineRoute {
                    guest_system: GuestSystem::Sgx,
                    runner: RunnerKind::Remote,
                },
            ) => Some(self.route()),
            _ => None,
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
            "shasta-risc0-local" => Ok(Self::Risc0Local),
            "shasta-sp1-local" => Ok(Self::Sp1Local),
            "shasta-native-local" => Ok(Self::NativeLocal),
            "shasta-sgx-remote" => Ok(Self::SgxRemote),
            "shasta-sgxgeth-remote" => Ok(Self::SgxGethRemote),
            "shasta-risc0-network" => Ok(Self::Risc0Network),
            _ => Err(format!("Unknown pipeline key: {s}")),
        }
    }
}

/// Guest execution system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GuestSystem {
    #[default]
    Risc0,
    Sp1,
    Native,
    Sgx,
    SgxGeth,
}

impl GuestSystem {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Risc0 => "risc0",
            Self::Sp1 => "sp1",
            Self::Native => "native",
            Self::Sgx => "sgx",
            Self::SgxGeth => "sgxgeth",
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
            "sgxgeth" => Ok(Self::SgxGeth),
            _ => Err(format!("Unknown guest_system: {s}")),
        }
    }
}

/// Prover runner implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
            "network" => Ok(Self::Network),
            "remote" => Ok(Self::Remote),
            _ => Err(format!("Unknown runner: {s}")),
        }
    }
}

/// Canonical route for a proving request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
mod frozen_identity_tests {
    use super::PipelineKey;

    /// Pins the `as_str` values, which are path components of live GCS proof URIs.
    ///
    /// Changing any of these orphans every stored artifact under the old path and makes the
    /// service re-prove and re-pay for work it already completed. If this test fails because a
    /// value was deliberately changed, the change needs a storage migration, not a new expectation.
    #[test]
    fn pipeline_key_as_str_values_are_frozen_gcs_path_components() {
        assert_eq!(PipelineKey::Risc0Local.as_str(), "shasta-risc0-local");
        assert_eq!(PipelineKey::Sp1Local.as_str(), "shasta-sp1-local");
        assert_eq!(PipelineKey::NativeLocal.as_str(), "shasta-native-local");
        assert_eq!(PipelineKey::Risc0Network.as_str(), "shasta-risc0-network");
        assert_eq!(PipelineKey::SgxRemote.as_str(), "shasta-sgx-remote");
        assert_eq!(PipelineKey::SgxGethRemote.as_str(), "shasta-sgxgeth-remote");
    }

    /// Pins the `serde` names, which appear in persisted `ProofArtifactRecord` JSON.
    ///
    /// These are deliberately decoupled from the Rust variant names: the variants were renamed
    /// away from the retired Shasta-era spelling, while the stored representation must keep
    /// deserializing records written by earlier releases.
    #[test]
    fn pipeline_key_serde_names_are_frozen_persisted_identity() {
        for (key, expected) in [
            (PipelineKey::Risc0Local, "\"ShastaRisc0\""),
            (PipelineKey::Sp1Local, "\"ShastaSp1\""),
            (PipelineKey::NativeLocal, "\"ShastaNative\""),
            (PipelineKey::Risc0Network, "\"ShastaRisc0Network\""),
            (PipelineKey::SgxRemote, "\"ShastaSgx\""),
            (PipelineKey::SgxGethRemote, "\"ShastaSgxGeth\""),
        ] {
            let encoded = serde_json::to_string(&key).expect("serialize pipeline key");
            assert_eq!(encoded, expected, "serde name drifted for {key:?}");
            let decoded: PipelineKey =
                serde_json::from_str(&encoded).expect("deserialize pipeline key");
            assert_eq!(decoded, key);
        }
    }

    /// `as_str` and `FromStr` must stay mutually inverse, or persisted routes stop resolving.
    #[test]
    fn pipeline_key_string_round_trip_is_total() {
        for key in PipelineKey::ALL {
            assert_eq!(
                key.as_str().parse::<PipelineKey>().expect("round trip"),
                key
            );
        }
    }
}

#[cfg(test)]
mod route_tests {
    use super::{GuestSystem, PipelineKey, PipelineRoute, RunnerKind};
    use raiko2_primitives::ProofType;

    #[test]
    fn pipeline_key_is_the_canonical_route_owner() {
        let pipeline_key = PipelineKey::Risc0Network;
        let route = pipeline_key.route();

        assert!(pipeline_key.supports_route(route));
        assert_eq!(
            "shasta-risc0-network"
                .parse::<PipelineKey>()
                .expect("parse pipeline key"),
            pipeline_key
        );
    }

    #[test]
    fn superseded_boundless_route_aliases_are_rejected() {
        assert!("shasta-risc0-boundless".parse::<PipelineKey>().is_err());
        assert!("boundless".parse::<RunnerKind>().is_err());
        assert!("risc0/boundless".parse::<PipelineRoute>().is_err());
    }

    #[test]
    fn pipeline_route_rejects_unsupported_combo() {
        let route = PipelineRoute::new(GuestSystem::Native, RunnerKind::Network);
        assert!(
            !PipelineKey::ALL
                .into_iter()
                .any(|pipeline| pipeline.supports_route(route))
        );
    }

    #[test]
    fn sp1_network_route_uses_sp1_pipeline() {
        let route = PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network);
        assert!(PipelineKey::Sp1Local.supports_route(route));
    }

    #[test]
    fn sgx_remote_route_uses_only_sgx_pipeline() {
        let route = PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote);
        assert!(PipelineKey::SgxRemote.supports_route(route));
        assert!(!PipelineKey::SgxGethRemote.supports_route(route));
        assert_eq!(
            "shasta-sgx-remote"
                .parse::<PipelineKey>()
                .expect("parse sgx pipeline key"),
            PipelineKey::SgxRemote
        );
        assert_eq!(
            "sgx/remote"
                .parse::<PipelineRoute>()
                .expect("parse sgx route"),
            route
        );
    }

    #[test]
    fn sgxgeth_remote_route_is_distinct_from_sgx() {
        let sgx = "sgx/remote"
            .parse::<PipelineRoute>()
            .expect("parse sgx route");
        let sgxgeth = "sgxgeth/remote"
            .parse::<PipelineRoute>()
            .expect("parse sgxgeth route");

        assert_ne!(sgxgeth, sgx);
        assert_eq!(sgxgeth.to_string(), "sgxgeth/remote");
        assert_eq!(PipelineKey::SgxGethRemote.route(), sgxgeth);
    }

    #[test]
    fn pipeline_key_parses_sgx_variants() {
        assert_eq!(
            "shasta-sgx-remote".parse::<PipelineKey>().expect("sgx key"),
            PipelineKey::SgxRemote
        );
        assert_eq!(
            "shasta-sgxgeth-remote"
                .parse::<PipelineKey>()
                .expect("sgxgeth key"),
            PipelineKey::SgxGethRemote
        );
    }

    #[test]
    fn pipeline_key_owns_proof_type_and_route_identity() {
        let sp1_network = PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network);
        assert!(PipelineKey::Sp1Local.supports_route(sp1_network));
        assert_eq!(PipelineKey::Sp1Local.proof_type(), ProofType::Sp1);

        let sgx_remote = PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote);
        assert!(PipelineKey::SgxRemote.supports_route(sgx_remote));
        assert!(!PipelineKey::SgxGethRemote.supports_route(sgx_remote));
        let sgxgeth_remote = PipelineRoute::new(GuestSystem::SgxGeth, RunnerKind::Remote);
        assert!(PipelineKey::SgxGethRemote.supports_route(sgxgeth_remote));
        assert!(!PipelineKey::SgxRemote.supports_route(sgxgeth_remote));
        assert_eq!(PipelineKey::SgxRemote.proof_type(), ProofType::Sgx);
        assert_eq!(PipelineKey::SgxGethRemote.proof_type(), ProofType::SgxGeth);
        assert!(!PipelineKey::NativeLocal.supports_route(sp1_network));
    }

    #[test]
    fn persisted_route_compatibility_is_limited_to_legacy_sgxgeth() {
        let legacy_sgxgeth = PipelineRoute::new(GuestSystem::Sgx, RunnerKind::Remote);
        let canonical_sgxgeth = PipelineKey::SgxGethRemote.route();
        let sp1_network = PipelineRoute::new(GuestSystem::Sp1, RunnerKind::Network);

        assert_eq!(
            PipelineKey::SgxGethRemote.canonicalize_persisted_route(canonical_sgxgeth),
            Some(canonical_sgxgeth)
        );
        assert_eq!(
            PipelineKey::Sp1Local.canonicalize_persisted_route(sp1_network),
            Some(sp1_network)
        );
        assert_eq!(
            PipelineKey::SgxGethRemote.canonicalize_persisted_route(legacy_sgxgeth),
            Some(canonical_sgxgeth)
        );
        assert_eq!(
            PipelineKey::SgxRemote.canonicalize_persisted_route(canonical_sgxgeth),
            None
        );
        assert_eq!(
            PipelineKey::SgxGethRemote.canonicalize_persisted_route(PipelineRoute::new(
                GuestSystem::Risc0,
                RunnerKind::Local,
            )),
            None
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

    /// # Errors
    ///
    /// Returns an error if the backend cannot provide SP1 verifying key bytes.
    fn sp1_vk(&self, _stage: ProofStage) -> RaikoResult<&[u8]> {
        Err(RaikoError::InvalidRequestConfig(
            "backend does not provide SP1 verifying key".to_string(),
        ))
    }
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
pub struct Risc0ProposalBackend {
    elf_backend: ShastaElfBackend,
}

impl Risc0ProposalBackend {
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

impl ProverBackend for Risc0ProposalBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&[u8]> {
        self.elf_backend.elf(stage)
    }
}

/// SP1 backend for Shasta guest programs.
#[derive(Debug, Clone)]
pub struct Sp1ProposalBackend {
    elf_backend: ShastaElfBackend,
    proposal_vk: Arc<[u8]>,
    aggregation_vk: Arc<[u8]>,
}

impl Sp1ProposalBackend {
    #[must_use]
    pub const fn new(
        proposal_elf: Arc<[u8]>,
        aggregation_elf: Arc<[u8]>,
        proposal_vk: Arc<[u8]>,
        aggregation_vk: Arc<[u8]>,
    ) -> Self {
        Self {
            elf_backend: ShastaElfBackend::new(proposal_elf, aggregation_elf),
            proposal_vk,
            aggregation_vk,
        }
    }
}

impl ProverBackend for Sp1ProposalBackend {
    fn elf(&self, stage: ProofStage) -> RaikoResult<&[u8]> {
        self.elf_backend.elf(stage)
    }

    fn sp1_vk(&self, stage: ProofStage) -> RaikoResult<&[u8]> {
        Ok(match stage {
            ProofStage::Proposal => self.proposal_vk.as_ref(),
            ProofStage::Aggregation => self.aggregation_vk.as_ref(),
        })
    }
}
