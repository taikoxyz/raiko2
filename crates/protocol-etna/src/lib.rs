//! Etna consensus input checks and landing statement encoding.
//!
//! The [`projection`] module follows the revised header/envelope/execution
//! binding. It checks equality against caller-authenticated context and returns
//! exact original RLP spans and transaction payloads. The composing verifier
//! must apply the existing strict EL verifier to the matched header and
//! transactions decoded from those payloads, authenticate the nested
//! witnesses, and check consensus and range continuity.
//!
//! The [`statement`] module follows the revised landing design. The full
//! proof must authenticate the deployment, origin, L1 state, committee
//! schedule and `CometBFT` header chain. It must check the current and next
//! validator hashes at every height and verify execution, forced inclusion,
//! data availability and complete range continuity. `ValState` belongs to node
//! validation.

#![no_std]

extern crate alloc;

pub mod consensus;
pub mod coverage;
pub mod input;
pub mod l1_history;
pub mod payload;
pub mod projection;
pub mod range;
pub mod statement;
