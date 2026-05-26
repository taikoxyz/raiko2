//! TDX attestation service client.
//!
//! Communicates with an external TDX attestation service via a Unix domain socket.
//! Supports issuing attestation quotes, retrieving metadata, and validating quotes.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// Generic JSON-RPC-like request envelope.
#[derive(Debug, Serialize)]
pub struct Request<T> {
    pub method: String,
    pub data: T,
}

/// Request data for issuing a TDX attestation quote.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRequestData {
    pub user_data: String,
    pub nonce: String,
}

/// Request data for retrieving attestation metadata.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataRequestData {}

/// Generic response envelope.
#[derive(Debug, Deserialize)]
pub struct Response<T> {
    pub data: Option<T>,
    pub error: Option<String>,
}

/// Response from the issue attestation endpoint.
pub type IssueResponse = Response<IssueResponseData>;
/// Response from the metadata endpoint.
pub type MetadataResponse = Response<MetadataResponseData>;

/// Attestation issuance result.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueResponseData {
    pub document: String,
}

/// Attestation metadata (issuer type, previous user data, etc.).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct MetadataResponseData {
    pub issuer_type: String,
    pub user_data: String,
    pub nonce: String,
    pub metadata: serde_json::Value,
}

impl<T> Response<T> {
    fn into_result(self) -> Result<T> {
        match (self.data, self.error) {
            (Some(data), None) => Ok(data),
            (None, Some(error)) => Err(anyhow!("Attestation service error: {error}")),
            (None, None) => Err(anyhow!("Invalid response: neither data nor error")),
            (Some(_), Some(error)) => Err(anyhow!(
                "Invalid response: both data and error present. Error: {error}"
            )),
        }
    }
}

/// Issue a TDX attestation quote for the given user data and nonce.
///
/// Returns the raw attestation document bytes.
///
/// # Errors
///
/// Returns an error if the attestation service is unreachable or returns an error.
pub async fn issue_attestation(
    socket_path: &str,
    user_data: &[u8],
    nonce: &[u8],
) -> Result<Vec<u8>> {
    let issue_data = IssueRequestData {
        user_data: hex::encode(user_data),
        nonce: hex::encode(nonce),
    };
    let request = Request {
        method: "issue".to_string(),
        data: serde_json::to_value(issue_data)?,
    };
    let request_json = serde_json::to_string(&request).context("Failed to serialize request")?;
    let socket_path = socket_path.to_owned();

    let response_buf =
        tokio::task::spawn_blocking(move || send_request_blocking(&socket_path, &request_json))
            .await
            .context("attestation blocking task panicked")??;

    let response: IssueResponse = serde_json::from_slice(&response_buf)
        .context("Failed to parse attestation service response")?;
    let data = response.into_result()?;
    hex::decode(&data.document).context("Failed to decode attestation document from hex")
}

/// Retrieve metadata from the attestation service (issuer type, etc.).
///
/// # Errors
///
/// Returns an error if the attestation service is unreachable or returns an error.
pub async fn metadata(socket_path: &str) -> Result<MetadataResponseData> {
    let request = Request {
        method: "metadata".to_string(),
        data: serde_json::to_value(MetadataRequestData {})?,
    };
    let request_json = serde_json::to_string(&request).context("Failed to serialize request")?;
    let socket_path = socket_path.to_owned();

    let response_buf =
        tokio::task::spawn_blocking(move || send_request_blocking(&socket_path, &request_json))
            .await
            .context("attestation blocking task panicked")??;

    let response: MetadataResponse = serde_json::from_slice(&response_buf)
        .context("Failed to parse attestation service response")?;
    response.into_result()
}

/// Blocking Unix-socket round-trip: connect → write → shutdown write → read all.
fn send_request_blocking(socket_path: &str, request_json: &str) -> Result<Vec<u8>> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("Failed to connect to attestation service at {socket_path}"))?;

    stream
        .write_all(request_json.as_bytes())
        .context("Failed to write request to socket")?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .context("Failed to shutdown write side of socket")?;

    let mut response_buf = Vec::new();
    stream
        .read_to_end(&mut response_buf)
        .context("Failed to read response from socket")?;

    Ok(response_buf)
}
