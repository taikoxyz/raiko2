use std::path::PathBuf;

pub(crate) fn trusted_temp_root() -> PathBuf {
    // macOS exposes the trusted temp root beneath symlinked `/var`; resolve only that OS-provided
    // root so the product's no-symlink input policy remains unchanged.
    std::fs::canonicalize(std::env::temp_dir()).expect("canonicalize trusted OS temp root")
}
