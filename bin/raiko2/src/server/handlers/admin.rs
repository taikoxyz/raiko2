use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{HeaderMap, request::Parts},
};
use serde::Serialize;
use std::future::Future;
use tracing::info;

use super::auth::header_api_key_matches;
use super::errors::ApiError;
use crate::server::sampling::{BallotConfig, ZkAnySampler};
use crate::server::state::AppState;

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
    let Some(matches) = header_api_key_matches(headers, expected_key) else {
        return Err(ApiError::unauthorized("missing admin API key"));
    };
    if !matches {
        return Err(ApiError::unauthorized("invalid admin API key"));
    }
    Ok(())
}
