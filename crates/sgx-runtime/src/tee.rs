//! Gramine-backed SGX TEE access helpers.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

use alloy_primitives::Address;
use anyhow::{Context, Result};
use secp256k1::SecretKey;

use crate::config::PRIV_KEY_FILENAME;

const ATTESTATION_QUOTE_DEVICE_FILE: &str = "/dev/attestation/quote";
const ATTESTATION_USER_REPORT_DATA_DEVICE_FILE: &str = "/dev/attestation/user_report_data";
const NATIVE_PROOF_PRIVATE_KEY: &str =
    "92954368afd3caa1f3ce3ead0069c1af414054aefe1ef9aeacc1bf426222ce38";

pub trait TeeProvider {
    fn save_private_key(&self, key: &SecretKey) -> Result<()>;
    fn load_private_key(&self) -> Result<SecretKey>;
    fn load_quote(&self, instance_address: Address) -> Result<Vec<u8>>;
}

#[derive(Clone, Debug)]
pub(crate) struct GramineProvider {
    secret_dir: PathBuf,
    quote_device_path: PathBuf,
    user_report_data_device_path: PathBuf,
}

impl GramineProvider {
    #[must_use]
    pub(crate) fn new(secret_dir: PathBuf) -> Self {
        Self {
            secret_dir,
            quote_device_path: PathBuf::from(ATTESTATION_QUOTE_DEVICE_FILE),
            user_report_data_device_path: PathBuf::from(ATTESTATION_USER_REPORT_DATA_DEVICE_FILE),
        }
    }
    fn private_key_path(&self) -> PathBuf {
        self.secret_dir.join(PRIV_KEY_FILENAME)
    }

    fn write_user_report_data(&self, instance_address: Address) -> Result<()> {
        let mut payload = instance_address.to_vec();
        payload.resize(64, 0);
        let mut file = OpenOptions::new()
            .write(true)
            .open(&self.user_report_data_device_path)
            .with_context(|| {
                format!(
                    "open {} for writing user report data",
                    self.user_report_data_device_path.display()
                )
            })?;
        file.write_all(&payload).context("write user report data")?;
        Ok(())
    }
}

impl TeeProvider for GramineProvider {
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

    fn load_quote(&self, instance_address: Address) -> Result<Vec<u8>> {
        self.write_user_report_data(instance_address)?;
        let mut file = fs::File::open(&self.quote_device_path)
            .with_context(|| format!("open quote device {}", self.quote_device_path.display()))?;
        let mut quote = Vec::new();
        file.read_to_end(&mut quote).context("read quote")?;
        Ok(quote)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NativeProvider;

impl NativeProvider {
    fn private_key() -> Result<SecretKey> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(NATIVE_PROOF_PRIVATE_KEY, &mut bytes)
            .context("decode native proof private key")?;
        SecretKey::from_byte_array(&bytes).context("decode native proof private key")
    }
}

impl TeeProvider for NativeProvider {
    fn save_private_key(&self, _key: &SecretKey) -> Result<()> {
        Ok(())
    }

    fn load_private_key(&self) -> Result<SecretKey> {
        Self::private_key()
    }

    fn load_quote(&self, _instance_address: Address) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }
}
