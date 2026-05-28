//! `cargo xtask register-tdx` — register a TDX prover instance on-chain.
//!
//! Fetches bootstrap data from a running reth-tdx instance
//! (`GET <reth-tdx-url>/bootstrap`) or reads `~/.config/reth-tdx/bootstrap.json`
//! from the local filesystem, then submits up to two transactions to the
//! `AzureTdxVerifier` contract:
//!
//! * `setTrustedParams` (owner-only) — locks in the running image's hardware
//!   measurements (mrSeam, mrTd, teeTcbSvn, PCRs). Skipped unless `--trust`.
//! * `registerInstance` (permissionless) — admits this VM's bootstrap address
//!   to the registry after on-chain attestation. Default when no flags given.

use std::fs;
use std::path::PathBuf;

use alloy::primitives::{Bytes, FixedBytes, U256, hex};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use clap::Args;
use serde::Deserialize;
use serde_json::Value;

// ---------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------

#[derive(Args)]
pub(crate) struct RegisterTdxArgs {
    /// Address of the deployed `AzureTdxVerifier` contract.
    #[arg(long, env = "TDX_VERIFIER")]
    verifier: alloy::primitives::Address,

    /// L1 RPC URL where the verifier lives.
    #[arg(long, env = "TDX_RPC")]
    rpc: String,

    /// Private key used to sign the registration transactions (hex, with or without 0x).
    ///
    /// For `--trust`, this must be the verifier owner. For `--register` alone, any
    /// funded account works.
    #[arg(long, env = "PRIVATE_KEY")]
    private_key: String,

    /// Fetch bootstrap data from a running reth-tdx instance instead of reading
    /// the local `~/.config/reth-tdx/bootstrap.json`.
    ///
    /// Example: `--reth-tdx-url http://localhost:8080`
    ///
    /// The CLI appends `/bootstrap` and parses the JSON response.
    #[arg(long, env = "TDX_RETH_URL", alias = "raiko-url")]
    reth_tdx_url: Option<String>,

    /// Trusted params slot to write/read. Use the same index for `--trust` and
    /// `--register` in one run; pick a fresh index for new image versions.
    #[arg(long, default_value_t = 0)]
    trusted_params_index: u64,

    /// PCR bitmap (24-bit mask of TPM PCR indices to bind into trusted params).
    /// Default `0xBA10` = PCRs 4, 9, 11, 12, 13, 15 (matches the Azure paravisor
    /// reference set).
    #[arg(long, default_value_t = 0xBA10)]
    pcr_bitmap: u32,

    /// Owner-only: also call `setTrustedParams` with the measurements extracted
    /// from this VM.
    #[arg(long)]
    trust: bool,

    /// Permissionless: call `registerInstance`. Defaults to true when neither
    /// flag is set so a bare `register-tdx` does the common case.
    #[arg(long)]
    register: bool,

    /// Print the transactions without broadcasting.
    #[arg(long)]
    dry_run: bool,
}

// ---------------------------------------------------------------
// Bootstrap data (mirrors reth-tdx's persistence::BootstrapData)
// ---------------------------------------------------------------

// reth-tdx writes `~/.config/reth-tdx/bootstrap.json` with camelCase keys
// (`#[serde(rename_all = "camelCase")]` in `reth_tdx::persistence::BootstrapData`)
// and serves the same record with snake_case keys at `GET /bootstrap`. We accept
// both casings via serde aliases.
#[derive(Deserialize)]
struct BootstrapData {
    #[serde(alias = "issuerType")]
    issuer_type: String,
    #[serde(alias = "publicKey")]
    public_key: String,
    quote: String,
    nonce: String,
    metadata: Value,
}

// ---------------------------------------------------------------
// Solidity ABI bindings
// ---------------------------------------------------------------

sol! {
    #[sol(rpc)]
    contract AzureTdxVerifier {
        struct TrustedParams {
            bytes16 teeTcbSvn;
            uint24 pcrBitmap;
            bytes mrSeam;
            bytes mrTd;
            bytes32[] pcrs;
        }

        struct PCR {
            uint256 index;
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

pub(crate) async fn run(args: RegisterTdxArgs) -> Result<()> {
    let do_trust = args.trust;
    let do_register = args.register || !args.trust;

    if args.pcr_bitmap >= (1u32 << 24) {
        anyhow::bail!(
            "--pcr-bitmap={:#x} exceeds 24-bit range; high bits would be silently truncated when packed into the Solidity uint24",
            args.pcr_bitmap
        );
    }

    println!(
        "register-tdx: verifier={:?} index={} trust={} register={} dry_run={}",
        args.verifier, args.trusted_params_index, do_trust, do_register, args.dry_run
    );

    let bootstrap = match &args.reth_tdx_url {
        Some(url) => fetch_bootstrap(url)
            .await
            .context("failed to fetch TDX bootstrap data from reth-tdx")?,
        None => read_bootstrap_from_disk()
            .context("failed to read TDX bootstrap data — has reth-tdx booted?")?,
    };
    let metadata = &bootstrap.metadata;
    println!(
        "loaded bootstrap data (issuer={} public_key={})",
        bootstrap.issuer_type, bootstrap.public_key
    );

    let attestation = parse_verify_params(metadata, &bootstrap.quote, &bootstrap.nonce)
        .context("failed to parse attestation document from bootstrap metadata")?;

    let signer: PrivateKeySigner = args
        .private_key
        .trim_start_matches("0x")
        .parse()
        .context("invalid --private-key value")?;

    let rpc_url = if args.rpc.starts_with("http://") || args.rpc.starts_with("https://") {
        args.rpc.clone()
    } else {
        format!("http://{}", args.rpc)
    };
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(rpc_url.parse().context("invalid --rpc URL")?);

    let contract = AzureTdxVerifier::new(args.verifier, provider);

    if do_trust {
        let params = extract_trusted_params(metadata, &bootstrap.quote, args.pcr_bitmap)
            .context("failed to extract trusted params from attestation report")?;

        println!(
            "setTrustedParams(index={}, teeTcbSvn=0x{}, mrSeam=0x{}, mrTd=0x{}, pcrs=[{} entries])",
            args.trusted_params_index,
            hex::encode(params.teeTcbSvn.0),
            hex::encode(&params.mrSeam[..]),
            hex::encode(&params.mrTd[..]),
            params.pcrs.len()
        );

        if args.dry_run {
            println!("dry-run: skipping setTrustedParams broadcast");
        } else {
            let receipt = contract
                .setTrustedParams(U256::from(args.trusted_params_index), params)
                .send()
                .await
                .context("setTrustedParams send failed")?
                .get_receipt()
                .await
                .context("setTrustedParams receipt failed")?;
            if !receipt.status() {
                bail!("setTrustedParams transaction reverted (status=0)");
            }
            println!(
                "setTrustedParams OK — block {:?} tx {:?}",
                receipt.block_number.unwrap_or_default(),
                receipt.transaction_hash
            );
        }
    } else if !args.dry_run {
        let on_chain = contract
            .trustedParams(U256::from(args.trusted_params_index))
            .call()
            .await
            .context("failed to read on-chain trustedParams")?;
        // A configured slot has both measurement bytes populated (TDX mrSeam/mrTd are
        // always 48 bytes when written). Using pcrBitmap=0 to detect "not set" would
        // misclassify slots that an owner legitimately set with a zero bitmap.
        if on_chain._2.is_empty() && on_chain._3.is_empty() {
            bail!(
                "trustedParams[{}] not set on-chain — run with --trust (as contract owner) first",
                args.trusted_params_index
            );
        }
    }

    if do_register {
        println!(
            "registerInstance(trustedParamsIdx={}) — instance address = {}",
            args.trusted_params_index, bootstrap.public_key
        );

        let call = contract.registerInstance(U256::from(args.trusted_params_index), attestation);
        let calldata_len = call.calldata().len();
        println!("registerInstance calldata: {} bytes", calldata_len);

        if args.dry_run {
            println!("dry-run: skipping registerInstance broadcast");
        } else {
            let receipt = call
                .send()
                .await
                .context("registerInstance send failed")?
                .get_receipt()
                .await
                .context("registerInstance receipt failed")?;
            println!(
                "registerInstance confirmed in block {:?} (tx {:?})",
                receipt.block_number, receipt.transaction_hash
            );

            if !receipt.status() {
                bail!("registerInstance transaction reverted (status=0)");
            }

            println!();
            println!("=======================================");
            println!("  TDX PROVER REGISTERED");
            println!("  AzureTdxVerifier:   {}", args.verifier);
            println!("  Instance address:   {}", bootstrap.public_key);
            println!("  trustedParamsIndex: {}", args.trusted_params_index);
            println!(
                "  Block:              {:?}",
                receipt.block_number.unwrap_or_default()
            );
            println!("  Tx:                 {:?}", receipt.transaction_hash);
            println!("=======================================");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------
// Bootstrap loading
// ---------------------------------------------------------------

async fn fetch_bootstrap(reth_tdx_url: &str) -> Result<BootstrapData> {
    let url = format!("{}/bootstrap", reth_tdx_url.trim_end_matches('/'));
    println!("fetching TDX bootstrap from {url}");

    // reth-tdx's `GET /bootstrap` returns the bootstrap record directly (flat
    // object) — no issuer-type wrapping. Serde aliases handle camelCase vs
    // snake_case for forward-compat with the on-disk record format.
    reqwest::get(&url)
        .await
        .with_context(|| format!("GET {url} failed"))?
        .error_for_status()
        .with_context(|| format!("GET {url} returned error status"))?
        .json::<BootstrapData>()
        .await
        .with_context(|| format!("failed to parse JSON from {url}"))
}

fn read_bootstrap_from_disk() -> Result<BootstrapData> {
    let path = bootstrap_path()?;
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| anyhow!("failed to parse bootstrap data: {e}"))
}

fn bootstrap_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("failed to get home directory"))?;
    Ok(home.join(".config").join("reth-tdx").join("bootstrap.json"))
}

// ---------------------------------------------------------------
// JSON → sol type conversion (registerInstance input)
//
// The bootstrap metadata.quote field is a hex-encoded JSON string with this
// structure (Azure vTPM / MAA attestation format):
//
// {
//   "Attestation": {
//     "ak_pub": "<b64 TPM2B_PUBLIC>",
//     "quotes": [
//       { "quote": "<b64>", "raw_sig": "<b64>", "pcrs": { "hash": 11, "pcrs": {"0": "<b64>", ...} } },
//       ...
//     ],
//     "event_log": "...",
//     ...
//   },
//   "InstanceInfo": "<b64 JSON { AttestationReport: <b64>, RuntimeData: <b64 JWK JSON> }>",
//   "UserData": "<b64>"
// }
//
// We pick the quote bank with SHA-256 PCRs (hash == 11, 32-byte values).
// ---------------------------------------------------------------

fn decode_b64(value: &Value, field: &str) -> Result<Vec<u8>> {
    let s = value
        .as_str()
        .ok_or_else(|| anyhow!("{field} must be a base64 string, got: {:?}", value))?;
    // standard b64 (possibly without padding)
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(s))
        .with_context(|| format!("{field}: base64 decode failed"))
}

fn decode_b64_str(s: &str, field: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(s))
        .with_context(|| format!("{field}: base64 decode failed"))
}

/// Parse the `quote` + `metadata` from bootstrap into a `VerifyParams` for
/// `registerInstance`. The `quote` field is a hex-encoded JSON blob (see above).
/// `bootstrap_nonce_hex` is the hex-encoded 32-byte random nonce stored in `bootstrap.nonce`
/// (not the inner metadata nonce, which may differ or be absent).
fn parse_verify_params(
    metadata: &Value,
    bootstrap_quote: &str,
    bootstrap_nonce_hex: &str,
) -> Result<AzureTdxVerifier::VerifyParams> {
    // Decode the outer hex wrapper
    let quote_hex = bootstrap_quote.trim_start_matches("0x");
    let doc_bytes = hex::decode(quote_hex).context("bootstrap quote: not valid hex")?;
    let doc: Value =
        serde_json::from_slice(&doc_bytes).context("bootstrap quote: not valid JSON")?;

    // ---- Attestation / TPM quotes ----
    let att = doc
        .get("Attestation")
        .ok_or_else(|| anyhow!("quote doc missing Attestation"))?;

    let quotes = att
        .get("quotes")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Attestation.quotes missing or not array"))?;

    // Pick the SHA-256 (hash alg 11) bank for 32-byte PCRs
    let sha256_quote = quotes
        .iter()
        .find(|q| {
            q.pointer("/pcrs/hash")
                .and_then(Value::as_u64)
                .map(|h| h == 11)
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("no SHA-256 (hash=11) PCR bank in quotes"))?;

    let tpm_quote_bytes = decode_b64(&sha256_quote["quote"], "tpmQuote.quote")?;
    // raw_sig is TPMT_SIGNATURE: sigAlg(2) || hashAlg(2) || sigLen(2) || sig[sigLen].
    // The Solidity RSA.pkcs1Sha256 expects the bare RSA signature (same length as modulus).
    // Strip the 6-byte header to get the raw signature.
    let tpm_sig_raw = decode_b64(&sha256_quote["raw_sig"], "tpmQuote.raw_sig")?;
    if tpm_sig_raw.len() < 6 {
        bail!("tpmQuote.raw_sig too short: {} bytes", tpm_sig_raw.len());
    }
    let sig_len = u16::from_be_bytes([tpm_sig_raw[4], tpm_sig_raw[5]]) as usize;
    if 6 + sig_len > tpm_sig_raw.len() {
        bail!(
            "tpmQuote.raw_sig: declared sigLen={sig_len} exceeds buffer ({})",
            tpm_sig_raw.len()
        );
    }
    let tpm_sig_bytes = tpm_sig_raw[6..6 + sig_len].to_vec();

    let sha256_pcrs = sha256_quote
        .pointer("/pcrs/pcrs")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("SHA-256 bank pcrs.pcrs missing or not object"))?;

    if sha256_pcrs.len() != 24 {
        bail!(
            "SHA-256 PCR bank has {} entries, expected 24",
            sha256_pcrs.len()
        );
    }
    let mut tpm_pcrs: [FixedBytes<32>; 24] = [FixedBytes::ZERO; 24];
    for (k, v) in sha256_pcrs.iter() {
        let idx: usize = k
            .parse()
            .with_context(|| format!("invalid PCR key '{k}'"))?;
        if idx >= 24 {
            bail!("PCR key {idx} out of range");
        }
        let raw = decode_b64(v, &format!("pcrs[{idx}]"))?;
        if raw.len() != 32 {
            bail!("pcrs[{idx}] is {} bytes, expected 32", raw.len());
        }
        tpm_pcrs[idx] = FixedBytes::from_slice(&raw);
    }

    // ---- ak_pub → AkPub ----
    // Resolve n,e from the JWK embedded in InstanceInfo.RuntimeData
    // (ak_pub here is raw TPM2B_PUBLIC binary; the JWK is in InstanceInfo)

    // ---- InstanceInfo ----
    let inst_b64 = doc
        .get("InstanceInfo")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("quote doc missing InstanceInfo"))?;
    let inst_bytes = decode_b64_str(inst_b64, "InstanceInfo")?;
    let inst: Value =
        serde_json::from_slice(&inst_bytes).context("InstanceInfo: not valid JSON")?;

    let att_report = decode_b64(
        inst.get("AttestationReport")
            .ok_or_else(|| anyhow!("InstanceInfo.AttestationReport missing"))?,
        "AttestationReport",
    )?;

    let rd_b64 = inst
        .get("RuntimeData")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("InstanceInfo.RuntimeData missing or not string"))?;
    let rd_bytes = decode_b64_str(rd_b64, "RuntimeData")?;
    let rd: Value = serde_json::from_slice(&rd_bytes).context("RuntimeData: not valid JSON")?;

    // Find the HCLAkPub key in the JWK set
    let hcl_ak = rd
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter()
                .find(|k| k.get("kid").and_then(Value::as_str) == Some("HCLAkPub"))
        })
        .ok_or_else(|| anyhow!("RuntimeData.keys[HCLAkPub] not found"))?;

    let e_val = hcl_ak
        .get("e")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("HCLAkPub.e missing"))?;
    let e_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(e_val)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(e_val))
        .context("HCLAkPub.e: urlsafe b64 decode failed")?;
    // JWK exponent is big-endian. Standard RSA exponents (e.g. 65537 = 0x010001)
    // are at most 4 bytes. Reject anything longer with non-zero high bytes rather
    // than silently truncating, which would produce an incorrect uint24 on-chain.
    if e_bytes.len() > 4 && e_bytes[..e_bytes.len() - 4].iter().any(|&b| b != 0) {
        bail!(
            "JWK exponent is {} bytes with non-zero high bytes — not a standard RSA exponent \
             and cannot be represented as uint24 without silent truncation",
            e_bytes.len()
        );
    }
    let mut e_padded = [0u8; 4];
    let start = e_bytes.len().saturating_sub(4);
    e_padded[4 - (e_bytes.len() - start)..].copy_from_slice(&e_bytes[start..]);
    let exponent = u32::from_be_bytes(e_padded);
    if exponent > 0xFF_FFFF {
        bail!(
            "JWK exponent {exponent:#010x} exceeds 24 bits (max 0x00FFFFFF) — \
             on-chain uint24 would truncate it, causing AK public-key mismatch and \
             a hard-to-diagnose verification failure"
        );
    }

    let n_val = hcl_ak
        .get("n")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("HCLAkPub.n missing"))?;
    let n_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(n_val)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(n_val))
        .context("HCLAkPub.n: urlsafe b64 decode failed")?;

    // ---- UserData ----
    let ud_b64 = doc
        .get("UserData")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("quote doc missing UserData"))?;
    let ud_bytes = decode_b64_str(ud_b64, "UserData")?;

    // ---- metadata.pcrs (array of {index, digest}) — built from SHA-256 bank ----
    // The metadata may have pcrs as a dict {"0": "0x...", ...} (32-byte hex)
    let meta_pcrs_val = metadata.get("pcrs");
    let pcrs: Vec<AzureTdxVerifier::PCR> =
        if let Some(meta_pcrs) = meta_pcrs_val.and_then(Value::as_object) {
            let mut out = Vec::new();
            for (k, v) in meta_pcrs.iter() {
                let idx: u64 = k
                    .parse()
                    .with_context(|| format!("metadata.pcrs key '{k}' not a number"))?;
                let raw = parse_hex(v, &format!("metadata.pcrs[{k}]"))?;
                if raw.len() != 32 {
                    continue; // skip non-SHA256 entries
                }
                out.push(AzureTdxVerifier::PCR {
                    index: U256::from(idx),
                    digest: FixedBytes::from_slice(&raw),
                });
            }
            out.sort_by_key(|p| p.index);
            out
        } else {
            // fallback: use tpm_pcrs from the SHA-256 quote bank
            (0u64..24u64)
                .map(|i| AzureTdxVerifier::PCR {
                    index: U256::from(i),
                    digest: tpm_pcrs[i as usize],
                })
                .collect()
        };

    // ---- nonce ----
    // Use the bootstrap-level nonce (the random 32 bytes passed to issue_attestation).
    // This is what the contract recomputes in _makeExtraData; using the wrong nonce
    // (e.g. from the inner metadata object) will cause ExtraDataMismatch on-chain.
    let nonce_bytes = hex::decode(bootstrap_nonce_hex.trim_start_matches("0x"))
        .context("bootstrap.nonce: not valid hex")?;
    if nonce_bytes.len() != 32 {
        anyhow::bail!(
            "bootstrap.nonce must be 32 bytes; got {} bytes — the contract's _makeExtraData expects a 32-byte nonce and will revert with ExtraDataMismatch otherwise",
            nonce_bytes.len()
        );
    }

    Ok(AzureTdxVerifier::VerifyParams {
        attestationDocument: AzureTdxVerifier::AttestationDocument {
            attestation: AzureTdxVerifier::Attestation {
                tpmQuote: AzureTdxVerifier::TPMQuote {
                    quote: Bytes::from(tpm_quote_bytes),
                    rsaSignature: Bytes::from(tpm_sig_bytes),
                    pcrs: tpm_pcrs,
                },
            },
            instanceInfo: AzureTdxVerifier::InstanceInfo {
                attestationReport: Bytes::from(att_report),
                runtimeData: AzureTdxVerifier::RuntimeData {
                    raw: Bytes::from(rd_bytes),
                    hclAkPub: AzureTdxVerifier::AkPub {
                        exponentRaw: alloy::primitives::aliases::U24::from(exponent),
                        modulusRaw: Bytes::from(n_bytes),
                    },
                },
            },
            userData: Bytes::from(ud_bytes),
        },
        pcrs,
        nonce: Bytes::from(nonce_bytes),
    })
}

// ---------------------------------------------------------------
// Trusted params extraction (TDX attestation report → on-chain TrustedParams)
//
// The attestationReport in InstanceInfo is the raw TDX V4 DCAP quote (5006 bytes).
// We extract teeTcbSvn, mrSeam, mrTd from standard offsets and use the SHA-256
// (hash=11) PCR bank for the PCR values.
// ---------------------------------------------------------------

// Offsets into a TDX V4 DCAP quote (Intel TDX DCAP spec § "TD Quote V4 Body").
const TDX_QUOTE_HEADER_LEN: usize = 48;
const TDX_BODY_TEE_TCB_SVN: usize = 0; // 16 bytes
const TDX_BODY_MR_SEAM: usize = 16; // 48 bytes
const TDX_BODY_MR_TD: usize = 136; // 48 bytes

fn extract_trusted_params(
    metadata: &Value,
    bootstrap_quote: &str,
    pcr_bitmap: u32,
) -> Result<AzureTdxVerifier::TrustedParams> {
    // Decode the outer hex wrapper to get the attestation doc JSON
    let quote_hex = bootstrap_quote.trim_start_matches("0x");
    let doc_bytes = hex::decode(quote_hex).context("bootstrap quote: not valid hex")?;
    let doc: Value =
        serde_json::from_slice(&doc_bytes).context("bootstrap quote: not valid JSON")?;

    // ---- AttestationReport → teeTcbSvn, mrSeam, mrTd ----
    let inst_b64 = doc
        .get("InstanceInfo")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("quote doc missing InstanceInfo"))?;
    let inst_bytes = decode_b64_str(inst_b64, "InstanceInfo")?;
    let inst: Value =
        serde_json::from_slice(&inst_bytes).context("InstanceInfo: not valid JSON")?;

    let report = decode_b64(
        inst.get("AttestationReport")
            .ok_or_else(|| anyhow!("InstanceInfo.AttestationReport missing"))?,
        "AttestationReport",
    )?;

    if report.len() < TDX_QUOTE_HEADER_LEN + TDX_BODY_MR_TD + 48 {
        bail!(
            "attestationReport too short ({} bytes); not a TDX V4 quote",
            report.len()
        );
    }

    // The mrSeam / mrTd / teeTcbSvn slices below are at TDX V4 body offsets.
    // A non-V4 quote would silently slice unrelated bytes and write incorrect
    // measurements on-chain via `setTrustedParams`, so refuse to proceed.
    if &report[0..2] != [0x04, 0x00] {
        bail!(
            "attestationReport version is 0x{:02x}{:02x} — expected TDX V4 (0x04 0x00); \
             refusing to extract trusted params from an unknown quote layout",
            report[0],
            report[1]
        );
    }

    let body = &report[TDX_QUOTE_HEADER_LEN..];
    let tee_tcb_svn: [u8; 16] = body[TDX_BODY_TEE_TCB_SVN..TDX_BODY_TEE_TCB_SVN + 16]
        .try_into()
        .map_err(|_| anyhow!("teeTcbSvn slice"))?;
    let mr_seam = body[TDX_BODY_MR_SEAM..TDX_BODY_MR_SEAM + 48].to_vec();
    let mr_td = body[TDX_BODY_MR_TD..TDX_BODY_MR_TD + 48].to_vec();

    // ---- PCRs from metadata dict {"0": "0x<hex32>", ...} ----
    let pcrs_obj = metadata
        .get("pcrs")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("metadata.pcrs missing or not an object"))?;

    let mut by_index = [None::<[u8; 32]>; 24];
    for (k, v) in pcrs_obj.iter() {
        let idx: usize = k
            .parse()
            .with_context(|| format!("metadata.pcrs key '{k}' not a number"))?;
        if idx >= 24 {
            continue;
        }
        let digest = parse_hex(v, &format!("metadata.pcrs[{k}]"))?;
        let bytes: [u8; 32] = digest.try_into().map_err(|d: Vec<u8>| {
            anyhow!("metadata.pcrs[{idx}] must be 32 bytes, got {}", d.len())
        })?;
        by_index[idx] = Some(bytes);
    }

    let mut selected_pcrs = Vec::new();
    for i in 0..24 {
        if pcr_bitmap & (1 << i) != 0 {
            let d = by_index[i]
                .ok_or_else(|| anyhow!("pcr_bitmap bit {i} set but metadata.pcrs[{i}] missing"))?;
            selected_pcrs.push(FixedBytes::from(d));
        }
    }

    Ok(AzureTdxVerifier::TrustedParams {
        teeTcbSvn: FixedBytes::from(tee_tcb_svn),
        pcrBitmap: alloy::primitives::aliases::U24::from(pcr_bitmap),
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
