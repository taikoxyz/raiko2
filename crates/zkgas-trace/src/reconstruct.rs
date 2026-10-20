#![allow(clippy::redundant_pub_crate)]

use alethia_reth_block::{
    config::{MissingBaseFee, TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::DerivedBlockExecutionOutcome,
    executor::TaikoBlockExecutor,
};
use alethia_reth_evm::{
    alloy::TaikoEvmContext,
    zk_gas::pricing::{ResourceSelectors, ZkGasMode, ZkGasShadowReport},
};
use alloy_consensus::Transaction as _;
use alloy_evm::EvmFactory as _;
use alloy_primitives::{Address, B256, U256};
use reth_ethereum_primitives::{Block, TransactionSigned};
use reth_evm::{ConfigureEvm as _, block::BlockExecutionError};
use reth_primitives_traits::{RecoveredBlock, SealedHeader};
use reth_revm::{
    Database, Inspector, State,
    context::{ContextTr as _, JournalTr as _},
    db::states::{BundleState, bundle_state::BundleRetention},
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, InstructionResult, Interpreter,
        interpreter::EthInterpreter,
        interpreter_types::{InputsTr as _, Jumps as _, LegacyBytecode as _, LoopControl as _},
    },
};
use reth_trie_common::{HashedPostState, KeccakKeyHasher};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
enum ResourceError {
    #[error("resource selector {0} exceeds u64")]
    Count(&'static str),
    #[error("unique code byte total overflows")]
    CodeBytes,
    #[error("code hash {hash} has inconsistent lengths {first} and {second}")]
    CodeLength { hash: B256, first: u64, second: u64 },
}

#[derive(Debug, Default)]
struct ResourceLedger {
    accounts: BTreeSet<Address>,
    storage_accesses: BTreeSet<(Address, U256)>,
    storage_writes: BTreeSet<(Address, U256)>,
    code_lengths: BTreeMap<B256, u64>,
    error: Option<ResourceError>,
}

impl ResourceLedger {
    fn account(&mut self, address: Address) {
        self.accounts.insert(address);
    }

    fn storage_access(&mut self, address: Address, slot: U256) {
        self.account(address);
        self.storage_accesses.insert((address, slot));
    }

    fn storage_write(&mut self, address: Address, slot: U256) {
        self.storage_access(address, slot);
        self.storage_writes.insert((address, slot));
    }

    fn code(&mut self, hash: B256, bytes: &[u8]) {
        if bytes.is_empty() || self.error.is_some() {
            return;
        }
        let Ok(length) = u64::try_from(bytes.len()) else {
            self.error = Some(ResourceError::Count("code_bytes"));
            return;
        };
        if let Some(first) = self.code_lengths.get(&hash) {
            if *first != length {
                self.error = Some(ResourceError::CodeLength {
                    hash,
                    first: *first,
                    second: length,
                });
            }
        } else {
            self.code_lengths.insert(hash, length);
        }
    }

    fn selectors(&self) -> Result<ResourceSelectors, ResourceError> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let count =
            |label, value: usize| u64::try_from(value).map_err(|_| ResourceError::Count(label));
        let unique_code_bytes = self
            .code_lengths
            .values()
            .try_fold(0_u64, |total, value| total.checked_add(*value))
            .ok_or(ResourceError::CodeBytes)?;
        Ok(ResourceSelectors {
            unique_account_count: count("accounts", self.accounts.len())?,
            unique_storage_slot_access_count: count(
                "storage_accesses",
                self.storage_accesses.len(),
            )?,
            unique_storage_slot_write_count: count("storage_writes", self.storage_writes.len())?,
            unique_code_hash_count: count("code_hashes", self.code_lengths.len())?,
            unique_code_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct PendingStep {
    resource_account: Option<Address>,
    storage: Option<(Address, U256, bool)>,
}

fn record_pending_step(
    ledger: &mut ResourceLedger,
    step: PendingStep,
    instruction_succeeded: bool,
) {
    if instruction_succeeded && let Some(address) = step.resource_account {
        ledger.account(address);
    }
    if let Some((address, slot, write)) = step.storage {
        if write {
            ledger.storage_write(address, slot);
        } else {
            ledger.storage_access(address, slot);
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ResourceInspector {
    ledger: Arc<Mutex<ResourceLedger>>,
    pending: BTreeMap<usize, PendingStep>,
}

impl ResourceInspector {
    const fn with_ledger(ledger: Arc<Mutex<ResourceLedger>>) -> Self {
        Self {
            ledger,
            pending: BTreeMap::new(),
        }
    }
}

fn resource_account(interpreter: &Interpreter<EthInterpreter>, opcode: u8) -> Option<Address> {
    matches!(opcode, 0x31 | 0x3b | 0x3c | 0x3f | 0xff)
        .then(|| interpreter.stack.peek(0).ok())
        .flatten()
        .map(|value| Address::from_word(B256::from(value.to_be_bytes())))
}

impl<DB: Database> Inspector<TaikoEvmContext<DB>, EthInterpreter> for ResourceInspector {
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        _context: &mut TaikoEvmContext<DB>,
    ) {
        let mut ledger = self.ledger.lock().expect("resource ledger poisoned");
        ledger.account(interpreter.input.caller_address());
        ledger.account(interpreter.input.target_address());
        if let Some(address) = interpreter.input.bytecode_address() {
            ledger.account(*address);
        }
        let hash = interpreter.bytecode.get_or_calculate_hash();
        ledger.code(hash, interpreter.bytecode.bytecode_slice());
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let opcode = interpreter.bytecode.opcode();
        let storage = matches!(opcode, 0x54 | 0x55)
            .then(|| interpreter.stack.peek(0).ok())
            .flatten()
            .map(|slot| (interpreter.input.target_address(), slot, opcode == 0x55));
        self.pending.insert(
            context.journal().depth(),
            PendingStep {
                resource_account: resource_account(interpreter, opcode),
                storage,
            },
        );
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter<EthInterpreter>,
        context: &mut TaikoEvmContext<DB>,
    ) {
        let Some(step) = self.pending.remove(&context.journal().depth()) else {
            return;
        };
        let instruction_succeeded = interpreter
            .bytecode
            .instruction_result()
            .is_none_or(InstructionResult::is_ok);
        let mut ledger = self.ledger.lock().expect("resource ledger poisoned");
        record_pending_step(&mut ledger, step, instruction_succeeded);
    }

    fn call(
        &mut self,
        _context: &mut TaikoEvmContext<DB>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        let mut ledger = self.ledger.lock().expect("resource ledger poisoned");
        ledger.account(inputs.caller);
        ledger.account(inputs.target_address);
        ledger.account(inputs.bytecode_address);
        None
    }

    fn create(
        &mut self,
        _context: &mut TaikoEvmContext<DB>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        self.ledger
            .lock()
            .expect("resource ledger poisoned")
            .account(inputs.caller());
        None
    }
}

struct CountingTransactions<'a, I> {
    inner: I,
    started: &'a mut usize,
    ledger: Arc<Mutex<ResourceLedger>>,
}

impl<'tx, I> Iterator for CountingTransactions<'_, I>
where
    I: Iterator<Item = alloy_consensus::transaction::Recovered<&'tx TransactionSigned>>,
{
    type Item = alloy_consensus::transaction::Recovered<&'tx TransactionSigned>;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.inner.next()?;
        *self.started += 1;
        if let Some(access_list) = (*transaction.inner()).access_list() {
            let mut ledger = self.ledger.lock().expect("resource ledger poisoned");
            for item in &access_list.0 {
                ledger.account(item.address);
                for key in &item.storage_keys {
                    ledger.storage_access(item.address, U256::from_be_bytes(key.0));
                }
            }
        }
        Some(transaction)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockExecutionObservation {
    pub report: ZkGasShadowReport,
    pub exact_resources: ResourceSelectors,
    pub started_transaction_count: usize,
    pub finalized_block_zkgas: u64,
}

#[derive(Debug, Default)]
pub(crate) struct QuoteDerivedBlockExecutor {
    completed: Option<BlockExecutionObservation>,
}

impl QuoteDerivedBlockExecutor {
    pub(crate) const fn take(&mut self) -> Option<BlockExecutionObservation> {
        self.completed.take()
    }

    fn reconcile_loaded_state<DB>(state: &State<DB>, ledger: &mut ResourceLedger) {
        for (address, account) in &state.cache.accounts {
            ledger.account(*address);
            if let Some(account) = &account.account {
                for slot in account.storage.keys().copied() {
                    ledger.storage_access(*address, slot);
                }
                // EIP-7702 installs the delegation bytecode inline on the authority account and
                // does not necessarily add it to `state.cache.contracts`.
                if let Some(code) = &account.info.code {
                    ledger.code(account.info.code_hash, code.original_byte_slice());
                }
            }
        }
        for (hash, code) in &state.cache.contracts {
            ledger.code(*hash, code.original_byte_slice());
        }
    }
}

fn attributes_from_block(
    block: &RecoveredBlock<Block>,
) -> Result<TaikoNextBlockEnvAttributes, BlockExecutionError> {
    let header = block.header();
    Ok(TaikoNextBlockEnvAttributes {
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
    })
}

impl raiko2_stateless::DerivedBlockExecutor for QuoteDerivedBlockExecutor {
    fn execute<DB: Database + std::fmt::Debug>(
        &mut self,
        evm_config: &TaikoEvmConfig,
        parent_header: &SealedHeader,
        derived_block: &RecoveredBlock<Block>,
        db: DB,
    ) -> Result<DerivedBlockExecutionOutcome, BlockExecutionError> {
        let mut state = State::builder()
            .with_database(db)
            .with_bundle_update()
            .build();
        let attributes = attributes_from_block(derived_block)?;
        let environment = evm_config
            .next_evm_env(parent_header, &attributes)
            .map_err(BlockExecutionError::other)?;
        let ledger = Arc::new(Mutex::new(ResourceLedger::default()));
        let evm = evm_config.evm_factory().create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        let execution_context = evm_config
            .context_for_next_block(parent_header, attributes)
            .map_err(BlockExecutionError::other)?;
        let finalized_zk_gas = execution_context.finalized_block_zk_gas.clone();
        let executor = TaikoBlockExecutor::new_with_zk_gas_mode(
            evm,
            execution_context,
            evm_config.executor_factory.spec().clone(),
            evm_config.executor_factory.receipt_builder(),
            ZkGasMode::Shadow,
        );
        let mut started = 0usize;
        let transactions = CountingTransactions {
            inner: derived_block.transactions_recovered(),
            started: &mut started,
            ledger: ledger.clone(),
        };
        let execution = executor.execute_block_with_committed_transactions(transactions)?;
        state.merge_transitions(BundleRetention::Reverts);
        {
            let mut locked = ledger.lock().expect("resource ledger poisoned");
            Self::reconcile_loaded_state(&state, &mut locked);
        }
        let exact_resources = ledger
            .lock()
            .expect("resource ledger poisoned")
            .selectors()
            .map_err(BlockExecutionError::other)?;
        let report = execution
            .zk_gas_shadow_report
            .ok_or_else(|| BlockExecutionError::msg("missing zkGas shadow report"))?;
        let finalized_block_zkgas = finalized_zk_gas.load(std::sync::atomic::Ordering::Relaxed);
        self.completed = Some(BlockExecutionObservation {
            report,
            exact_resources,
            started_transaction_count: started,
            finalized_block_zkgas,
        });

        let bundle_state: BundleState = state.take_bundle();
        Ok(DerivedBlockExecutionOutcome {
            committed_transactions: execution.committed_transactions,
            execution_result: execution.execution_result,
            hashed_state: HashedPostState::from_bundle_state::<KeccakKeyHasher>(
                bundle_state.state(),
            ),
            finalized_block_zk_gas: finalized_block_zkgas,
            zk_gas_shadow_report: Some(report),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alethia_reth_evm::{
        alloy::TaikoZkGasEvm as _, factory::TaikoEvmFactory, spec::TaikoSpecId,
    };
    use alloy_evm::{Evm as _, EvmEnv};
    use alloy_primitives::{Bytes, TxKind, keccak256};
    use reth_revm::{
        context::{
            TxEnv,
            transaction::{Authorization, RecoveredAuthority, RecoveredAuthorization},
        },
        db::InMemoryDB,
        state::{AccountInfo, Bytecode},
    };

    #[test]
    fn ledger_deduplicates_resources_and_code_bytes() {
        let mut ledger = ResourceLedger::default();
        let address = Address::with_last_byte(1);
        let slot = U256::from(7);
        let hash = B256::with_last_byte(9);
        ledger.storage_write(address, slot);
        ledger.storage_write(address, slot);
        ledger.code(hash, &[1, 2, 3]);
        ledger.code(hash, &[1, 2, 3]);

        assert_eq!(
            ledger.selectors().unwrap(),
            ResourceSelectors {
                unique_account_count: 1,
                unique_storage_slot_access_count: 1,
                unique_storage_slot_write_count: 1,
                unique_code_hash_count: 1,
                unique_code_bytes: 3,
            }
        );
    }

    #[test]
    fn failed_step_records_storage_attempt_but_not_resource_account() {
        let storage_target = Address::with_last_byte(0xa1);
        let resource_account = Address::with_last_byte(0xa2);
        let slot = U256::from(7);
        let mut ledger = ResourceLedger::default();

        record_pending_step(
            &mut ledger,
            PendingStep {
                resource_account: Some(resource_account),
                storage: Some((storage_target, slot, true)),
            },
            false,
        );

        assert!(ledger.accounts.contains(&storage_target));
        assert!(!ledger.accounts.contains(&resource_account));
        assert!(ledger.storage_accesses.contains(&(storage_target, slot)));
        assert!(ledger.storage_writes.contains(&(storage_target, slot)));
    }

    #[test]
    fn reconciliation_includes_inline_eip7702_delegation_code() {
        let caller = Address::with_last_byte(0xc1);
        let target = Address::with_last_byte(0xc2);
        let authority = Address::with_last_byte(0xc3);
        let delegate = Address::with_last_byte(0xc4);
        let mut database = InMemoryDB::default();
        database.insert_account_info(
            caller,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        database.insert_account_info(target, AccountInfo::default());
        database.insert_account_info(authority, AccountInfo::default());
        let mut state = State::builder().with_database(database).build();
        let ledger = Arc::new(Mutex::new(ResourceLedger::default()));
        let mut environment: EvmEnv<TaikoSpecId> = EvmEnv::default();
        environment.cfg_env.spec = TaikoSpecId::UNZEN;
        environment.cfg_env.chain_id = 167;
        environment.block_env.gas_limit = 30_000_000;
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        evm.set_zk_gas_pricing_enabled(true);
        let result = evm
            .transact_commit(
                TxEnv::builder()
                    .caller(caller)
                    .kind(TxKind::Call(target))
                    .chain_id(Some(167))
                    .gas_limit(1_000_000)
                    .authorization_list_recovered(vec![RecoveredAuthorization::new_unchecked(
                        Authorization {
                            chain_id: U256::from(167),
                            address: delegate,
                            nonce: 0,
                        },
                        RecoveredAuthority::Valid(authority),
                    )])
                    .build()
                    .expect("valid EIP-7702 transaction"),
            )
            .expect("EIP-7702 execution");
        assert!(result.is_success());
        evm.commit_transaction_zk_gas_pricing()
            .expect("pricing commit");
        let local = evm
            .zk_gas_pricing_snapshot()
            .expect("pricing snapshot")
            .shadow_report();
        drop(evm);

        let authority_account = state
            .cache
            .accounts
            .get(&authority)
            .and_then(|account| account.account.as_ref())
            .expect("authorization authority cached");
        let delegation = authority_account
            .info
            .code
            .as_ref()
            .expect("authorization installs delegation code inline");
        assert_eq!(delegation.original_byte_slice().len(), 23);
        assert!(
            !state
                .cache
                .contracts
                .contains_key(&authority_account.info.code_hash),
            "delegation must exercise the account-inline reconciliation path"
        );

        QuoteDerivedBlockExecutor::reconcile_loaded_state(
            &state,
            &mut ledger.lock().expect("resource ledger poisoned"),
        );
        let exact = ledger
            .lock()
            .expect("resource ledger poisoned")
            .selectors()
            .expect("exact resources");
        let alethia_reth_evm::zk_gas::pricing::ZkGasShadowStatus::Incomplete { known, .. } =
            local.status()
        else {
            panic!("expected incomplete local report")
        };
        assert!(exact.unique_code_hash_count >= known.evm_local_resources().unique_code_hash_count);
        assert!(exact.unique_code_bytes >= known.evm_local_resources().unique_code_bytes);
        assert!(exact.unique_code_bytes >= 23);
    }

    #[test]
    fn failed_child_sstore_is_still_an_exact_storage_write() {
        let caller = Address::with_last_byte(0xd1);
        let target = Address::with_last_byte(0xd2);
        let child = Address::with_last_byte(0xd3);
        let child_code = Bytes::from_static(&[
            0x60, 0x01, // PUSH1 value
            0x60, 0x00, // PUSH1 slot
            0x55, // SSTORE (halts this static child frame)
            0x00, // STOP
        ]);
        let mut target_code = vec![
            0x60, 0x00, // return size
            0x60, 0x00, // return offset
            0x60, 0x00, // calldata size
            0x60, 0x00, // calldata offset
            0x73, // PUSH20 child
        ];
        target_code.extend_from_slice(child.as_slice());
        target_code.extend_from_slice(&[
            0x61, 0xff, 0xff, // PUSH2 gas
            0xfa, // STATICCALL; returns false after child SSTORE halt
            0x50, // POP success flag
            0x00, // STOP, so the outer transaction commits
        ]);

        let account_with_code = |raw: Bytes| AccountInfo {
            code_hash: keccak256(&raw),
            code: Some(Bytecode::new_raw(raw)),
            ..Default::default()
        };
        let mut database = InMemoryDB::default();
        database.insert_account_info(
            caller,
            AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
        );
        database.insert_account_info(target, account_with_code(target_code.into()));
        database.insert_account_info(child, account_with_code(child_code));
        let mut state = State::builder().with_database(database).build();
        let ledger = Arc::new(Mutex::new(ResourceLedger::default()));
        let mut environment: EvmEnv<TaikoSpecId> = EvmEnv::default();
        environment.cfg_env.spec = TaikoSpecId::UNZEN;
        environment.cfg_env.chain_id = 167;
        environment.block_env.gas_limit = 30_000_000;
        let mut evm = TaikoEvmFactory.create_evm_with_inspector(
            &mut state,
            environment,
            ResourceInspector::with_ledger(ledger.clone()),
        );
        evm.set_zk_gas_pricing_enabled(true);
        let result = evm
            .transact_commit(
                TxEnv::builder()
                    .caller(caller)
                    .kind(TxKind::Call(target))
                    .chain_id(Some(167))
                    .gas_limit(1_000_000)
                    .build()
                    .expect("valid call transaction"),
            )
            .expect("outer execution");
        assert!(result.is_success(), "outer transaction must commit");
        evm.commit_transaction_zk_gas_pricing()
            .expect("pricing commit");
        let local = evm
            .zk_gas_pricing_snapshot()
            .expect("pricing snapshot")
            .shadow_report();
        drop(evm);

        QuoteDerivedBlockExecutor::reconcile_loaded_state(
            &state,
            &mut ledger.lock().expect("resource ledger poisoned"),
        );
        let exact = ledger
            .lock()
            .expect("resource ledger poisoned")
            .selectors()
            .expect("exact resources");
        let alethia_reth_evm::zk_gas::pricing::ZkGasShadowStatus::Incomplete { known, .. } =
            local.status()
        else {
            panic!("expected incomplete local report")
        };
        let local = known.evm_local_resources();
        assert_eq!(local.unique_storage_slot_write_count, 1);
        assert!(
            exact.unique_storage_slot_write_count >= local.unique_storage_slot_write_count,
            "failed child storage attempts must remain eligible for exact completion"
        );
    }
}
