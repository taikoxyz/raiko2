//! Anchor transaction construction for Taiko Shasta.

use std::borrow::Cow;

use super::signer::{FixedKSigner, FixedKSignerError};
use alethia_reth_consensus::validation::ANCHOR_V3_V4_GAS_LIMIT;
use alethia_reth_primitives::addresses::TAIKO_GOLDEN_TOUCH_ADDRESS;
use alloy_consensus::{
    EthereumTypedTransaction, TxEip1559, TxEnvelope,
    transaction::{SignableTransaction, TxHashable},
};
use alloy_eips::{BlockId, eip1898::RpcBlockHash, eip2930::AccessList};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256};
use alloy_provider::Provider;
use alloy_sol_types::{SolCall, sol};
use thiserror::Error;
use tracing::info;

sol! {
    #[derive(Debug)]
    struct AnchorV4Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    function anchorV4(AnchorV4Checkpoint _checkpoint) external;
}

/// Errors emitted by the anchor transaction constructor.
#[derive(Debug, Error)]
pub enum AnchorTxConstructorError {
    /// Invalid signer or fixed-k signing failure.
    #[error(transparent)]
    Signer(#[from] FixedKSignerError),
    /// Provider call failure while assembling anchor data.
    #[error("provider error: {0}")]
    Provider(String),
    /// Nonce did not fit into the transaction nonce type.
    #[error("nonce exceeds u64 range")]
    NonceOverflow,
    /// Base fee did not fit into EIP-1559 fee-cap type.
    #[error("fee cap exceeds u128 range")]
    FeeOverflow,
}

/// Parameters required to assemble an `anchorV4` transaction.
#[derive(Debug)]
pub struct AnchorV4Input {
    /// L1 anchor block number included in the checkpoint.
    pub anchor_block_number: u64,
    /// L1 anchor block hash included in the checkpoint.
    pub anchor_block_hash: B256,
    /// L1 anchor block state root included in the checkpoint.
    pub anchor_state_root: B256,
    /// Target L2 height used for logging and call context.
    pub l2_height: u64,
    /// Base fee used to derive EIP-1559 fee cap.
    pub base_fee: U256,
}

/// Builds Shasta anchor transactions for the golden touch account.
pub struct AnchorTxConstructor<L2Provider>
where
    L2Provider: Provider + Clone,
{
    l2_provider: L2Provider,
    anchor_address: Address,
    chain_id: u64,
    signer: FixedKSigner,
    golden_touch_address: Address,
}

impl<L2Provider> AnchorTxConstructor<L2Provider>
where
    L2Provider: Provider + Clone + Send + Sync + 'static,
{
    /// Create a new constructor using the shared golden touch key.
    pub async fn new(
        l2_provider: L2Provider,
        anchor_address: Address,
    ) -> Result<Self, AnchorTxConstructorError> {
        let signer = FixedKSigner::golden_touch()?;
        let golden_touch_address = Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS);
        let chain_id = l2_provider
            .get_chain_id()
            .await
            .map_err(|err| AnchorTxConstructorError::Provider(err.to_string()))?;

        Ok(Self {
            l2_provider,
            anchor_address,
            chain_id,
            signer,
            golden_touch_address,
        })
    }

    /// Assemble an `anchorV4` transaction for the given parent header and parameters.
    pub async fn assemble_anchor_v4_tx(
        &self,
        parent_hash: B256,
        params: AnchorV4Input,
    ) -> Result<TxEnvelope, AnchorTxConstructorError> {
        let AnchorV4Input {
            anchor_block_number,
            anchor_block_hash,
            anchor_state_root,
            l2_height,
            base_fee,
        } = params;

        let nonce: U256 = self
            .l2_provider
            .raw_request(
                Cow::Borrowed("eth_getTransactionCount"),
                (
                    self.golden_touch_address,
                    BlockId::Hash(RpcBlockHash {
                        block_hash: parent_hash,
                        require_canonical: Some(true),
                    }),
                ),
            )
            .await
            .or_else(|err| {
                if err.to_string().contains("not found") {
                    Ok(U256::ZERO)
                } else {
                    Err(AnchorTxConstructorError::Provider(err.to_string()))
                }
            })?;

        let nonce = u64::try_from(&nonce).map_err(|_| AnchorTxConstructorError::NonceOverflow)?;
        let gas_fee_cap =
            u128::try_from(&base_fee).map_err(|_| AnchorTxConstructorError::FeeOverflow)?;

        info!(
            l2_height,
            anchor_block_number,
            ?anchor_block_hash,
            ?anchor_state_root,
            nonce,
            ?base_fee,
            gas_fee_cap,
            "assembling shasta anchorV4 transaction",
        );

        let checkpoint = AnchorV4Checkpoint {
            blockNumber: anchor_block_number,
            blockHash: anchor_block_hash,
            stateRoot: anchor_state_root,
        };

        let calldata = Bytes::from(
            anchorV4Call {
                _checkpoint: checkpoint,
            }
            .abi_encode(),
        );
        let tx = TxEip1559 {
            chain_id: self.chain_id,
            nonce,
            gas_limit: ANCHOR_V3_V4_GAS_LIMIT,
            max_fee_per_gas: gas_fee_cap,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(self.anchor_address),
            value: U256::ZERO,
            access_list: AccessList::default(),
            input: calldata,
        };

        let sig_hash = tx.signature_hash();
        let mut hash_bytes = [0_u8; 32];
        hash_bytes.copy_from_slice(sig_hash.as_slice());
        let signature = self.signer.sign_with_predefined_k(&hash_bytes)?;
        let tx_hash = tx.tx_hash(&signature);

        Ok(TxEnvelope::new_unchecked(
            EthereumTypedTransaction::Eip1559(tx),
            signature,
            tx_hash,
        ))
    }
}
