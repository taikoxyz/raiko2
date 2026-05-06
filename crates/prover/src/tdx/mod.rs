//! TDX Prover for Raiko V2
//!
//! Trusted Domain Extensions (TDX) prover that runs inside a TDX-protected VM.
//! Unlike zkVM provers (RISC0, SP1) the TDX prover trusts the host execution
//! environment and produces a TEE attestation quote bound to the proof's
//! public input hash.
//!
//! ## Bootstrap
//!
//! On first use the prover auto-bootstraps:
//! 1. Generates a fresh secp256k1 private key
//! 2. Requests a TDX attestation quote embedding the derived Ethereum address
//! 3. Persists key + quote to `~/.config/raiko2/tdx/`
//!
//! ## Proof flow
//!
//! 1. Compute the Shasta sub-proof input hash from `GuestInput::proof_carry_data`
//! 2. Sign the hash with the bootstrapped private key
//! 3. Build the 89-byte SGX-compatible proof (`instance_id` ‖ address ‖ signature)
//! 4. Request a TDX attestation quote over the input hash

mod attestation_client;
pub mod config;
mod proof;
pub mod signature;
pub mod types;

pub use types::{TdxConfig, TdxResponse};

use alloy_primitives::Bytes;
use raiko2_pipeline::ProverBackend;
use raiko2_primitives::{
    AggregationGuestInput, Proof, ProofType, ProverConfig, RaikoError, RaikoResult,
};
use raiko2_primitives_shasta::{
    GuestInput, encode_proof_carry_data,
    instance::{build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output},
};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use tokio::sync::OnceCell;
use tracing::info;

use crate::{GuestInputCodec, build_shasta_aggregation_input};

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
    /// Idempotent — reuses existing bootstrap data on disk if present.
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

    /// Return the bootstrap quote, public key, nonce, and attestation metadata.
    ///
    /// Bootstraps lazily if necessary, then reads `~/.config/raiko2/tdx/bootstrap.json`.
    /// Mirrors raiko's `get_guest_data` for on-chain prover registration.
    ///
    /// # Errors
    ///
    /// Returns an error if bootstrap or disk access fails.
    pub async fn get_guest_data(&self) -> RaikoResult<serde_json::Value> {
        self.ensure_bootstrapped().await?;
        guest_data_from_bootstrap()
    }
}

/// Read on-disk TDX bootstrap data and return it shaped for `/guest_data`.
///
/// The bootstrap file is written by [`TdxProver::ensure_bootstrapped`]; if the
/// server has started successfully with the `tdx` feature enabled the file will
/// exist on disk. The returned JSON has one top-level key (the issuer type, e.g.
/// `"tdx"`) whose value is the bootstrap record.
///
/// # Errors
///
/// Returns an error if the bootstrap file is missing or malformed.
pub fn guest_data_from_bootstrap() -> RaikoResult<serde_json::Value> {
    let bootstrap = config::read_bootstrap()
        .map_err(|e| RaikoError::Guest(format!("Failed to read TDX bootstrap data: {e}")))?;

    let issuer_type = config::issuer_proof_type()
        .map_err(|e| RaikoError::Guest(format!("Failed to resolve TDX issuer type: {e}")))?
        .to_string();

    Ok(serde_json::json!({
        issuer_type: {
            "issuer_type": bootstrap.issuer_type,
            "public_key": bootstrap.public_key,
            "quote": bootstrap.quote,
            "nonce": bootstrap.nonce,
            "metadata": bootstrap.metadata,
        }
    }))
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
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        info!("Starting TDX proposal proof generation...");

        self.ensure_bootstrapped().await?;
        config::validate_issuer_type(ProofType::Tdx)
            .map_err(|e| RaikoError::Guest(format!("TDX issuer mismatch: {e}")))?;

        let input: GuestInput = bincode::deserialize(input.as_ref())
            .map_err(|e| RaikoError::Guest(format!("Failed to deserialize input: {e}")))?;

        if input.witnesses.is_empty() {
            return Err(RaikoError::Guest(
                "GuestInput must contain at least one witness".to_string(),
            ));
        }

        let proof_carry_data = input.proof_carry_data;
        let instance_hash = hash_shasta_subproof_input(&proof_carry_data);
        let extra_data = encode_proof_carry_data(&proof_carry_data)?;

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
        _config: &ProverConfig,
        _backend: &B,
    ) -> RaikoResult<Proof> {
        info!(
            "Starting TDX aggregation proof generation with {} proofs...",
            input.proofs.len()
        );

        self.ensure_bootstrapped().await?;
        config::validate_issuer_type(ProofType::Tdx)
            .map_err(|e| RaikoError::Guest(format!("TDX issuer mismatch: {e}")))?;

        let aggregation_input = build_shasta_aggregation_input(&input.proofs)?;

        let sgx_instance = config::load_private_key()
            .map(|key| signature::address_from_private_key(&key))
            .map_err(|e| RaikoError::Guest(format!("Failed to load TDX key: {e}")))?;

        let commitment = build_shasta_commitment_from_proof_carry_data_vec(
            &aggregation_input.proof_carry_data_vec,
        )
        .ok_or_else(|| {
            RaikoError::InvalidRequestConfig(
                "Invalid proof_carry_data_vec for aggregation".to_string(),
            )
        })?;
        let first = aggregation_input
            .proof_carry_data_vec
            .first()
            .ok_or_else(|| {
                RaikoError::InvalidRequestConfig(
                    "Missing proof_carry_data_vec for aggregation".to_string(),
                )
            })?;
        let aggregation_hash =
            shasta_aggregation_output(&commitment, first.chain_id, first.verifier, sgx_instance);

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
