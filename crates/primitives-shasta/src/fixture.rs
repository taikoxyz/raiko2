use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub const DEFAULT_GUEST_INPUT_ROOT: &str = "test/guest_inputs/shasta";

/// Returns the directory that stores proposal guest input fixtures for a network.
///
/// # Errors
///
/// Returns an error when `network` is not a safe fixture key.
pub fn guest_input_proposals_dir(root: &Path, network: &str) -> Result<PathBuf> {
    let network = validate_fixture_key("network", network)?;
    Ok(root.join(network).join("proposals"))
}

/// Returns the directory that stores guest input suite fixtures for a network.
///
/// # Errors
///
/// Returns an error when `network` is not a safe fixture key.
pub fn guest_input_suites_dir(root: &Path, network: &str) -> Result<PathBuf> {
    let network = validate_fixture_key("network", network)?;
    Ok(root.join(network).join("suites"))
}

/// Returns the canonical proposal guest input fixture path.
///
/// # Errors
///
/// Returns an error when `network` is not a safe fixture key.
pub fn guest_input_proposal_path(root: &Path, network: &str, proposal_id: u64) -> Result<PathBuf> {
    Ok(guest_input_proposals_dir(root, network)?.join(format!("proposal_{proposal_id}.json")))
}

/// Returns the canonical guest input suite fixture path.
///
/// # Errors
///
/// Returns an error when `network` or `suite` is not a safe fixture key.
pub fn guest_input_suite_path(root: &Path, network: &str, suite: &str) -> Result<PathBuf> {
    let suite = validate_fixture_key("suite", suite)?;
    Ok(guest_input_suites_dir(root, network)?.join(format!("{suite}.json")))
}

#[must_use]
pub fn parse_guest_input_proposal_id(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let raw = name.strip_prefix("proposal_")?.strip_suffix(".json")?;
    raw.parse().ok()
}

/// Validates that a fixture key cannot escape the fixture root.
///
/// # Errors
///
/// Returns an error when `value` is empty, a parent/current directory marker, or contains
/// characters outside the fixture key allowlist.
pub fn validate_fixture_key<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() || value == "." || value == ".." {
        bail!("{label} must not be empty, '.' or '..'");
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        bail!("{label} must only contain ASCII letters, digits, '_', '.' or '-'");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{
        guest_input_proposal_path, guest_input_suite_path, parse_guest_input_proposal_id,
        validate_fixture_key,
    };
    use std::path::Path;

    #[test]
    fn guest_input_paths_use_network_and_proposal_keys() {
        let path =
            guest_input_proposal_path(Path::new("test/guest_inputs/shasta"), "taiko_hoodi", 42)
                .expect("proposal path");
        assert_eq!(
            path,
            Path::new("test/guest_inputs/shasta/taiko_hoodi/proposals/proposal_42.json")
        );

        let path = guest_input_suite_path(
            Path::new("test/guest_inputs/shasta"),
            "taiko_hoodi",
            "smoke",
        )
        .expect("suite path");
        assert_eq!(
            path,
            Path::new("test/guest_inputs/shasta/taiko_hoodi/suites/smoke.json")
        );
    }

    #[test]
    fn fixture_keys_reject_path_traversal() {
        for invalid in ["", ".", "..", "../hoodi", "hoodi/main", "hoodi main"] {
            assert!(validate_fixture_key("network", invalid).is_err());
        }
    }

    #[test]
    fn proposal_id_parser_accepts_canonical_names_only() {
        assert_eq!(
            parse_guest_input_proposal_id(Path::new("proposal_123.json")),
            Some(123)
        );
        assert_eq!(
            parse_guest_input_proposal_id(Path::new("proposal_x.json")),
            None
        );
        assert_eq!(parse_guest_input_proposal_id(Path::new("123.json")), None);
    }
}
