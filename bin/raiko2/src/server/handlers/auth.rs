use axum::http::HeaderMap;
use std::time::Duration;

use super::errors::ApiError;
use crate::config::ServerAclFeature;
use crate::server::state::AppState;

pub(crate) const API_KEY_HEADER: &str = "x-api-key";
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

pub(crate) fn authorize_acl_feature(
    state: &AppState,
    headers: &HeaderMap,
    feature: ServerAclFeature,
) -> Result<(), ApiError> {
    authorize_acl_key(state, headers, feature).map(|_| ())
}

pub(crate) fn authorize_acl_feature_with_rate_limit(
    state: &AppState,
    headers: &HeaderMap,
    feature: ServerAclFeature,
) -> Result<(), ApiError> {
    let (key_index, rate_limit_per_minute) = authorize_acl_key(state, headers, feature)?;
    enforce_rate_limit(state, key_index, rate_limit_per_minute)
}

fn authorize_acl_key(
    state: &AppState,
    headers: &HeaderMap,
    feature: ServerAclFeature,
) -> Result<(usize, Option<u32>), ApiError> {
    let feature_enabled = state
        .config
        .server
        .acl
        .keys
        .iter()
        .any(|key| acl_allows_feature(&key.allow, feature));
    if !feature_enabled {
        return Err(ApiError::not_found("ACL feature is not enabled"));
    }

    let Some(actual_key) = headers
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::unauthorized("missing API key"));
    };
    let max_key_len = state
        .config
        .server
        .acl
        .keys
        .iter()
        .map(|key| key.key.len())
        .max()
        .unwrap_or_default();
    if actual_key.len() > max_key_len {
        return Err(ApiError::unauthorized("invalid API key"));
    }

    let mut key_known = false;
    let mut authorized = None;
    for (key_index, key) in state.config.server.acl.keys.iter().enumerate() {
        let matches = constant_time_eq(actual_key, &key.key);
        let allows_feature = acl_allows_feature(&key.allow, feature);
        key_known |= matches;
        if matches && allows_feature {
            authorized = Some((key_index, key.rate_limit_per_minute));
        }
    }
    if let Some(key) = authorized {
        return Ok(key);
    }

    if key_known {
        return Err(ApiError::forbidden(
            "API key is not allowed for this feature",
        ));
    }

    Err(ApiError::unauthorized("invalid API key"))
}

fn enforce_rate_limit(
    state: &AppState,
    key_index: usize,
    limit_per_minute: Option<u32>,
) -> Result<(), ApiError> {
    let Some(limit_per_minute) = limit_per_minute else {
        return Ok(());
    };

    let allowed = state
        .acl_rate_limiter
        .allow_request(key_index, limit_per_minute, RATE_LIMIT_WINDOW)
        .map_err(|()| ApiError::internal("failed to lock ACL rate limiter"))?;
    if !allowed {
        return Err(ApiError::too_many_requests("rate limit exceeded"));
    }
    Ok(())
}

fn acl_allows_feature(allow: &[ServerAclFeature], feature: ServerAclFeature) -> bool {
    allow.contains(&ServerAclFeature::Admin) || allow.contains(&feature)
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
        assert!(constant_time_eq("secret-api-key", "secret-api-key"));
        assert!(!constant_time_eq("secret-api-key", "secret-api-kex"));
        assert!(!constant_time_eq("secret-api-key", "secret-api-key-extra"));
    }
}
