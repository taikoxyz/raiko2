//! Public deterministic offline EIP-7702 identities and execution-result gates.
use alloy_consensus::TrieAccount;
use alloy_eips::eip7702::{Authorization, SignedAuthorization};
use alloy_primitives::{Address, B256, Signature, U256, keccak256};
use alloy_signer::SignerSync;
use alloy_signer_local_fixture::PrivateKeySigner;
use anyhow::{Result, ensure};
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const CHAIN_ID: u64 = 167_000;
const DELEGATE: Address = Address::repeat_byte(0x66);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthorizationPattern {
    Sequential,
    Repeated,
    Distinct,
    Compact,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramAuthorizations {
    pub pattern: AuthorizationPattern,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct AuthorizationReport {
    pub pattern: AuthorizationPattern,
    pub entries_per_block: usize,
    pub blocks: Vec<AuthorizationBlockReport>,
}

#[derive(Debug, Serialize)]
pub struct AuthorizationBlockReport {
    pub block_index: usize,
    pub signed_entries: usize,
    pub applied_delegations: u64,
    pub authorities: Vec<AuthorityPostState>,
}

#[derive(Debug, Serialize)]
pub struct AuthorityPostState {
    pub address: Address,
    pub previous_nonce: u64,
    pub nonce: u64,
    pub code_hash: B256,
}

fn compact_entry() -> SignedAuthorization {
    // Canonical compact signature. Its authority is determined by recovery for
    // this authorization hash; no private fixture key is used for this pattern.
    Authorization {
        chain_id: U256::from(CHAIN_ID),
        address: DELEGATE,
        nonce: 0,
    }
    .into_signed(Signature::new(U256::from(1), U256::from(1), false))
}

fn signer(index: usize) -> Result<PrivateKeySigner> {
    // Public fixture keys are the integers 1..=1024, distinct from the outer
    // sender's repeated-0x11 key. These are not credentials; never load a wallet.
    Ok(PrivateKeySigner::from_bytes(&B256::from(
        U256::from(index + 1).to_be_bytes::<32>(),
    ))?)
}

impl ProgramAuthorizations {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=1024).contains(&self.count),
            "authorization count must be in 1..=1024"
        );
        Ok(())
    }

    fn entry_nonce(&self, block_index: usize, index: usize) -> Result<u64> {
        Ok(u64::try_from(match self.pattern {
            AuthorizationPattern::Compact => 0,
            AuthorizationPattern::Sequential => block_index * self.count + index,
            _ => block_index,
        })?)
    }

    fn entry_authority(&self, index: usize) -> Result<PrivateKeySigner> {
        signer(if self.pattern == AuthorizationPattern::Distinct {
            index
        } else {
            0
        })
    }

    fn authority_address(&self, index: usize) -> Result<Address> {
        if self.pattern == AuthorizationPattern::Compact {
            return Ok(compact_entry().recover_authority()?);
        }
        Ok(self.entry_authority(index)?.address())
    }

    fn expected_before_nonce(&self, block_index: usize) -> Result<u64> {
        if self.pattern == AuthorizationPattern::Compact {
            return Ok(u64::from(block_index != 0));
        }
        self.entry_nonce(block_index, 0)
    }

    fn expected_after_nonce(&self, block_index: usize) -> Result<u64> {
        if self.pattern == AuthorizationPattern::Compact {
            return Ok(1);
        }
        self.entry_nonce(block_index + 1, 0)
    }

    fn expected_applied(&self, block_index: usize) -> Result<u64> {
        match self.pattern {
            AuthorizationPattern::Compact => Ok(u64::from(block_index == 0)),
            AuthorizationPattern::Repeated => Ok(1),
            _ => Ok(u64::try_from(self.count)?),
        }
    }

    pub(super) fn signed_list(&self, block_index: usize) -> Result<Vec<SignedAuthorization>> {
        self.validate()?;
        if self.pattern == AuthorizationPattern::Compact {
            let entry = compact_entry();
            entry.recover_authority()?;
            return Ok(vec![entry; self.count]);
        }
        let sign = |index| -> Result<SignedAuthorization> {
            let signer = self.entry_authority(index)?;
            let auth = Authorization {
                chain_id: U256::from(CHAIN_ID),
                address: DELEGATE,
                nonce: self.entry_nonce(block_index, index)?,
            };
            let signature = signer.sign_hash_sync(&auth.signature_hash())?;
            Ok(auth.into_signed(signature))
        };
        if self.pattern == AuthorizationPattern::Repeated {
            return Ok(vec![sign(0)?; self.count]);
        }
        (0..self.count).map(sign).collect()
    }

    pub(super) fn validate_transaction(
        &self,
        tx: &TransactionSigned,
        block_index: usize,
    ) -> Result<()> {
        self.validate()?;
        let transaction = tx
            .as_eip7702()
            .ok_or_else(|| {
                anyhow::anyhow!("authorization fixture requires canonical type-4 transaction")
            })?
            .tx();
        ensure!(
            transaction.authorization_list.len() == self.count,
            "canonical authorization list count differs"
        );
        for (index, auth) in transaction.authorization_list.iter().enumerate() {
            ensure!(
                auth.inner().chain_id == U256::from(CHAIN_ID)
                    && auth.inner().address == DELEGATE
                    && auth.inner().nonce == self.entry_nonce(block_index, index)?,
                "canonical authorization entry differs"
            );
            ensure!(
                auth.recover_authority()? == self.authority_address(index)?,
                "canonical authorization signature differs"
            );
        }
        if self.pattern == AuthorizationPattern::Compact {
            ensure!(
                transaction.authorization_list[0] == compact_entry(),
                "compact authorization must use canonical r=1,s=1,parity=0"
            );
        }
        if matches!(
            self.pattern,
            AuthorizationPattern::Repeated | AuthorizationPattern::Compact
        ) {
            ensure!(
                transaction
                    .authorization_list
                    .iter()
                    .all(|a| a == &transaction.authorization_list[0]),
                "repeated authorizations must be identical signed entries"
            );
        }
        Ok(())
    }

    pub(super) fn before_block(
        &self,
        block_index: usize,
        accounts: &BTreeMap<B256, TrieAccount>,
    ) -> Result<BTreeMap<Address, u64>> {
        self.validate()?;
        let count = if self.pattern == AuthorizationPattern::Distinct {
            self.count
        } else {
            1
        };
        (0..count)
            .map(|index| {
                let address = self.authority_address(index)?;
                let account = accounts.get(&keccak256(address));
                ensure!(
                    block_index != 0 || account.is_none(),
                    "authorization authority must be absent from the initial prestate"
                );
                let nonce = account.map_or(0, |a| a.nonce);
                let expected = self.expected_before_nonce(block_index)?;
                ensure!(
                    nonce == expected,
                    "authorization prestate nonce differs from the required progression"
                );
                ensure!(
                    account.is_none_or(|a| a.code_hash == delegation_code_hash()),
                    "authorization prestate has unexpected code"
                );
                Ok((address, nonce))
            })
            .collect()
    }

    pub(super) fn after_block(
        &self,
        block_index: usize,
        before: &BTreeMap<Address, u64>,
        accounts: &BTreeMap<B256, TrieAccount>,
    ) -> Result<AuthorizationBlockReport> {
        let mut applied_delegations = 0;
        let authorities = before
            .iter()
            .map(|(address, previous_nonce)| {
                let account = accounts.get(&keccak256(address)).ok_or_else(|| {
                    anyhow::anyhow!("authorization authority missing after execution")
                })?;
                let expected = self.expected_after_nonce(block_index)?;
                ensure!(
                    account.nonce == expected && account.code_hash == delegation_code_hash(),
                    "actual authority nonce/delegation differs from required post-state"
                );
                applied_delegations += account
                    .nonce
                    .checked_sub(*previous_nonce)
                    .ok_or_else(|| anyhow::anyhow!("authority nonce decreased"))?;
                Ok(AuthorityPostState {
                    address: *address,
                    previous_nonce: *previous_nonce,
                    nonce: account.nonce,
                    code_hash: account.code_hash,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let expected = self.expected_applied(block_index)?;
        ensure!(
            applied_delegations == expected,
            "actual applied delegation count differs"
        );
        Ok(AuthorizationBlockReport {
            block_index,
            signed_entries: self.count,
            applied_delegations,
            authorities,
        })
    }
}

fn delegation_code_hash() -> B256 {
    keccak256([&[0xef, 0x01, 0x00][..], DELEGATE.as_slice()].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_poststate_preserves_nonce_one_when_every_later_entry_is_stale() {
        let config = ProgramAuthorizations {
            pattern: AuthorizationPattern::Compact,
            count: 16,
        };
        let mut accounts = BTreeMap::new();
        let address = compact_entry().recover_authority().unwrap();
        for block in 0..3 {
            let before = config.before_block(block, &accounts).unwrap();
            if block == 0 {
                accounts.insert(
                    keccak256(address),
                    TrieAccount {
                        nonce: 1,
                        code_hash: delegation_code_hash(),
                        ..Default::default()
                    },
                );
            }
            let report = config.after_block(block, &before, &accounts).unwrap();
            assert_eq!(report.signed_entries, 16);
            assert_eq!(report.applied_delegations, u64::from(block == 0));
            assert_eq!(report.authorities[0].nonce, 1);
            assert!(
                config
                    .signed_list(block)
                    .unwrap()
                    .iter()
                    .all(|entry| entry == &compact_entry())
            );
        }
        let before = config.before_block(3, &accounts).unwrap();
        accounts.get_mut(&keccak256(address)).unwrap().nonce = 2;
        assert!(config.after_block(3, &before, &accounts).is_err());
        assert!(config.before_block(3, &accounts).is_err());
    }

    #[test]
    fn authority_poststate_gate_rejects_missing_nonce_and_code_mismatches() {
        let config = ProgramAuthorizations {
            pattern: AuthorizationPattern::Sequential,
            count: 4,
        };
        let mut accounts = BTreeMap::new();
        let before = config.before_block(0, &accounts).unwrap();
        assert!(config.after_block(0, &before, &accounts).is_err());
        let address = *before.keys().next().unwrap();
        accounts.insert(
            keccak256(address),
            TrieAccount {
                nonce: 4,
                code_hash: delegation_code_hash(),
                ..Default::default()
            },
        );
        let report = config.after_block(0, &before, &accounts).unwrap();
        assert_eq!(report.applied_delegations, 4);
        assert!(config.before_block(0, &accounts).is_err());
        assert!(config.before_block(1, &accounts).is_ok());
        accounts.get_mut(&keccak256(address)).unwrap().nonce = 3;
        assert!(config.after_block(0, &before, &accounts).is_err());
        assert!(config.before_block(1, &accounts).is_err());
        accounts.get_mut(&keccak256(address)).unwrap().nonce = 4;
        accounts.get_mut(&keccak256(address)).unwrap().code_hash = B256::ZERO;
        assert!(config.after_block(0, &before, &accounts).is_err());
        assert!(config.before_block(1, &accounts).is_err());
    }
}
