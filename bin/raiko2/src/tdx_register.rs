//! `raiko2 tdx register` — register this prover's TDX instance on-chain.
//!
//! Reads `~/.config/raiko2/tdx/bootstrap.json` (written during prover bootstrap),
//! parses the embedded Azure TDX attestation, and submits two transactions to
//! the `TdxVerifier` contract:
//!
//! * `setTrustedParams` (owner-only) — locks in the running image's hardware
//!   measurements (mrSeam, mrTd, teeTcbSvn, PCRs). Skipped unless `--trust`.
//! * `registerInstance` (permissionless) — admits this VM's bootstrap address
//!   to the registry after on-chain attestation. Default when no flags given.
//!
//! Trusted params are extracted directly from the raw TDX V4 quote inside
//! `attestationDocument.instanceInfo.attestationReport`, using offsets from
//! the Intel TDX DCAP spec. PCRs come straight from the metadata's `pcrs`
//! array.

use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy_primitives::{Bytes, FixedBytes, U256, hex};
use anyhow::{Context, Result, anyhow, bail};
use raiko2_prover::tdx::config::read_bootstrap;
use serde_json::Value;
use tracing::{info, warn};

use crate::cli::TdxRegisterArgs;

// ---------------------------------------------------------------
// Solidity ABI bindings
// ---------------------------------------------------------------

sol! {
    #[sol(rpc)]
    contract TdxVerifier {
        struct TrustedParams {
            bytes16 teeTcbSvn;
            uint24 pcrBitmap;
            bytes mrSeam;
            bytes mrTd;
            bytes32[] pcrs;
        }

        struct PCR {
            uint8 index;
            bytes32 digest;
        }

        struct AkPub {
            uint24 exponentRaw;
            bytes modulusRaw;
        }

        struct RuntimeData {
            bytes raw;
            AkPub hclAkPub;
        }

        struct InstanceInfo {
            bytes attestationReport;
            RuntimeData runtimeData;
        }

        struct TPMQuote {
            bytes quote;
            bytes rsaSignature;
            bytes32[24] pcrs;
        }

        struct Attestation {
            TPMQuote tpmQuote;
        }

        struct AttestationDocument {
            Attestation attestation;
            InstanceInfo instanceInfo;
            bytes userData;
        }

        struct VerifyParams {
            AttestationDocument attestationDocument;
            PCR[] pcrs;
            bytes nonce;
        }

        function setTrustedParams(uint256 index, TrustedParams calldata params) external;
        function registerInstance(uint256 trustedParamsIdx, VerifyParams memory attestation)
            external
            returns (uint256);
        function trustedParams(uint256 index)
            external
            view
            returns (bytes16, uint24, bytes memory, bytes memory);
    }
}

// ---------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------

/// Run the `raiko2 tdx register` command.
pub async fn run(args: TdxRegisterArgs) -> Result<()> {
    // Default behaviour: if neither flag set, register only.
    let do_trust = args.trust;
    let do_register = args.register || !args.trust;

    info!(
        "raiko2 tdx register: verifier={:?} index={} trust={} register={} dry_run={}",
        args.verifier, args.trusted_params_index, do_trust, do_register, args.dry_run
    );

    // Load bootstrap data written by `TdxProver::ensure_bootstrapped`.
    let bootstrap =
        read_bootstrap().context("failed to read TDX bootstrap data — has the prover booted?")?;
    let metadata = &bootstrap.metadata;
    info!(
        "loaded bootstrap.json (issuer={} public_key={})",
        bootstrap.issuer_type, bootstrap.public_key
    );

    let attestation = parse_verify_params(metadata)
        .context("failed to parse attestation document from bootstrap metadata")?;

    let signer: PrivateKeySigner = args
        .private_key
        .trim_start_matches("0x")
        .parse()
        .context("invalid --private-key value")?;

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(args.rpc.parse().context("invalid --rpc URL")?);

    let contract = TdxVerifier::new(args.verifier, provider);

    if do_trust {
        let params = extract_trusted_params(metadata, args.pcr_bitmap)
            .context("failed to extract trusted params from attestation report")?;

        info!(
            "setTrustedParams(index={}, teeTcbSvn=0x{}, mrSeam=0x{}, mrTd=0x{}, pcrs=[{} entries])",
            args.trusted_params_index,
            hex::encode(params.teeTcbSvn.0),
            hex::encode(&params.mrSeam[..]),
            hex::encode(&params.mrTd[..]),
            params.pcrs.len()
        );

        if args.dry_run {
            info!("dry-run: skipping setTrustedParams broadcast");
        } else {
            let call = contract.setTrustedParams(U256::from(args.trusted_params_index), params);
            let receipt = call
                .send()
                .await
                .context("setTrustedParams send failed")?
                .get_receipt()
                .await
                .context("setTrustedParams receipt failed")?;
            info!(
                "setTrustedParams confirmed in block {:?} (tx {:?})",
                receipt.block_number, receipt.transaction_hash
            );
        }
    } else {
        // Pre-flight: if not trusting now, the on-chain `params.pcrBitmap` must already be set.
        if !args.dry_run {
            let on_chain = contract
                .trustedParams(U256::from(args.trusted_params_index))
                .call()
                .await
                .context("failed to read on-chain trustedParams")?;
            if on_chain._1.is_zero() {
                bail!(
                    "trustedParams[{}] not set on-chain — run with --trust (as contract owner) first",
                    args.trusted_params_index
                );
            }
        }
    }

    if do_register {
        info!(
            "registerInstance(trustedParamsIdx={}) — instance address = {}",
            args.trusted_params_index, bootstrap.public_key
        );

        if args.dry_run {
            info!("dry-run: skipping registerInstance broadcast");
        } else {
            let call =
                contract.registerInstance(U256::from(args.trusted_params_index), attestation);
            let receipt = call
                .send()
                .await
                .context("registerInstance send failed")?
                .get_receipt()
                .await
                .context("registerInstance receipt failed")?;
            info!(
                "registerInstance confirmed in block {:?} (tx {:?})",
                receipt.block_number, receipt.transaction_hash
            );
        }
    }

    Ok(())
}

// ---------------------------------------------------------------
// JSON → sol type conversion (registerInstance input)
// ---------------------------------------------------------------

fn parse_verify_params(metadata: &Value) -> Result<TdxVerifier::VerifyParams> {
    let doc = metadata
        .get("attestationDocument")
        .ok_or_else(|| anyhow!("metadata.attestationDocument missing"))?;
    let attestation = doc
        .get("attestation")
        .ok_or_else(|| anyhow!("metadata.attestationDocument.attestation missing"))?;
    let tpm = attestation
        .get("tpmQuote")
        .ok_or_else(|| anyhow!("metadata.attestationDocument.attestation.tpmQuote missing"))?;
    let info = doc
        .get("instanceInfo")
        .ok_or_else(|| anyhow!("metadata.attestationDocument.instanceInfo missing"))?;
    let rd = info
        .get("runtimeData")
        .ok_or_else(|| anyhow!("metadata.attestationDocument.instanceInfo.runtimeData missing"))?;
    let ak = rd.get("hclAkPub").ok_or_else(|| {
        anyhow!("metadata.attestationDocument.instanceInfo.runtimeData.hclAkPub missing")
    })?;

    let tpm_pcrs_json = tpm
        .get("pcrs")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("tpmQuote.pcrs must be an array of 24"))?;
    if tpm_pcrs_json.len() != 24 {
        bail!("tpmQuote.pcrs has {} entries, expected 24", tpm_pcrs_json.len());
    }
    let mut tpm_pcrs: [FixedBytes<32>; 24] = [FixedBytes::ZERO; 24];
    for (i, p) in tpm_pcrs_json.iter().enumerate() {
        tpm_pcrs[i] = FixedBytes::from_slice(&parse_hex(p, &format!("tpmQuote.pcrs[{i}]"))?);
    }

    let pcrs_json = metadata
        .get("pcrs")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("metadata.pcrs missing or not an array"))?;
    let pcrs = pcrs_json
        .iter()
        .map(|p| {
            let index = p
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow!("pcrs[].index missing"))?;
            let digest = parse_hex(
                p.get("digest").ok_or_else(|| anyhow!("pcrs[].digest missing"))?,
                "pcrs[].digest",
            )?;
            Ok(TdxVerifier::PCR {
                index: index as u8,
                digest: FixedBytes::from_slice(&digest),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(TdxVerifier::VerifyParams {
        attestationDocument: TdxVerifier::AttestationDocument {
            attestation: TdxVerifier::Attestation {
                tpmQuote: TdxVerifier::TPMQuote {
                    quote: Bytes::from(parse_hex(&tpm["quote"], "tpmQuote.quote")?),
                    rsaSignature: Bytes::from(parse_hex(
                        &tpm["rsaSignature"],
                        "tpmQuote.rsaSignature",
                    )?),
                    pcrs: tpm_pcrs,
                },
            },
            instanceInfo: TdxVerifier::InstanceInfo {
                attestationReport: Bytes::from(parse_hex(
                    &info["attestationReport"],
                    "instanceInfo.attestationReport",
                )?),
                runtimeData: TdxVerifier::RuntimeData {
                    raw: Bytes::from(parse_hex(&rd["raw"], "runtimeData.raw")?),
                    hclAkPub: TdxVerifier::AkPub {
                        exponentRaw: alloy_primitives::aliases::U24::from(
                            ak.get("exponentRaw")
                                .and_then(Value::as_u64)
                                .ok_or_else(|| anyhow!("hclAkPub.exponentRaw missing"))?,
                        ),
                        modulusRaw: Bytes::from(parse_hex(
                            &ak["modulusRaw"],
                            "hclAkPub.modulusRaw",
                        )?),
                    },
                },
            },
            userData: Bytes::from(parse_hex(&doc["userData"], "attestationDocument.userData")?),
        },
        pcrs,
        nonce: Bytes::from(parse_hex(
            metadata.get("nonce").ok_or_else(|| anyhow!("metadata.nonce missing"))?,
            "metadata.nonce",
        )?),
    })
}

// ---------------------------------------------------------------
// Trusted params extraction (TDX V4 quote → on-chain TrustedParams)
// ---------------------------------------------------------------

// Offsets into a TDX V4 DCAP quote (Intel TDX DCAP spec § "TD Quote V4 Body").
// The quote starts with a 48-byte header followed by the 584-byte TD report body.
const TDX_QUOTE_HEADER_LEN: usize = 48;
const TDX_BODY_TEE_TCB_SVN: usize = 0; // 16 bytes
const TDX_BODY_MR_SEAM: usize = 16; // 48 bytes
const TDX_BODY_MR_TD: usize = 136; // 48 bytes

fn extract_trusted_params(metadata: &Value, pcr_bitmap: u32) -> Result<TdxVerifier::TrustedParams> {
    let report_hex = metadata
        .pointer("/attestationDocument/instanceInfo/attestationReport")
        .ok_or_else(|| anyhow!("metadata.attestationDocument.instanceInfo.attestationReport missing"))?;
    let report = parse_hex(report_hex, "attestationReport")?;

    if report.len() < TDX_QUOTE_HEADER_LEN + TDX_BODY_MR_TD + 48 {
        bail!(
            "attestationReport too short ({} bytes); not a TDX V4 quote",
            report.len()
        );
    }

    // Sanity check: TEE version (0x04, LE 0x0004) at byte 0, TEE type (0x81 family).
    if &report[0..2] != [0x04, 0x00] {
        warn!(
            "attestationReport version is 0x{:02x}{:02x} — expected TDX V4 (0x04 0x00); proceeding anyway",
            report[0], report[1]
        );
    }

    let body = &report[TDX_QUOTE_HEADER_LEN..];

    let tee_tcb_svn: [u8; 16] = body[TDX_BODY_TEE_TCB_SVN..TDX_BODY_TEE_TCB_SVN + 16]
        .try_into()
        .map_err(|_| anyhow!("teeTcbSvn slice"))?;
    let mr_seam = body[TDX_BODY_MR_SEAM..TDX_BODY_MR_SEAM + 48].to_vec();
    let mr_td = body[TDX_BODY_MR_TD..TDX_BODY_MR_TD + 48].to_vec();

    // PCR digests live in metadata.pcrs[i].digest, indexed by TPM PCR number.
    // Pick the ones whose bit is set in `pcr_bitmap`, in ascending index order.
    let pcrs_json = metadata
        .get("pcrs")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("metadata.pcrs missing"))?;
    let mut by_index = [None::<[u8; 32]>; 24];
    for p in pcrs_json {
        let idx = p
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("pcrs[].index missing"))? as usize;
        if idx >= 24 {
            continue;
        }
        let digest = parse_hex(
            p.get("digest").ok_or_else(|| anyhow!("pcrs[].digest missing"))?,
            "pcrs[].digest",
        )?;
        let bytes: [u8; 32] = digest
            .try_into()
            .map_err(|d: Vec<u8>| anyhow!("pcrs[{idx}].digest must be 32 bytes, got {}", d.len()))?;
        by_index[idx] = Some(bytes);
    }
    let mut selected_pcrs = Vec::new();
    for i in 0..24 {
        if pcr_bitmap & (1 << i) != 0 {
            let d = by_index[i]
                .ok_or_else(|| anyhow!("pcr_bitmap bit {i} set but pcrs[{i}] missing from metadata"))?;
            selected_pcrs.push(FixedBytes::from(d));
        }
    }

    Ok(TdxVerifier::TrustedParams {
        teeTcbSvn: FixedBytes::from(tee_tcb_svn),
        pcrBitmap: alloy_primitives::aliases::U24::from(pcr_bitmap),
        mrSeam: Bytes::from(mr_seam),
        mrTd: Bytes::from(mr_td),
        pcrs: selected_pcrs,
    })
}

// ---------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------

fn parse_hex(value: &Value, field: &str) -> Result<Vec<u8>> {
    let s = value
        .as_str()
        .ok_or_else(|| anyhow!("{field} must be a hex string"))?;
    let s = s.trim_start_matches("0x");
    hex::decode(s).with_context(|| format!("{field} not valid hex"))
}
