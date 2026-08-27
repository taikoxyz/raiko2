use alloy::{consensus::Header, eips::BlockNumberOrTag, providers::Provider as AlloyProvider};
use alloy_primitives::{Address, Log as PrimitiveLog};
use alloy_rpc_types_eth::Filter;
use alloy_sol_types::SolEvent;
use futures::{StreamExt, TryStreamExt, stream};
use raiko2_primitives::{RaikoError, RaikoResult};
use raiko2_protocol_shasta::shasta::{Proposed, ShastaEventData};

use super::NetworkProvider;

const L1_HEADER_CONCURRENCY: usize = 16;

impl NetworkProvider {
    pub(crate) async fn fetch_l1_header(&self, block_number: u64) -> RaikoResult<Header> {
        let rpc_block = self
            .l1_provider
            .get_block(BlockNumberOrTag::from(block_number).into())
            .await
            .map_err(|e| {
                RaikoError::RPC(format!(
                    "eth_getBlockByNumber failed for L1 block {block_number}: {e}"
                ))
            })?
            .ok_or_else(|| RaikoError::RPC(format!("L1 block {block_number} not found")))?;
        Ok(rpc_block.header.clone().into())
    }

    pub(crate) async fn fetch_l1_headers(&self, block_numbers: &[u64]) -> RaikoResult<Vec<Header>> {
        stream::iter(block_numbers.iter().copied())
            .map(|block_number| async move { self.fetch_l1_header(block_number).await })
            .buffered(L1_HEADER_CONCURRENCY)
            .try_collect()
            .await
    }

    pub(crate) async fn fetch_proposal_event(
        &self,
        l1_contract: Address,
        l1_inclusion_block_number: u64,
        proposal_id: u64,
    ) -> RaikoResult<ShastaEventData> {
        if l1_inclusion_block_number == 0 {
            return Err(RaikoError::InvalidRequestConfig(
                "shasta_l1_inclusion_block_number must be greater than zero".to_string(),
            ));
        }

        let filter = Filter::new()
            .address(l1_contract)
            .from_block(l1_inclusion_block_number)
            .to_block(l1_inclusion_block_number)
            .event_signature(Proposed::SIGNATURE_HASH);
        let logs = self.l1_provider.get_logs(&filter).await.map_err(|e| {
            RaikoError::RPC(format!(
                "eth_getLogs failed for Shasta proposal event lookup at L1 block {l1_inclusion_block_number}: {e}"
            ))
        })?;

        for log in logs {
            let Some(log_struct) = PrimitiveLog::new(
                log.address(),
                log.topics().to_vec(),
                log.data().data.clone(),
            ) else {
                return Err(RaikoError::RPC(
                    "failed to decode Shasta proposal log envelope".to_string(),
                ));
            };
            let event = Proposed::decode_log(&log_struct).map_err(|e| {
                RaikoError::RPC(format!("failed to decode Shasta proposal log: {e}"))
            })?;
            if event.data.id.to::<u64>() != proposal_id {
                continue;
            }

            let mut event_data =
                ShastaEventData::from_proposal_event(&event.data).map_err(|e| {
                    RaikoError::RPC(format!("failed to convert Shasta proposal event: {e}"))
                })?;
            let origin_block_number = l1_inclusion_block_number - 1;
            let inclusion_header = self.fetch_l1_header(l1_inclusion_block_number).await?;
            let origin_header = self.fetch_l1_header(origin_block_number).await?;
            event_data.proposal.originBlockNumber =
                origin_block_number.try_into().map_err(|_| {
                    RaikoError::RPC(format!(
                        "Shasta origin block number {origin_block_number} does not fit in uint48"
                    ))
                })?;
            event_data.proposal.originBlockHash = origin_header.hash_slow();
            event_data.proposal.timestamp =
                inclusion_header.timestamp.try_into().map_err(|_| {
                    RaikoError::RPC(format!(
                        "Shasta inclusion timestamp {} does not fit in uint48",
                        inclusion_header.timestamp
                    ))
                })?;
            return Ok(event_data);
        }

        Err(RaikoError::RPC(format!(
            "Shasta proposal event for proposal_id {proposal_id} not found in L1 block {l1_inclusion_block_number}"
        )))
    }
}
