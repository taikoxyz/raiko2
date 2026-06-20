//! Validation helpers for SGX lifecycle state.

use anyhow::Result;

use crate::{
    bootstrap::load_bootstrap_data,
    config::{GlobalOpts, RuntimeFlavor, RuntimeMode},
    tee::{GramineProvider, TeeProvider},
};

/// Validate that lifecycle state is usable for the selected runtime mode.
///
/// # Errors
///
/// Returns an error when `tee` mode bootstrap metadata or the SGX private key cannot be loaded.
/// `native` mode succeeds without SGX lifecycle files.
pub fn check(opts: &GlobalOpts) -> Result<()> {
    match (opts.flavor, opts.mode) {
        (RuntimeFlavor::Sgx, RuntimeMode::Tee) => {
            let provider = GramineProvider::new(opts.secret_dir.clone());
            let _ = load_bootstrap_data(&opts.config_dir)?;
            check_with_provider(&provider)
        }
        (RuntimeFlavor::Tdx, RuntimeMode::Tee) => {
            anyhow::bail!("TDX tee mode is not implemented")
        }
        (_, RuntimeMode::Native) => Ok(()),
    }
}

/// Validate that the SGX private key has been bootstrapped and can be read.
///
/// # Errors
///
/// Returns an error when the private key has not been bootstrapped or cannot be decoded.
pub fn check_with_provider<P: TeeProvider>(provider: &P) -> Result<()> {
    let _ = provider.load_private_key()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use alloy_primitives::Address;
    use secp256k1::SecretKey;

    use super::{check, check_with_provider};
    use crate::{
        config::{GlobalOpts, RuntimeFlavor, RuntimeMode},
        tee::TeeProvider,
    };

    struct FailingProvider;

    impl TeeProvider for FailingProvider {
        fn save_private_key(&self, _key: &SecretKey) -> anyhow::Result<()> {
            unreachable!("unused in test")
        }

        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            anyhow::bail!("missing bootstrap key")
        }

        fn load_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            unreachable!("unused in test")
        }
    }

    struct OkProvider;

    impl TeeProvider for OkProvider {
        fn save_private_key(&self, _key: &SecretKey) -> anyhow::Result<()> {
            unreachable!("unused in test")
        }

        fn load_private_key(&self) -> anyhow::Result<SecretKey> {
            SecretKey::from_slice(&[7u8; 32]).map_err(Into::into)
        }

        fn load_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            unreachable!("unused in test")
        }
    }

    #[test]
    fn check_fails_when_private_key_is_missing() {
        let err = check_with_provider(&FailingProvider).expect_err("missing bootstrap");
        assert!(err.to_string().contains("missing bootstrap key"));
    }

    #[test]
    fn check_passes_when_private_key_is_readable() {
        check_with_provider(&OkProvider).expect("check ok");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "raiko2-sgx-runtime-check-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn native_mode_check_is_noop_without_bootstrap_files() {
        let opts = GlobalOpts {
            flavor: RuntimeFlavor::Sgx,
            mode: RuntimeMode::Native,
            config_dir: temp_dir("native-config"),
            secret_dir: temp_dir("native-secret"),
        };

        check(&opts).expect("native check");
    }

    #[test]
    fn tdx_tee_check_fails_until_tdx_quote_provider_is_available() {
        let opts = GlobalOpts {
            flavor: RuntimeFlavor::Tdx,
            mode: RuntimeMode::Tee,
            config_dir: temp_dir("tdx-tee-config"),
            secret_dir: temp_dir("tdx-tee-secret"),
        };

        let err = check(&opts).expect_err("tdx tee unsupported");

        assert!(err.to_string().contains("TDX tee mode is not implemented"));
    }
}
