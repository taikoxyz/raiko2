//! TDX Prover for Raiko V2
//!
//! This module provides a Trusted Domain Extensions (TDX) prover that runs
//! inside a TDX-protected VM. Unlike zkVM provers (RISC0, SP1), the TDX prover
//! trusts the host execution environment, and produces TEE attestation quotes
//! instead of zero-knowledge proofs.
//!
//! ## Bootstrap
//!
//! On the first proof request the prover auto-bootstraps:
//! 1. Generates a fresh secp256k1 private key
//! 2. Requests a TDX attestation quote embedding the derived Ethereum address
//! 3. Persists key + quote to `~/.config/raiko2/tdx/`
//!
//! ## Proof flow
//!
//! 1. Compute the protocol instance hash from the block data and carry data
//! 2. Sign the hash with the bootstrapped private key
//! 3. Build the 89-byte proof (`instance_id` ‖ address ‖ signature)
//! 4. Request a TDX attestation quote over the instance hash

mod attestation_client;
pub mod config;
mod proof;
pub mod signature;
pub mod types;

use raiko2_primitives_shasta::instance::shasta_aggregation_output_from_proof_carry_data_vec;
pub use types::{TdxConfig, TdxResponse};

use alloy_primitives::{Bytes, Uint};
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{AggregationGuestInput, Proof, ProverConfig, RaikoError, RaikoResult};
use raiko2_primitives_shasta::{GuestInput, encode_proof_carry_data};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use raiko2_protocol_shasta::shasta::{Checkpoint, ProofCarryData};
use tokio::sync::OnceCell;
use tracing::info;

use crate::{
    GuestInputCodec, parse_proof_carry_data, parse_shasta_aggregation_input,
    validate_shasta_aggregation_lengths,
};

/// TDX Prover for Shasta proposal and aggregation proofs.
pub struct TdxProver {
    config: TdxConfig,
    /// Lazy one-time bootstrap guard.
    bootstrapped: OnceCell<()>,
}

impl TdxProver {
    /// Create a new TDX prover with the given configuration.
    #[must_use]
    pub fn new(config: TdxConfig) -> Self {
        Self {
            config,
            bootstrapped: OnceCell::new(),
        }
    }

    /// Ensure the prover has been bootstrapped (key + quote generated).
    ///
    /// This is idempotent — if bootstrap data already exists on disk it is reused.
    /// Call at startup to fail fast if the TDX environment is misconfigured.
    ///
    /// # Errors
    ///
    /// Returns an error if key generation or attestation quote retrieval fails.
    pub async fn ensure_bootstrapped(&self) -> RaikoResult<()> {
        self.bootstrapped
            .get_or_try_init(|| async {
                bootstrap(&self.config.socket_path)
                    .map_err(|e| RaikoError::Guest(format!("TDX bootstrap failed: {e}")))?;
                Ok::<(), RaikoError>(())
            })
            .await?;
        Ok(())
    }
}

impl GuestInputCodec<GuestInput> for TdxProver {
    fn encode(&self, input: &GuestInput, _config: &ProverConfig) -> RaikoResult<Bytes> {
        let bytes = bincode::serialize(input)
            .map_err(|e| RaikoError::Guest(format!("Failed to serialize input: {e}")))?;
        Ok(Bytes::from(bytes))
    }
}

#[async_trait::async_trait]
impl<B> crate::Prover<B> for TdxProver
where
    B: ProverBackend,
{
    type GuestInput = GuestInput;

    fn encode(&self, input: &Self::GuestInput, config: &ProverConfig) -> RaikoResult<Bytes> {
        GuestInputCodec::encode(self, input, config)
    }

    async fn prove_encoded(
        &self,
        input: Bytes,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        info!("Starting TDX proposal proof generation...");

        // Auto-bootstrap on first call
        self.ensure_bootstrapped().await?;

        let input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;

        if input.witnesses.is_empty() {
            return Err(RaikoError::Guest(
                "GuestInput must contain at least one witness".to_string(),
            ));
        }

        let mut proof_carry_data: ProofCarryData = parse_proof_carry_data(config);

        // Update checkpoint from actual block execution results.
        // This is the source of truth — matches raiko v1 behaviour where the
        // checkpoint is always derived from the executed blocks, not config.
        let last = input.witnesses.last().ok_or_else(|| {
            RaikoError::Guest("GuestInput must contain at least one witness".to_string())
        })?;

        proof_carry_data.transition_input.checkpoint = Checkpoint {
            blockNumber: Uint::from(last.block.header.number),
            blockHash: last.block.header.hash_slow(),
            stateRoot: last.block.header.state_root,
        };

        // Use the same domain-separated hash as raiko v1's TDX prover.
        // `hash_shasta_subproof_input` hashes all TransitionInputData fields
        // (proposal linkage, prover identity, full checkpoint) with a
        // VERIFY_PROOF domain tag, chain_id, and verifier address.
        let instance_hash = hash_shasta_subproof_input(&proof_carry_data);
        let extra_data = encode_proof_carry_data(&proof_carry_data)?;

        // Generate TDX proof: sign + attestation quote
        let prove_data = proof::prove(
            &self.config.socket_path,
            self.config.instance_id,
            instance_hash,
        )
        .map_err(|e| RaikoError::Guest(format!("TDX proof generation failed: {e}")))?;

        info!("TDX proposal proof generated successfully");

        Ok(TdxResponse {
            proof: format!("0x{}", hex::encode(&prove_data.proof)),
            quote: hex::encode(&prove_data.quote),
            input: prove_data.instance_hash,
            extra_data: Some(extra_data),
        }
        .into())
    }

    async fn aggregate(
        &self,
        input: AggregationGuestInput,
        config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        info!(
            "Starting TDX aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        // Auto-bootstrap on first call
        self.ensure_bootstrapped().await?;

        let aggregation_input = parse_shasta_aggregation_input(config)?;
        validate_shasta_aggregation_lengths(&aggregation_input)?;

        let sgx_instance = config::load_private_key()
            .map(|key| signature::address_from_private_key(&key))
            .map_err(|e| RaikoError::Guest(format!("Failed to load TDX key: {e}")))?;

        let aggregation_hash = shasta_aggregation_output_from_proof_carry_data_vec(
            &aggregation_input.proof_carry_data_vec,
            sgx_instance,
        )
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(
                "Invalid proof_carry_data_vec for aggregation".to_string(),
            )
        })?;

        // Collect sub-proof bytes + input hashes for verification
        let sub_proofs = input
            .proofs
            .iter()
            .map(|p| {
                let proof_hex = p.proof.as_ref().ok_or_else(|| {
                    RaikoError::Guest("Missing proof bytes in sub-proof".to_string())
                })?;
                let proof_bytes = hex::decode(proof_hex.trim_start_matches("0x")).map_err(|e| {
                    RaikoError::Guest(format!("Failed to decode sub-proof hex: {e}"))
                })?;
                let input_hash = p.input.ok_or_else(|| {
                    RaikoError::Guest("Missing input hash in sub-proof".to_string())
                })?;
                Ok((proof_bytes, input_hash))
            })
            .collect::<RaikoResult<Vec<_>>>()?;

        let agg_data = proof::prove_shasta_aggregation(
            &self.config.socket_path,
            self.config.instance_id,
            &sub_proofs,
            aggregation_hash,
        )
        .map_err(|e| RaikoError::Guest(format!("TDX aggregation failed: {e}")))?;

        info!("TDX aggregation proof generated successfully");

        Ok(TdxResponse {
            proof: format!("0x{}", hex::encode(&agg_data.proof)),
            quote: hex::encode(&agg_data.quote),
            input: agg_data.aggregation_hash,
            extra_data: None,
        }
        .into())
    }
}

// ────────────────────────── Bootstrap ──────────────────────────

/// Bootstrap the TDX prover (idempotent).
///
/// If bootstrap data already exists on disk, this is a no-op.
/// Otherwise, generates a new private key and requests a TDX attestation quote.
fn bootstrap(socket_path: &str) -> anyhow::Result<()> {
    if config::bootstrap_exists()? {
        info!("TDX already bootstrapped, reusing existing key");
        return Ok(());
    }

    info!("Bootstrapping TDX prover...");

    let private_key = config::generate_private_key()?;
    let address = signature::address_from_private_key(&private_key);
    info!("Generated TDX prover address: {address}");

    let (quote, nonce) = proof::generate_tdx_quote_from_public_key(socket_path, &address)?;
    info!("TDX bootstrap quote generated ({} bytes)", quote.len());

    let metadata = proof::get_tdx_metadata(socket_path)?;
    config::write_bootstrap(
        &metadata.issuer_type,
        &quote,
        &address,
        &nonce,
        metadata.metadata,
    )?;

    info!("TDX bootstrap complete");
    Ok(())
}
