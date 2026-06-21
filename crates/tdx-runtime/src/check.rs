//! Validation helpers for TDX lifecycle state.

use anyhow::Result;

use crate::{
    bootstrap::load_bootstrap_data,
    config::{GlobalOpts, RuntimeMode},
    tee::{TdxProvider, TeeProvider},
};

/// Validate that lifecycle state is usable for the selected runtime mode.
///
/// # Errors
///
/// Returns an error when `tee` mode bootstrap metadata or the TDX private key cannot be loaded.
/// `native` mode succeeds without TDX lifecycle files.
pub fn check(opts: &GlobalOpts) -> Result<()> {
    match opts.mode {
        RuntimeMode::Tee => {
            let provider = TdxProvider::new(opts.secret_dir.clone(), opts.tdxs_socket.clone());
            let _ = load_bootstrap_data(&opts.config_dir)?;
            check_with_provider(&provider)
        }
        RuntimeMode::Native => Ok(()),
    }
}

/// Validate that the TDX private key has been bootstrapped and can be read.
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

    use alloy_primitives::{Address, B256};
    use secp256k1::SecretKey;

    use super::{check, check_with_provider};
    use crate::{
        config::{GlobalOpts, RuntimeMode},
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

        fn load_bootstrap_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            unreachable!("unused in test")
        }

        fn load_proof_quote(
            &self,
            _instance_address: Address,
            _input_hash: B256,
        ) -> anyhow::Result<Vec<u8>> {
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

        fn load_bootstrap_quote(&self, _instance_address: Address) -> anyhow::Result<Vec<u8>> {
            unreachable!("unused in test")
        }

        fn load_proof_quote(
            &self,
            _instance_address: Address,
            _input_hash: B256,
        ) -> anyhow::Result<Vec<u8>> {
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
            "raiko2-tdx-runtime-check-{name}-{}",
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
            mode: RuntimeMode::Native,
            config_dir: temp_dir("native-config"),
            secret_dir: temp_dir("native-secret"),
            tdxs_socket: crate::tee::DEFAULT_TDXS_SOCKET.into(),
        };

        check(&opts).expect("native check");
    }
}
