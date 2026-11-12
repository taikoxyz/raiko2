use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).context("inspect input")?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "input must be a regular file of at most {limit} bytes"
    );
    let file = File::open(path).context("open input")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "input type or size changed"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "input exceeded size limit while reading"
    );
    Ok(bytes)
}

pub(crate) fn executable_sha256() -> Result<String> {
    #[cfg(target_os = "linux")]
    let mut file = File::open("/proc/self/exe")?;
    #[cfg(not(target_os = "linux"))]
    let mut file = File::open(std::env::current_exe()?)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn reserve(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .context("reserve new output")
}

pub(crate) fn write_json(mut output: File, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    output.sync_all().context("sync report")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_regular_input_and_exclusive_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.bin");
        fs::write(&input, b"abc").unwrap();
        assert_eq!(read_regular(&input, 3).unwrap(), b"abc");
        assert!(read_regular(&input, 2).is_err());
        assert!(read_regular(dir.path(), 100).is_err());
        assert!(reserve(&input).is_err());
        assert_eq!(fs::read(&input).unwrap(), b"abc");
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&input, &link).unwrap();
            assert!(read_regular(&link, 3).is_err());
        }
    }
}
