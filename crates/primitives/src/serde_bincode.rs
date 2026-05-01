use std::borrow::Cow;

use alloy_consensus::{BlockBody as AlloyBlockBody, Header};
use alloy_eips::eip4895::Withdrawals;
use reth_ethereum_primitives::{Block, TransactionSigned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::{DeserializeAs, SerializeAs};

/// Bincode/JSON-compatible wrapper for [`reth_ethereum_primitives::Block`].
#[derive(Debug, Serialize, Deserialize)]
pub struct EthereumBlock<'a> {
    header: alloy_consensus::serde_bincode_compat::Header<'a>,
    body: EthereumBlockBody<'a>,
}

impl<'a> From<&'a Block> for EthereumBlock<'a> {
    fn from(value: &'a Block) -> Self {
        Self {
            header: (&value.header).into(),
            body: (&value.body).into(),
        }
    }
}

impl From<EthereumBlock<'_>> for Block {
    fn from(value: EthereumBlock<'_>) -> Self {
        Self {
            header: value.header.into(),
            body: value.body.into(),
        }
    }
}

impl SerializeAs<Block> for EthereumBlock<'_> {
    fn serialize_as<S>(source: &Block, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        EthereumBlock::from(source).serialize(serializer)
    }
}

impl<'de> DeserializeAs<'de, Block> for EthereumBlock<'de> {
    fn deserialize_as<D>(deserializer: D) -> Result<Block, D::Error>
    where
        D: Deserializer<'de>,
    {
        EthereumBlock::deserialize(deserializer).map(Into::into)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EthereumBlockBody<'a> {
    transactions: Vec<alloy_consensus::serde_bincode_compat::transaction::EthereumTxEnvelope<'a>>,
    ommers: Vec<alloy_consensus::serde_bincode_compat::Header<'a>>,
    withdrawals: Cow<'a, Option<Withdrawals>>,
}

impl<'a> From<&'a AlloyBlockBody<TransactionSigned, Header>> for EthereumBlockBody<'a> {
    fn from(value: &'a AlloyBlockBody<TransactionSigned, Header>) -> Self {
        Self {
            transactions: value.transactions.iter().map(Into::into).collect(),
            ommers: value.ommers.iter().map(Into::into).collect(),
            withdrawals: Cow::Borrowed(&value.withdrawals),
        }
    }
}

impl From<EthereumBlockBody<'_>> for AlloyBlockBody<TransactionSigned, Header> {
    fn from(value: EthereumBlockBody<'_>) -> Self {
        Self {
            transactions: value.transactions.into_iter().map(Into::into).collect(),
            ommers: value.ommers.into_iter().map(Into::into).collect(),
            withdrawals: value.withdrawals.into_owned(),
        }
    }
}

impl SerializeAs<AlloyBlockBody<TransactionSigned, Header>> for EthereumBlockBody<'_> {
    fn serialize_as<S>(
        source: &AlloyBlockBody<TransactionSigned, Header>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        EthereumBlockBody::from(source).serialize(serializer)
    }
}

impl<'de> DeserializeAs<'de, AlloyBlockBody<TransactionSigned, Header>> for EthereumBlockBody<'de> {
    fn deserialize_as<D>(
        deserializer: D,
    ) -> Result<AlloyBlockBody<TransactionSigned, Header>, D::Error>
    where
        D: Deserializer<'de>,
    {
        EthereumBlockBody::deserialize(deserializer).map(Into::into)
    }
}
