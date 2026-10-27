//! Redaction helpers for text that may embed RPC credentials.
//!
//! Authenticated RPC endpoints may carry tokens inside URL userinfo or query parameters.
//! HTTP clients can include the full URL in error messages, exposing credentials to logs
//! and persisted task state unless those messages are sanitized.
//!
use url::Url;

/// Strip credentials, query, and fragment from a single URL.
///
/// Falls back to `<redacted-url>` when `raw` cannot be parsed but still looks
/// like it carries a credential, and returns `raw` unchanged when it is neither
/// a URL nor credential-shaped.
#[must_use]
pub fn sanitize_url_for_log(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        if raw.contains('@') || raw.contains('?') || raw.contains('#') {
            return "<redacted-url>".to_string();
        }
        return raw.to_string();
    };

    if url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
    {
        // Nothing to strip. Return the caller's exact spelling rather than the
        // normalized one, so a plain endpoint logs the way it was configured.
        return raw.to_string();
    }

    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::sanitize_url_for_log;

    #[test]
    fn sanitize_url_strips_query_userinfo_and_fragment() {
        assert_eq!(
            sanitize_url_for_log("https://rpc.example.test/?secret=token#frag"),
            "https://rpc.example.test/"
        );
        assert_eq!(
            sanitize_url_for_log("https://user:pass@rpc.example.test:8545/path"),
            "https://rpc.example.test:8545/path"
        );
    }

    #[test]
    fn sanitize_url_preserves_a_plain_endpoint() {
        assert_eq!(
            sanitize_url_for_log("http://l2-node.svc.cluster.local:8545"),
            "http://l2-node.svc.cluster.local:8545"
        );
    }

    #[test]
    fn sanitize_url_redacts_unparseable_credential_shaped_input() {
        assert_eq!(
            sanitize_url_for_log("not a url?secret=token"),
            "<redacted-url>"
        );
        assert_eq!(sanitize_url_for_log("plain-text"), "plain-text");
    }
}
