use axum::http::HeaderMap;

use super::errors::ApiError;
use crate::config::ServerAclFeature;
use crate::server::state::AppState;

pub(crate) const API_KEY_HEADER: &str = "x-api-key";

pub(crate) fn authorize_acl_feature(
    state: &AppState,
    headers: &HeaderMap,
    feature: ServerAclFeature,
) -> Result<(), ApiError> {
    let feature_enabled = state
        .config
        .server
        .acl
        .keys
        .iter()
        .any(|key| key.allow.contains(&feature));
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
    let mut authorized = false;
    for key in &state.config.server.acl.keys {
        let matches = constant_time_eq(actual_key, &key.key);
        let allows_feature = key.allow.contains(&feature);
        key_known |= matches;
        authorized |= matches && allows_feature;
    }
    if authorized {
        return Ok(());
    }

    if key_known {
        return Err(ApiError::forbidden(
            "API key is not allowed for this feature",
        ));
    }

    Err(ApiError::unauthorized("invalid API key"))
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
