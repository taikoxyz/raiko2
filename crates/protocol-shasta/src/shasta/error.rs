//! Shasta protocol errors from taiko-client-rs.

pub use taiko_client_protocol::shasta::error::{
    ForkConfigError as ShastaForkConfigError, ForkConfigResult,
};
pub use taiko_client_protocol::shasta::{ProtocolError, Result};
