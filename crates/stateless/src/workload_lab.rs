//! Explicit execution profiles for offline workload experiments.
//!
//! This module is excluded from default builds. Its experimental profile changes only the
//! legacy budget used for transaction admission. EVM gas, nonce checks, execution accounting,
//! block assembly and stateless validation are still performed by their existing implementations.

use crate::DerivedBlockExecutor;
use alethia_reth_block::{
    config::{MissingBaseFee, TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::{DerivedBlockExecutionOutcome, execute_derived_block},
    executor::TaikoBlockExecutor,
};
use alethia_reth_evm::zk_gas::{
    meter::ZkGasMeter, schedule::ZkGasSchedule, unzen::UNZEN_ZK_GAS_SCHEDULE,
};
use reth_ethereum_primitives::Block;
use reth_evm::{ConfigureEvm, block::BlockExecutionError};
use reth_primitives_traits::{RecoveredBlock, SealedHeader};
use reth_revm::{Database, State, db::states::bundle_state::BundleRetention};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr, sync::atomic::Ordering};

static EVM_GAS_ONLY_SCHEDULE: ZkGasSchedule = ZkGasSchedule {
    block_limit: u64::MAX,
    ..UNZEN_ZK_GAS_SCHEDULE
};

/// Admission behavior selected by an offline workload experiment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionProfile {
    /// Preserve the consensus Unzen budget and all normal validation.
    #[default]
    LegacyUnzen,
    /// Retain EVM gas limits and legacy accounting without the legacy admission cap.
    EvmGasOnly,
}

impl ExecutionProfile {
    /// Stable profile identity stored beside experiment artifacts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyUnzen => "legacy-unzen",
            Self::EvmGasOnly => "evm-gas-only",
        }
    }

    fn schedule(self) -> &'static ZkGasSchedule {
        match self {
            Self::LegacyUnzen => &UNZEN_ZK_GAS_SCHEDULE,
            Self::EvmGasOnly => &EVM_GAS_ONLY_SCHEDULE,
        }
    }
}

impl fmt::Display for ExecutionProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExecutionProfile {
    type Err = ExecutionProfileError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "legacy-unzen" => Ok(Self::LegacyUnzen),
            "evm-gas-only" => Ok(Self::EvmGasOnly),
            _ => Err(ExecutionProfileError::UnknownProfile),
        }
    }
}

/// Invalid or unsafe placement of an experimental admission override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionProfileError {
    /// The profile label is not one of the two supported execution policies.
    UnknownProfile,
    /// The execution path did not install the expected Unzen meter.
    MissingMeter,
    /// The meter already contains charges, so replacing it would erase work.
    MeterAlreadyUsed,
}

impl fmt::Display for ExecutionProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownProfile => "execution profile must be legacy-unzen or evm-gas-only",
            Self::MissingMeter => "workload execution requires an Unzen meter",
            Self::MeterAlreadyUsed => {
                "execution profile must be installed before any block charges"
            }
        })
    }
}

impl std::error::Error for ExecutionProfileError {}

/// Installs the selected profile on a fresh block meter.
///
/// The `evm-gas-only` schedule retains every Unzen coefficient and checked counter; its only
/// changed field is the legacy block limit. Arithmetic overflow remains an execution error.
/// This must be called before execution so a profile change can never erase charged work.
///
/// # Errors
///
/// Returns an error if no legacy meter exists or the meter already contains block or transaction
/// charges. Existing accounting is never reset after work has begun.
pub fn apply_execution_profile(
    meter: Option<&mut ZkGasMeter<'_>>,
    profile: ExecutionProfile,
) -> Result<(), ExecutionProfileError> {
    let meter = meter.ok_or(ExecutionProfileError::MissingMeter)?;
    if meter.block_zk_gas_used() != 0 || meter.tx_zk_gas_used() != 0 {
        return Err(ExecutionProfileError::MeterAlreadyUsed);
    }
    *meter = ZkGasMeter::new(profile.schedule());
    Ok(())
}

/// Uninspected derived-block executor for explicitly labelled workload programs.
///
/// Default construction delegates to the upstream legacy executor. The experimental branch
/// follows that same execution sequence, replacing only its fresh legacy meter before any
/// transaction begins. Reconstructed state roots and proposal outputs are still validated by
/// the caller's ordinary stateless/proposal reconstruction path.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlainDerivedBlockExecutor {
    profile: ExecutionProfile,
}

impl PlainDerivedBlockExecutor {
    /// Selects a fixed profile for every block executed by this instance.
    #[must_use]
    pub const fn new(profile: ExecutionProfile) -> Self {
        Self { profile }
    }
}

impl DerivedBlockExecutor for PlainDerivedBlockExecutor {
    fn execute<DB: Database + fmt::Debug>(
        &mut self,
        evm_config: &TaikoEvmConfig,
        parent_header: &SealedHeader,
        derived_block: &RecoveredBlock<Block>,
        db: DB,
    ) -> Result<DerivedBlockExecutionOutcome, BlockExecutionError> {
        if self.profile == ExecutionProfile::LegacyUnzen {
            return execute_derived_block(evm_config, parent_header, derived_block, db);
        }

        let mut state = State::builder()
            .with_database(db)
            .with_bundle_update()
            .build();
        let header = derived_block.header();
        let attributes = TaikoNextBlockEnvAttributes {
            timestamp: header.timestamp,
            suggested_fee_recipient: header.beneficiary,
            prev_randao: header.mix_hash,
            gas_limit: header.gas_limit,
            extra_data: header.extra_data.clone(),
            base_fee_per_gas: header.base_fee_per_gas.ok_or_else(|| {
                BlockExecutionError::other(MissingBaseFee {
                    block_number: header.number,
                })
            })?,
            parent_beacon_block_root: header.parent_beacon_block_root,
        };
        let environment = evm_config
            .next_evm_env(parent_header, &attributes)
            .map_err(BlockExecutionError::other)?;
        let mut evm = evm_config.evm_with_env(&mut state, environment);
        apply_execution_profile(evm.meter_mut(), self.profile)
            .map_err(BlockExecutionError::other)?;
        let context = evm_config
            .context_for_next_block(parent_header, attributes)
            .map_err(BlockExecutionError::other)?;
        let finalized = context.finalized_block_zk_gas.clone();
        let executor = TaikoBlockExecutor::new_with_zk_gas_mode(
            evm,
            context,
            evm_config.executor_factory.spec().clone(),
            evm_config.executor_factory.receipt_builder(),
            evm_config.zk_gas_mode(),
        );
        let execution = executor
            .execute_block_with_committed_transactions(derived_block.transactions_recovered())?;
        state.merge_transitions(BundleRetention::Reverts);
        let bundle = state.take_bundle();

        // This outcome, like the upstream executor, is Unzen-only. EIP-6780 means that the
        // ordinary bundle state contains all required account/storage changes; no pre-Unzen
        // out-of-band storage wipe list is introduced by this experiment.
        Ok(DerivedBlockExecutionOutcome {
            committed_transactions: execution.committed_transactions,
            execution_result: execution.execution_result,
            hashed_state: HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle.state()),
            finalized_block_zk_gas: finalized.load(Ordering::Relaxed),
            zk_gas_shadow_report: execution.zk_gas_shadow_report,
            attempted_zk_gas_shadow_report: execution.attempted_zk_gas_shadow_report,
            attempted_transaction_count: execution.attempted_transaction_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alethia_reth_chainspec::TAIKO_DEVNET;
    use alloy_consensus::{Header, Signed, TxLegacy, TxReceipt, transaction::Recovered};
    use alloy_primitives::{Address, B256, Bytes, Signature, TxKind, U256};
    use reth_ethereum_primitives::{BlockBody, TransactionSigned};
    use reth_revm::{
        db::InMemoryDB,
        state::{AccountInfo, Bytecode},
    };

    const CALLER: Address = Address::with_last_byte(0x30);
    const STOP_TARGET: Address = Address::with_last_byte(0x31);
    const WORKLOAD_TARGET: Address = Address::with_last_byte(0x32);

    fn transaction(
        chain_id: u64,
        nonce: u64,
        target: Address,
        gas_limit: u64,
    ) -> Recovered<TransactionSigned> {
        Recovered::new_unchecked(
            Signed::new_unchecked(
                TxLegacy {
                    chain_id: Some(chain_id),
                    nonce,
                    gas_price: 1,
                    gas_limit,
                    to: TxKind::Call(target),
                    value: U256::ZERO,
                    input: Bytes::new(),
                },
                Signature::new(U256::from(1), U256::from(2), false),
                B256::with_last_byte(nonce as u8),
            )
            .into(),
            CALLER,
        )
    }

    fn execute_workload(
        executor: &mut PlainDerivedBlockExecutor,
        number: u64,
        code: &[u8],
        user_nonce: u64,
        user_gas_limit: u64,
        block_gas_limit: u64,
    ) -> DerivedBlockExecutionOutcome {
        let config = TaikoEvmConfig::new(TAIKO_DEVNET.clone());
        let chain_id = TAIKO_DEVNET.inner.chain().id();
        let parent = SealedHeader::seal_slow(Header {
            number: number - 1,
            timestamp: number - 1,
            ..Header::default()
        });
        let transactions = [
            transaction(chain_id, 0, STOP_TARGET, 100_000),
            transaction(chain_id, user_nonce, WORKLOAD_TARGET, user_gas_limit),
        ];
        let block = RecoveredBlock::new_unhashed(
            Block {
                header: Header {
                    number,
                    timestamp: number,
                    gas_limit: block_gas_limit,
                    base_fee_per_gas: Some(0),
                    parent_beacon_block_root: Some(B256::ZERO),
                    ..Header::default()
                },
                body: BlockBody {
                    transactions: transactions.iter().map(Recovered::clone_inner).collect(),
                    ommers: Vec::new(),
                    withdrawals: None,
                },
            },
            transactions.iter().map(Recovered::signer).collect(),
        );
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            CALLER,
            AccountInfo {
                balance: U256::from(100_000_000_000_u64),
                ..AccountInfo::default()
            },
        );
        for (address, bytes) in [(STOP_TARGET, &[0x00][..]), (WORKLOAD_TARGET, code)] {
            let bytecode = Bytecode::new_raw(Bytes::copy_from_slice(bytes));
            db.insert_account_info(
                address,
                AccountInfo {
                    nonce: 1,
                    code_hash: bytecode.hash_slow(),
                    code: Some(bytecode),
                    ..AccountInfo::default()
                },
            );
        }
        executor.execute(&config, &parent, &block, db).unwrap()
    }

    #[test]
    fn experimental_profile_charges_and_commits_beyond_the_legacy_cap() {
        let mut meter = ZkGasMeter::new(&UNZEN_ZK_GAS_SCHEDULE);
        apply_execution_profile(Some(&mut meter), ExecutionProfile::EvmGasOnly).unwrap();
        let amount = UNZEN_ZK_GAS_SCHEDULE.block_limit + 1;
        // SLOAD has a nonzero multiplier; choose a charge strictly above the old budget.
        assert!(meter.charge_opcode(0x54, amount).is_ok());
        assert!(meter.tx_zk_gas_used() > UNZEN_ZK_GAS_SCHEDULE.block_limit);
        assert!(meter.commit_transaction().is_ok());
        assert!(meter.block_zk_gas_used() > UNZEN_ZK_GAS_SCHEDULE.block_limit);
    }

    #[test]
    fn legacy_profile_preserves_the_consensus_budget_and_all_coefficients() {
        let mut meter = ZkGasMeter::new(&EVM_GAS_ONLY_SCHEDULE);
        apply_execution_profile(Some(&mut meter), ExecutionProfile::default()).unwrap();
        assert!(*meter.schedule() == UNZEN_ZK_GAS_SCHEDULE);
        assert!(
            meter
                .charge_opcode(0x54, UNZEN_ZK_GAS_SCHEDULE.block_limit + 1)
                .is_err()
        );
        let mut expected = UNZEN_ZK_GAS_SCHEDULE;
        expected.block_limit = u64::MAX;
        assert!(EVM_GAS_ONLY_SCHEDULE == expected);
    }

    #[test]
    fn replacing_a_used_or_missing_meter_cannot_erase_work() {
        assert_eq!(
            apply_execution_profile(None, ExecutionProfile::EvmGasOnly),
            Err(ExecutionProfileError::MissingMeter)
        );
        let mut meter = ZkGasMeter::new(&UNZEN_ZK_GAS_SCHEDULE);
        meter.charge_tx_intrinsic().unwrap();
        let charged = meter.tx_zk_gas_used();
        assert_eq!(
            apply_execution_profile(Some(&mut meter), ExecutionProfile::EvmGasOnly),
            Err(ExecutionProfileError::MeterAlreadyUsed)
        );
        assert_eq!(meter.tx_zk_gas_used(), charged);
        meter.commit_transaction().unwrap();
        assert_eq!(
            apply_execution_profile(Some(&mut meter), ExecutionProfile::LegacyUnzen),
            Err(ExecutionProfileError::MeterAlreadyUsed)
        );
        assert_eq!(meter.block_zk_gas_used(), charged);
    }

    #[test]
    fn profile_labels_are_strict_and_round_trip() {
        for profile in [ExecutionProfile::LegacyUnzen, ExecutionProfile::EvmGasOnly] {
            assert_eq!(
                profile.as_str().parse::<ExecutionProfile>().unwrap(),
                profile
            );
            assert_eq!(profile.to_string(), profile.as_str());
            let encoded = serde_json::to_string(&profile).unwrap();
            assert_eq!(
                serde_json::from_str::<ExecutionProfile>(&encoded).unwrap(),
                profile
            );
        }
        assert!("disabled".parse::<ExecutionProfile>().is_err());
        assert!(serde_json::from_str::<ExecutionProfile>("\"unbounded\"").is_err());
    }

    #[test]
    fn real_evm_work_above_the_legacy_cap_executes_only_with_explicit_profile() {
        // MSTORE expands to slightly over 2MiB: EVM gas remains below 15M, while the
        // unchanged Unzen MSTORE multiplier prices this above the old 100M cap.
        let code = [0x60, 0x01, 0x62, 0x20, 0x00, 0x00, 0x52, 0x00];
        let legacy = execute_workload(
            &mut PlainDerivedBlockExecutor::default(),
            1,
            &code,
            1,
            15_000_000,
            30_000_000,
        );
        let relaxed = execute_workload(
            &mut PlainDerivedBlockExecutor::new(ExecutionProfile::EvmGasOnly),
            1,
            &code,
            1,
            15_000_000,
            30_000_000,
        );
        assert_eq!(legacy.committed_transactions.len(), 1);
        assert_eq!(relaxed.committed_transactions.len(), 2);
        assert!(relaxed.execution_result.receipts[1].success);
        assert!(relaxed.finalized_block_zk_gas > UNZEN_ZK_GAS_SCHEDULE.block_limit);
        assert!(relaxed.execution_result.gas_used < 15_000_000);
    }

    #[test]
    fn experimental_profile_preserves_evm_out_of_gas_and_transaction_validation() {
        let code = [0x60, 0x01, 0x62, 0x20, 0x00, 0x00, 0x52, 0x00];
        let mut executor = PlainDerivedBlockExecutor::new(ExecutionProfile::EvmGasOnly);
        let out_of_gas = execute_workload(&mut executor, 1, &code, 1, 30_000, 30_000_000);
        assert_eq!(out_of_gas.committed_transactions.len(), 2);
        assert!(!out_of_gas.execution_result.receipts[1].success);
        assert_eq!(
            out_of_gas.execution_result.receipts[1].cumulative_gas_used(),
            51_000
        );

        let invalid_nonce = execute_workload(&mut executor, 1, &code, 99, 15_000_000, 30_000_000);
        assert_eq!(invalid_nonce.committed_transactions.len(), 1);
        let native_block_limit =
            execute_workload(&mut executor, 1, &code, 1, 15_000_000, 1_000_000);
        assert_eq!(native_block_limit.committed_transactions.len(), 1);
    }

    #[test]
    fn each_block_gets_a_fresh_meter_under_the_fixed_experimental_profile() {
        let code = [0x60, 0x01, 0x62, 0x20, 0x00, 0x00, 0x52, 0x00];
        let mut executor = PlainDerivedBlockExecutor::new(ExecutionProfile::EvmGasOnly);
        let first = execute_workload(&mut executor, 1, &code, 1, 15_000_000, 30_000_000);
        let second = execute_workload(&mut executor, 2, &code, 1, 15_000_000, 30_000_000);
        assert_eq!(first.committed_transactions.len(), 2);
        assert_eq!(second.committed_transactions.len(), 2);
        assert_eq!(first.finalized_block_zk_gas, second.finalized_block_zk_gas);
        assert_eq!(
            first.execution_result.gas_used,
            second.execution_result.gas_used
        );
    }
}
