use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::request::Parts,
};
use serde::Serialize;
use std::future::Future;
use tracing::info;

use super::auth::authorize_acl_feature;
use super::errors::ApiError;
use crate::config::ServerAclFeature;
use crate::server::sampling::{BallotConfig, ZkAnySampler};
use crate::server::state::AppState;

pub(crate) struct AdminBallotReadAuth;
pub(crate) struct AdminBallotWriteAuth;

#[derive(Serialize)]
pub(crate) struct AdminStatus {
    status: &'static str,
}

pub(crate) async fn get_ballot(
    _: AdminBallotReadAuth,
    State(state): State<AppState>,
) -> Result<Json<BallotConfig>, ApiError> {
    let sampler = state
        .zk_any_sampler
        .lock()
        .map_err(|_| ApiError::internal("failed to lock zk_any sampler"))?;
    Ok(Json(sampler.to_ballot_config()))
}

pub(crate) async fn set_ballot(
    _: AdminBallotWriteAuth,
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

impl FromRequestParts<AppState> for AdminBallotReadAuth {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let result =
            authorize_acl_feature(state, &parts.headers, ServerAclFeature::AdminBallotRead)
                .map(|()| Self);
        async move { result }
    }
}

impl FromRequestParts<AppState> for AdminBallotWriteAuth {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let result =
            authorize_acl_feature(state, &parts.headers, ServerAclFeature::AdminBallotWrite)
                .map(|()| Self);
        async move { result }
    }
}
