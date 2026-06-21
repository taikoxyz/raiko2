//! TDX TEE access helpers.

use std::{
    fs,
    io::{Read, Write},
    net::Shutdown,
    os::unix::fs::PermissionsExt,
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use alloy_primitives::{Address, keccak256};
use anyhow::{Context, Result, bail};
use getrandom::fill as fill_random;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};

use crate::config::PRIV_KEY_FILENAME;

pub(crate) const DEFAULT_TDXS_SOCKET: &str = "/var/tdxs.sock";
const TDX_NONCE_LEN: usize = 32;

pub trait TeeProvider {
    fn save_private_key(&self, key: &SecretKey) -> Result<()>;
    fn load_private_key(&self) -> Result<SecretKey>;
    fn load_bootstrap_quote(&self, instance_address: Address) -> Result<Vec<u8>>;
    fn load_proof_quote(
        &self,
        instance_address: Address,
        input_hash: alloy_primitives::B256,
    ) -> Result<Vec<u8>>;
}

#[derive(Clone, Debug)]
pub(crate) struct TdxProvider {
    secret_dir: PathBuf,
    socket_path: PathBuf,
}

#[derive(Debug, Serialize)]
struct TdxsRequest<'a, T> {
    method: &'a str,
    data: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TdxsIssueRequestData {
    user_data: String,
    nonce: String,
}

#[derive(Debug, Deserialize)]
struct TdxsIssueResponse {
    data: Option<TdxsIssueResponseData>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TdxsIssueResponseData {
    document: String,
}

impl TdxProvider {
    #[must_use]
    pub(crate) fn new(secret_dir: PathBuf, socket_path: PathBuf) -> Self {
        Self {
            secret_dir,
            socket_path,
        }
    }

    fn private_key_path(&self) -> PathBuf {
        self.secret_dir.join(PRIV_KEY_FILENAME)
    }

    fn load_quote_for_report_data(&self, report_data: [u8; 32]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; TDX_NONCE_LEN];
        fill_random(&mut nonce)
            .map_err(|err| anyhow::anyhow!("generate tdx quote nonce: {err}"))?;
        let request = TdxsRequest {
            method: "issue",
            data: TdxsIssueRequestData {
                user_data: hex::encode(report_data),
                nonce: hex::encode(nonce),
            },
        };
        let response = self.send_tdxs_request(&request)?;
        let response: TdxsIssueResponse =
            serde_json::from_slice(&response).context("decode tdxs issue response")?;
        if let Some(error) = response.error {
            bail!("tdxs issue error: {error}");
        }
        let data = response
            .data
            .ok_or_else(|| anyhow::anyhow!("tdxs issue response missing data"))?;
        hex::decode(data.document).context("decode tdxs document")
    }

    fn send_tdxs_request<T: Serialize>(&self, request: &TdxsRequest<'_, T>) -> Result<Vec<u8>> {
        let payload = serde_json::to_vec(request).context("encode tdxs request")?;
        let mut stream = UnixStream::connect(&self.socket_path)
            .with_context(|| format!("connect tdxs socket {}", self.socket_path.display()))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .context("set tdxs read timeout")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .context("set tdxs write timeout")?;
        stream.write_all(&payload).context("write tdxs request")?;
        let _ = stream.shutdown(Shutdown::Write);
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .context("read tdxs response")?;
        Ok(response)
    }
}

impl TeeProvider for TdxProvider {
    fn save_private_key(&self, key: &SecretKey) -> Result<()> {
        fs::create_dir_all(&self.secret_dir)
            .with_context(|| format!("create secret dir {}", self.secret_dir.display()))?;
        let path = self.private_key_path();
        fs::write(&path, key.secret_bytes())
            .with_context(|| format!("write private key {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("set permissions on {}", path.display()))?;
        Ok(())
    }

    fn load_private_key(&self) -> Result<SecretKey> {
        let path = self.private_key_path();
        let bytes =
            fs::read(&path).with_context(|| format!("read private key {}", path.display()))?;
        SecretKey::from_slice(&bytes).context("decode private key")
    }

    fn load_bootstrap_quote(&self, instance_address: Address) -> Result<Vec<u8>> {
        self.load_quote_for_report_data(tdx_instance_report_data(instance_address))
    }

    fn load_proof_quote(
        &self,
        instance_address: Address,
        input_hash: alloy_primitives::B256,
    ) -> Result<Vec<u8>> {
        self.load_quote_for_report_data(tdx_proof_report_data(instance_address, input_hash))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeProvider;

impl NativeProvider {
    pub(crate) const fn new() -> Self {
        Self
    }

    fn private_key(self) -> Result<SecretKey> {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(native_key_seed().as_slice());
        SecretKey::from_byte_array(&bytes).context("decode native proof private key")
    }
}

impl TeeProvider for NativeProvider {
    fn save_private_key(&self, _key: &SecretKey) -> Result<()> {
        Ok(())
    }

    fn load_private_key(&self) -> Result<SecretKey> {
        self.private_key()
    }

    fn load_bootstrap_quote(&self, _instance_address: Address) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn load_proof_quote(
        &self,
        _instance_address: Address,
        _input_hash: alloy_primitives::B256,
    ) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

fn native_key_seed() -> alloy_primitives::B256 {
    keccak256(b"raiko2:native-tdx-provider")
}

fn tdx_instance_report_data(instance_address: Address) -> [u8; 32] {
    let mut report_data = [0u8; 32];
    report_data[..20].copy_from_slice(instance_address.as_slice());
    report_data
}

fn tdx_proof_report_data(
    instance_address: Address,
    input_hash: alloy_primitives::B256,
) -> [u8; 32] {
    let mut preimage = Vec::with_capacity("raiko2:tdx-proof:v1".len() + 20 + 32);
    preimage.extend_from_slice(b"raiko2:tdx-proof:v1");
    preimage.extend_from_slice(instance_address.as_slice());
    preimage.extend_from_slice(input_hash.as_slice());
    keccak256(preimage).0
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
        path::{Path, PathBuf},
        sync::mpsc,
        thread,
    };

    use alloy_primitives::{Address, B256};
    use serde_json::Value;

    use super::{TdxProvider, TeeProvider, tdx_proof_report_data};

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "raiko2-tdx-runtime-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ))
    }

    fn serve_one_tdxs_request(socket_path: &Path) -> thread::JoinHandle<Value> {
        let socket_path = socket_path.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let listener = UnixListener::bind(&socket_path).expect("bind fake tdxs socket");
            ready_tx.send(()).expect("signal fake tdxs readiness");
            let (mut conn, _) = listener.accept().expect("accept tdxs request");
            let mut payload = Vec::new();
            conn.read_to_end(&mut payload).expect("read tdxs request");
            conn.write_all(br#"{"data":{"document":"abcd"}}"#)
                .expect("write tdxs response");
            serde_json::from_slice(&payload).expect("decode tdxs request")
        });
        ready_rx.recv().expect("fake tdxs socket ready");
        handle
    }

    #[test]
    fn tdx_provider_bootstrap_quote_binds_instance_address() {
        let socket_path = temp_path("bootstrap.sock");
        let request = serve_one_tdxs_request(&socket_path);
        let provider = TdxProvider::new(temp_path("bootstrap-secret"), socket_path);
        let instance_address = Address::repeat_byte(0x11);

        let quote = provider
            .load_bootstrap_quote(instance_address)
            .expect("load bootstrap quote");

        assert_eq!(quote, vec![0xab, 0xcd]);
        let request = request.join().expect("tdxs request");
        assert_eq!(request["method"], "issue");
        assert_eq!(
            request["data"]["userData"],
            format!("{}{}", hex::encode(instance_address), "00".repeat(12))
        );
        assert_eq!(request["data"]["nonce"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn tdx_provider_proof_quote_binds_input_hash() {
        let socket_path = temp_path("proof.sock");
        let request = serve_one_tdxs_request(&socket_path);
        let provider = TdxProvider::new(temp_path("proof-secret"), socket_path);
        let instance_address = Address::repeat_byte(0x22);
        let input_hash = B256::repeat_byte(0x33);

        let quote = provider
            .load_proof_quote(instance_address, input_hash)
            .expect("load proof quote");

        assert_eq!(quote, vec![0xab, 0xcd]);
        let request = request.join().expect("tdxs request");
        let bare_instance_report_data =
            format!("{}{}", hex::encode(instance_address), "00".repeat(12));
        assert_eq!(
            request["data"]["userData"],
            hex::encode(tdx_proof_report_data(instance_address, input_hash))
        );
        assert_ne!(request["data"]["userData"], bare_instance_report_data);
    }
}
