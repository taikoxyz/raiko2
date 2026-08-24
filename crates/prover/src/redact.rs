//! Redaction helpers for text that may embed RPC credentials.
//!
//! Authenticated RPC endpoints have to carry their token inside the URL, either
//! as userinfo or as a query parameter: neither the alloy HTTP transport nor the
//! Boundless SDK client exposes a per-request header hook, so the URL is the only
//! place a credential can travel. Both `reqwest` and `alloy` render the full URL
//! into their error `Display` (`reqwest` appends `" for url ({url})"`), which means
//! any message built from `format!("...: {err}")` carries the token into logs and
//! into persisted task state.
//!
//! Everything that renders an external error from such a client must pass the
//! rendering through [`redact_urls`] first.

use url::Url;

/// Characters that cannot appear in a URI (RFC 3986) and therefore end one.
///
/// ASCII whitespace and control characters terminate a URL too; they are checked
/// separately so the table stays limited to printable exclusions.
const URL_TERMINATORS: &[char] = &['"', '<', '>', '\\', '^', '`', '{', '|', '}'];

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

/// Replace every URL embedded in `text` with its [`sanitize_url_for_log`] form.
///
/// Used to render external errors: the caller cannot know whether a given error
/// type embeds the endpoint it failed to reach, so the rendering is scrubbed
/// unconditionally.
#[must_use]
pub fn redact_urls(text: &str) -> String {
    // Only scheme-qualified URLs can carry a credential, and every renderer we
    // care about emits the fully serialized form.
    if !text.contains("://") {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;

    while let Some(offset) = text[cursor..].find("://") {
        let separator = cursor + offset;
        // `max(cursor)` keeps the scheme walk inside the unconsumed remainder so
        // the slices below can never run backwards.
        let start = scheme_start(text, separator).max(cursor);
        let end = url_end(text, start, separator + "://".len());

        // `start == separator` means the "://" had no scheme in front of it, so
        // there is no URL to redact here; emit the span verbatim and move on.
        if start < separator {
            out.push_str(&text[cursor..start]);
            out.push_str(&sanitize_url_for_log(&text[start..end]));
        } else {
            out.push_str(&text[cursor..end]);
        }
        cursor = end;
    }

    out.push_str(&text[cursor..]);
    out
}

/// Walk back from `separator` over the scheme, returning its start offset.
///
/// Returns `separator` itself when no valid scheme precedes it, which the caller
/// treats as "not a URL".
fn scheme_start(text: &str, separator: usize) -> usize {
    let start = text[..separator]
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
        .last()
        .map_or(separator, |(index, _)| index);

    // A scheme must begin with a letter; anything else is not a URL.
    match text[start..].chars().next() {
        Some(ch) if ch.is_ascii_alphabetic() => start,
        _ => separator,
    }
}

/// Walk forward from `from` to the end of the URL span.
fn url_end(text: &str, start: usize, from: usize) -> usize {
    let span_len = text[from..]
        .find(|ch: char| ch.is_whitespace() || ch.is_control() || URL_TERMINATORS.contains(&ch))
        .unwrap_or(text.len() - from);
    let mut end = from + span_len;

    // Preserve a confirmed paired wrapper such as reqwest's `url (https://...)`.
    // Other punctuation is valid URI data and must stay inside the span so query
    // credentials made only of punctuation cannot be copied back verbatim.
    let opener = text[..start].chars().next_back();
    let closer = text[..end].chars().next_back();
    if matches!(
        (opener, closer),
        (Some('('), Some(')')) | (Some('['), Some(']'))
    ) && let Some(closer) = closer
    {
        end -= closer.len_utf8();
    }

    end
}

#[cfg(test)]
mod tests {
    use super::{redact_urls, sanitize_url_for_log};

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

    #[test]
    fn redact_urls_scrubs_a_reqwest_style_error_rendering() {
        let rendered =
            "error sending request for url (https://rpc.mainnet.taiko.xyz/?secret=SENTINEL)";

        let redacted = redact_urls(rendered);

        assert!(!redacted.contains("SENTINEL"), "{redacted}");
        assert_eq!(
            redacted,
            "error sending request for url (https://rpc.mainnet.taiko.xyz/)"
        );
    }

    #[test]
    fn redact_urls_scrubs_every_url_in_one_message() {
        let rendered = "https://a.test/?secret=ONE failed, retrying https://b.test/?secret=TWO";

        let redacted = redact_urls(rendered);

        assert!(!redacted.contains("ONE"), "{redacted}");
        assert!(!redacted.contains("TWO"), "{redacted}");
        assert_eq!(redacted, "https://a.test/ failed, retrying https://b.test/");
    }

    #[test]
    fn redact_urls_scrubs_userinfo_credentials() {
        let redacted = redact_urls("connect failed: http://user:HUNTER2@rpc.test:8545/");

        assert!(!redacted.contains("HUNTER2"), "{redacted}");
    }

    #[test]
    fn redact_urls_leaves_credential_free_text_alone() {
        let rendered =
            "HTTP status server error (500 Internal Server Error) for url (http://rpc.test:8545/)";

        assert_eq!(redact_urls(rendered), rendered);
        assert_eq!(redact_urls("nonce too low"), "nonce too low");
    }

    #[test]
    fn redact_urls_ignores_a_bare_scheme_separator() {
        assert_eq!(
            redact_urls("://no-scheme?secret=X"),
            "://no-scheme?secret=X"
        );
    }

    #[test]
    fn redact_urls_scrubs_a_token_containing_trailing_punctuation() {
        // Base64 padding must survive the trailing-punctuation trim.
        let redacted = redact_urls("for url (https://rpc.test/?secret=YWJj==)");

        assert!(!redacted.contains("YWJj"), "{redacted}");
    }

    #[test]
    fn redact_urls_keeps_url_valid_query_punctuation_inside_the_redacted_span() {
        for (credential, punctuation) in [("!!!!", "!"), (",,,,", ","), (";;;;", ";")] {
            let rendered = format!("for url (https://rpc.test/?secret={credential})");

            let redacted = redact_urls(&rendered);

            assert_eq!(redacted, "for url (https://rpc.test/)");
            assert!(!redacted.contains(punctuation), "{redacted}");
        }
    }

    #[test]
    fn redact_urls_preserves_a_bracketed_ipv6_endpoint() {
        let rendered = "for url (http://user:pass@[2001:db8::1])";

        assert_eq!(redact_urls(rendered), "for url (http://[2001:db8::1]/)");
    }
}
