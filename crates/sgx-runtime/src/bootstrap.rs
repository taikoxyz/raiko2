//! Bootstrap artifact generation and persistence.

use std::{collections::BTreeMap, fs, path::Path};

use alloy_primitives::{Address, keccak256};
use anyhow::{Context, Result};
use getrandom::fill as fill_random;
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};

use crate::{
    config::{BOOTSTRAP_INFO_FILENAME, GlobalOpts, REGISTERED_INFO_FILENAME, RuntimeMode},
    tee::{GramineProvider, NativeProvider, TeeProvider},
};

/// Registered instance ids keyed by fork name.
pub type RegisteredInstanceIds = BTreeMap<String, u64>;

/// Persisted SGX bootstrap metadata used for registration and prove responses.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapData {
    /// Uncompressed secp256k1 public key.
    pub public_key: String,
    /// Derived SGX instance address.
    pub new_instance: Address,
    /// Hex-encoded attestation quote.
    pub quote: String,
}

/// Bootstrap the SGX runtime and return operator-facing bootstrap metadata.
///
/// # Errors
///
/// Returns an error when the selected runtime mode cannot produce its bootstrap metadata.
/// `tee` mode persists bootstrap artifacts to disk, while `native` mode is a no-op that only
/// returns the fixed signer identity.
pub fn bootstrap(opts: &GlobalOpts) -> Result<BootstrapData> {
    match opts.mode {
        RuntimeMode::Tee => {
            let provider = GramineProvider::new(opts.secret_dir.clone());
            bootstrap_with_provider(&provider, &opts.config_dir)
        }
        RuntimeMode::Native => bootstrap_native(),
    }
}

/// Bootstrap the SGX runtime using an injected TEE provider.
///
/// # Errors
///
/// Returns an error when the provider fails to persist the private key, load the quote,
/// or when the bootstrap metadata cannot be written.
pub fn bootstrap_with_provider<P: TeeProvider>(
    provider: &P,
    config_dir: &Path,
) -> Result<BootstrapData> {
    let secret_key = generate_secret_key();
    provider.save_private_key(&secret_key)?;

    let public_key = public_key(&secret_key);
    let instance_address = public_key_to_address(&public_key);
    let quote = provider.load_quote(instance_address)?;
    let data = bootstrap_data_from_parts(public_key, instance_address, quote);
    save_bootstrap_data(config_dir, &data)?;
    Ok(data)
}

fn bootstrap_native() -> Result<BootstrapData> {
    let provider = NativeProvider;
    let secret_key = provider.load_private_key()?;
    let public_key = public_key(&secret_key);
    let instance_address = public_key_to_address(&public_key);
    let quote = provider.load_quote(instance_address)?;
    Ok(bootstrap_data_from_parts(
        public_key,
        instance_address,
        quote,
    ))
}

fn bootstrap_data_from_parts(
    public_key: PublicKey,
    instance_address: Address,
    quote: Vec<u8>,
) -> BootstrapData {
    BootstrapData {
        public_key: format!("0x{}", hex::encode(public_key.serialize_uncompressed())),
        new_instance: instance_address,
        quote: hex::encode(quote),
    }
}

/// Persist bootstrap metadata into the configured operator directory.
///
/// # Errors
///
/// Returns an error when the bootstrap file cannot be serialized or written.
pub fn save_bootstrap_data(config_dir: &Path, data: &BootstrapData) -> Result<()> {
    write_json(&config_dir.join(BOOTSTRAP_INFO_FILENAME), data)
}

/// Load bootstrap metadata from disk.
///
/// # Errors
///
/// Returns an error when the bootstrap file cannot be read or decoded.
pub fn load_bootstrap_data(config_dir: &Path) -> Result<BootstrapData> {
    read_json(&config_dir.join(BOOTSTRAP_INFO_FILENAME))
}

/// Persist registered instance ids resolved after onchain registration.
///
/// # Errors
///
/// Returns an error when the registered instance id file cannot be serialized or written.
pub fn save_registered_instance_ids(
    config_dir: &Path,
    instance_ids: &RegisteredInstanceIds,
) -> Result<()> {
    write_json(&config_dir.join(REGISTERED_INFO_FILENAME), instance_ids)
}

/// Load registered instance ids from disk.
///
/// # Errors
///
/// Returns an error when the registered instance id file cannot be read or decoded.
pub fn load_registered_instance_ids(config_dir: &Path) -> Result<RegisteredInstanceIds> {
    read_json(&config_dir.join(REGISTERED_INFO_FILENAME))
}

fn generate_secret_key() -> SecretKey {
    loop {
        let mut bytes = [0u8; 32];
        fill_random(&mut bytes).expect("fill secret key entropy");
        if let Ok(secret_key) = SecretKey::from_byte_array(&bytes) {
            return secret_key;
        }
    }
}

/// Derive the public key for a previously bootstrapped secret key.
#[must_use]
pub fn public_key(secret_key: &SecretKey) -> PublicKey {
    PublicKey::from_secret_key(&Secp256k1::new(), secret_key)
}

/// Convert a secp256k1 public key into the onchain SGX instance address.
#[must_use]
pub fn public_key_to_address(public_key: &PublicKey) -> Address {
    let hash = keccak256(&public_key.serialize_uncompressed()[1..]);
    Address::from_slice(&hash[12..])
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {}", parent.display()))?;
    }
    let contents = serde_json::to_vec_pretty(value).context("serialize json")?;
    let mut contents = contents;
    contents.push(b'\n');
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let contents = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&contents).with_context(|| format!("decode {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

    use alloy_primitives::Address;
    use anyhow::Context;
    use secp256k1::SecretKey;

    use super::{
        BootstrapData, RegisteredInstanceIds, bootstrap, bootstrap_with_provider,
        load_bootstrap_data, load_registered_instance_ids, public_key, public_key_to_address,
        save_registered_instance_ids,
    };
    use crate::{
        config::{BOOTSTRAP_INFO_FILENAME, GlobalOpts, PRIV_KEY_FILENAME, RuntimeMode},
        tee::TeeProvider,
    };

    #[derive(Default)]
    struct FakeProvider {
        saved_key: Mutex<Option<SecretKey>>,
        quote: Vec<u8>,
    }

    impl TeeProvider for FakeProvider {
        fn save_private_key(&self, key: &SecretKey) -> anyhow::Result<()> {
            *self.saved_key.lock().unwrap() = Some(*key);
            Ok(())
        }

        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            self.saved_key
                .lock()
                .unwrap()
                .as_ref()
                .copied()
                .context("missing private key")
        }

        fn load_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            Ok(self.quote.clone())
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "raiko2-sgx-runtime-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn bootstrap_persists_bootstrap_metadata() {
        let provider = FakeProvider {
            quote: vec![0x12, 0x34, 0x56],
            ..FakeProvider::default()
        };
        let dir = temp_dir("bootstrap");

        let saved = bootstrap_with_provider(&provider, &dir).expect("bootstrap");
        let loaded = load_bootstrap_data(&dir).expect("load bootstrap");

        assert_eq!(loaded, saved);
        assert_eq!(loaded.quote, "123456");

        let secret = provider.load_private_key().expect("saved private key");
        let expected_addr = public_key_to_address(&public_key(&secret));
        assert_eq!(loaded.new_instance, expected_addr);
        assert!(loaded.public_key.starts_with("0x"));
    }

    #[test]
    fn registered_instance_ids_roundtrip() {
        let dir = temp_dir("registered");
        let want: RegisteredInstanceIds = BTreeMap::from([
            ("shasta".to_string(), 3131899904),
            ("pacaya".to_string(), 7),
        ]);

        save_registered_instance_ids(&dir, &want).expect("save registered ids");
        let got = load_registered_instance_ids(&dir).expect("load registered ids");

        assert_eq!(got, want);
    }

    #[test]
    fn bootstrap_data_json_shape_stays_stable() {
        let data = BootstrapData {
            public_key: "0x1234".to_string(),
            new_instance: Address::repeat_byte(0x11),
            quote: "beef".to_string(),
        };

        let value = serde_json::to_value(&data).expect("serialize");
        assert_eq!(value["public_key"], "0x1234");
        assert_eq!(
            value["new_instance"],
            format!("{:#x}", Address::repeat_byte(0x11))
        );
        assert_eq!(value["quote"], "beef");
    }

    #[test]
    fn native_bootstrap_is_noop_and_does_not_persist_files() {
        let config_dir = temp_dir("native-bootstrap-config");
        let secret_dir = temp_dir("native-bootstrap-secret");
        let opts = GlobalOpts {
            mode: RuntimeMode::Native,
            config_dir: config_dir.clone(),
            secret_dir: secret_dir.clone(),
        };

        let data = bootstrap(&opts).expect("native bootstrap");

        assert!(data.public_key.starts_with("0x04"));
        assert_eq!(data.quote, "");
        assert!(
            !config_dir.join(BOOTSTRAP_INFO_FILENAME).exists(),
            "native bootstrap should not persist bootstrap metadata"
        );
        assert!(
            !secret_dir.join(PRIV_KEY_FILENAME).exists(),
            "native bootstrap should not persist a private key"
        );
    }
}
