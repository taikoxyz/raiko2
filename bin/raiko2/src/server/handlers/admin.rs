use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, request::Parts},
};
use serde::Serialize;
use std::future::Future;
use tracing::info;

use super::errors::ApiError;
use crate::server::sampling::{BallotConfig, ZkAnySampler};
use crate::server::state::AppState;

const API_KEY_HEADER: &str = "x-api-key";

pub(crate) struct AdminAuth;

#[derive(Serialize)]
pub(crate) struct AdminStatus {
    status: &'static str,
}

pub(crate) async fn get_ballot(
    _: AdminAuth,
    State(state): State<AppState>,
) -> Result<Json<BallotConfig>, ApiError> {
    let sampler = state
        .zk_any_sampler
        .lock()
        .map_err(|_| ApiError::internal("failed to lock zk_any sampler"))?;
    Ok(Json(sampler.to_ballot_config()))
}

pub(crate) async fn set_ballot(
    _: AdminAuth,
    State(state): State<AppState>,
    Json(ballot): Json<BallotConfig>,
) -> Result<Json<AdminStatus>, ApiError> {
    let new_sampler =
        ZkAnySampler::from_ballot_config(ballot.clone()).map_err(ApiError::bad_request)?;
    let mut sampler = state
        .zk_any_sampler
        .lock()
        .map_err(|_| ApiError::internal("failed to lock zk_any sampler"))?;
    let old_ballot = sampler.to_ballot_config();
    *sampler = new_sampler;
    info!(
        old_policy = ?old_ballot,
        new_policy = ?ballot,
        "updated zk_any ballot"
    );
    Ok(Json(AdminStatus { status: "ok" }))
}

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = authorize_admin(state, &parts.headers).map(|()| Self);
        async move { result }
    }
}

fn authorize_admin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected_key) = state.config.server.admin_api_key.as_deref() else {
        return Err(ApiError::not_found("admin API is not enabled"));
    };
    let Some(actual_key) = headers
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::unauthorized("missing admin API key"));
    };
    if !constant_time_eq(actual_key, expected_key) {
        return Err(ApiError::unauthorized("invalid admin API key"));
    }
    Ok(())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut diff = left.len() ^ right.len();

    for idx in 0..left.len().max(right.len()) {
        let left_byte = left.get(idx).copied().unwrap_or_default();
        let right_byte = right.get(idx).copied().unwrap_or_default();
        diff |= usize::from(left_byte ^ right_byte);
    }

    diff == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_matches_string_equality() {
        assert!(constant_time_eq("secret-admin-key", "secret-admin-key"));
        assert!(!constant_time_eq("secret-admin-key", "secret-admin-kex"));
        assert!(!constant_time_eq(
            "secret-admin-key",
            "secret-admin-key-extra"
        ));
    }
}
