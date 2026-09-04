use std::borrow::Cow;

use alloy_consensus::{BlockBody as AlloyBlockBody, Header};
use alloy_eips::eip4895::Withdrawals;
use alloy_primitives::map::AddressMap;
use reth_ethereum_primitives::{Block, TransactionSigned};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};
use serde_with::{DeserializeAs, SerializeAs};

/// Stable serde adapter for address-keyed maps.
///
/// `AddressMap` uses a randomized hasher, so its iteration order must not define the guest wire
/// encoding. The map representation itself remains unchanged; only entry order is canonicalized.
pub struct SortedAddressMap;

impl<V> SerializeAs<AddressMap<V>> for SortedAddressMap
where
    V: Serialize,
{
    fn serialize_as<S>(source: &AddressMap<V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries = source.iter().collect::<Vec<_>>();
        entries.sort_unstable_by_key(|(address, _)| *address);

        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (address, value) in entries {
            map.serialize_entry(address, value)?;
        }
        map.end()
    }
}

impl<'de, V> DeserializeAs<'de, AddressMap<V>> for SortedAddressMap
where
    V: Deserialize<'de>,
{
    fn deserialize_as<D>(deserializer: D) -> Result<AddressMap<V>, D::Error>
    where
        D: Deserializer<'de>,
    {
        AddressMap::deserialize(deserializer)
    }
}

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

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, map::AddressMap};
    use serde::{Deserialize, Serialize};
    use serde_with::serde_as;

    use super::SortedAddressMap;

    #[serde_as]
    #[derive(Debug, Deserialize, Serialize)]
    struct WrappedAddressMap {
        #[serde_as(as = "SortedAddressMap")]
        values: AddressMap<u64>,
    }

    #[test]
    fn sorted_address_map_encoding_ignores_insertion_order() {
        let low = Address::with_last_byte(1);
        let high = Address::with_last_byte(2);

        let mut forward = AddressMap::default();
        forward.insert(low, 10);
        forward.insert(high, 20);

        let mut reverse = AddressMap::default();
        reverse.insert(high, 20);
        reverse.insert(low, 10);

        let forward_bytes = bincode::serialize(&WrappedAddressMap { values: forward }).unwrap();
        let reverse_bytes = bincode::serialize(&WrappedAddressMap { values: reverse }).unwrap();
        assert_eq!(forward_bytes, reverse_bytes);

        let decoded: WrappedAddressMap = bincode::deserialize(&forward_bytes).unwrap();
        assert_eq!(decoded.values.get(&low), Some(&10));
        assert_eq!(decoded.values.get(&high), Some(&20));
    }
}
