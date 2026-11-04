//! Standalone data generator. Build with `rustc --edition=2024 -O` (no Cargo).
//! U256 limbs are numbered from least significant (k0) to most significant (k3).
//! This is the observed specialization's hash, not a cryptographic collision tool.
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Write},
    path::Path,
    process::ExitCode,
    time::{Duration, Instant},
};

const MAX_KEYS: usize = 1024;
const CATALOG_KEYS: usize = 512;
const HASH_CAP: usize = 256 * 1024;
const CATALOG_CAP: usize = 128 * 1024;
const METADATA_CAP: usize = 16 * 1024;
// Check every 1024 trials and once at completion, including successful completion.
const DEADLINE_INTERVAL: u64 = 1024;
type Key = [u64; 4];
type Result<T> = std::result::Result<T, String>;

fn fold(x: u64, y: u64) -> u64 {
    let product = u128::from(x) * u128::from(y);
    product as u64 ^ (product >> 64) as u64
}

fn hash(key: Key, seed: u64, shared: u64) -> u64 {
    let a = fold(key[0] ^ seed, key[1] ^ shared);
    fold(key[2] ^ a, key[3] ^ shared)
}

fn hex(value: &str, digits: usize) -> Result<&str> {
    if value.len() != digits + 2
        || !value.starts_with("0x")
        || !value.as_bytes()[2..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return Err(format!(
            "expected canonical lowercase 0x plus {digits} hex digits"
        ));
    }
    Ok(&value[2..])
}

fn scalar(value: &str) -> Result<u64> {
    u64::from_str_radix(hex(value, 16)?, 16).map_err(|_| "invalid u64".into())
}

fn key(value: &str) -> Result<Key> {
    let value = hex(value, 64)?;
    let mut limbs = [0; 4];
    for (i, limb) in limbs.iter_mut().enumerate() {
        let end = 64 - i * 16;
        *limb = u64::from_str_radix(&value[end - 16..end], 16)
            .map_err(|_| "invalid U256".to_string())?;
    }
    Ok(limbs)
}

fn canonical(key: Key) -> String {
    format!(
        "0x{:016x}{:016x}{:016x}{:016x}",
        key[3], key[2], key[1], key[0]
    )
}

fn bounded_decimal(value: &str, maximum: u64) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err("expected positive decimal integer".into());
    }
    let number: u64 = value.parse().map_err(|_| "decimal integer overflow")?;
    if number == 0 || number > maximum {
        return Err(format!("integer must be in 1..={maximum}"));
    }
    Ok(number)
}

struct Arguments {
    mode: String,
    fields: BTreeMap<String, String>,
    keys: Vec<Key>,
}

impl Arguments {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mode = args.next().ok_or("expected hash or search")?;
        if mode != "hash" && mode != "search" {
            return Err("expected hash or search".into());
        }
        let mut fields = BTreeMap::new();
        let mut keys = Vec::new();
        for _ in 0..MAX_KEYS + 8 {
            let Some(flag) = args.next() else {
                return Ok(Self { mode, fields, keys });
            };
            let allowed = flag == "--seed"
                || flag == "--shared"
                || (mode == "hash" && flag == "--key")
                || (mode == "search"
                    && matches!(
                        flag.as_str(),
                        "--query" | "--bucket-mask" | "--max-trials" | "--max-seconds" | "--output"
                    ));
            if !allowed {
                return Err("unknown flag".into());
            }
            let value = args.next().ok_or("missing flag value")?;
            if flag == "--key" {
                if keys.len() == MAX_KEYS {
                    return Err("hash accepts at most 1024 keys".into());
                }
                keys.push(key(&value)?);
            } else if fields.insert(flag, value).is_some() {
                return Err("duplicate scalar flag".into());
            }
        }
        Err("too many arguments".into())
    }

    fn get(&self, name: &str) -> Result<&str> {
        self.fields
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {name}"))
    }
}

fn permutation(counter: u32) -> u32 {
    let z = counter.wrapping_add(0x9e3779b9);
    let z = (z ^ (z >> 16)).wrapping_mul(0x85ebca6b);
    let z = (z ^ (z >> 13)).wrapping_mul(0xc2b2ae35);
    z ^ (z >> 16)
}

struct Search {
    seed: u64,
    shared: u64,
    query: Key,
    mask: u64,
    max_trials: u64,
    max_seconds: u64,
}

struct Outcome {
    keys: Vec<Key>,
    trials: u64,
    status: &'static str,
    elapsed: Duration,
}

impl Search {
    fn parse(args: &Arguments, seed: u64, shared: u64) -> Result<Self> {
        let query = key(args.get("--query")?)?;
        if query[..3] != [0, 0, 0] || query[3].to_be_bytes().contains(&0) {
            return Err("query requires zero low192 bits and eight nonzero high64 bytes".into());
        }
        let mask = bounded_decimal(args.get("--bucket-mask")?, 65535)?;
        if !(mask + 1).is_power_of_two() {
            return Err("bucket mask must be a positive power-of-two-minus-one".into());
        }
        Ok(Self {
            seed,
            shared,
            query,
            mask,
            max_trials: bounded_decimal(args.get("--max-trials")?, 268435456)?,
            max_seconds: bounded_decimal(args.get("--max-seconds")?, 120)?,
        })
    }

    fn run(&self) -> Outcome {
        self.run_with_clock(Instant::now())
    }

    fn run_with_clock(&self, start: Instant) -> Outcome {
        let target = hash(self.query, self.seed, self.shared);
        let deadline = Duration::from_secs(self.max_seconds);
        let mut keys = Vec::with_capacity(CATALOG_KEYS);
        let mut trials = 0;
        let mut status = "trial_limit";
        while trials < self.max_trials && keys.len() < CATALOG_KEYS - 1 {
            if trials % DEADLINE_INTERVAL == 0 && start.elapsed() >= deadline {
                status = "time_limit";
                break;
            }
            // The permutation is bijective on u32; the bounded counter cannot wrap.
            let high = (u64::from(permutation(trials as u32)) << 32) | (self.query[3] & 0xffffffff);
            trials += 1;
            if high == self.query[3] || high.to_be_bytes().contains(&0) {
                continue;
            }
            let candidate = [0, 0, 0, high];
            let h = hash(candidate, self.seed, self.shared);
            if h & self.mask == target & self.mask && h >> 57 == target >> 57 {
                keys.push(candidate);
            }
        }
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            status = "time_limit";
        } else if keys.len() == CATALOG_KEYS - 1 {
            status = "complete";
        }
        // Query is known to match, is excluded from candidates and always last.
        keys.push(self.query);
        Outcome {
            keys,
            trials,
            status,
            elapsed,
        }
    }

    fn metadata(&self, result: &Outcome) -> String {
        let target = hash(self.query, self.seed, self.shared);
        format!(
            "{{\"schema_version\":1,\"status\":\"{}\",\"trials\":{},\"found\":{},\"retained_candidates\":{},\"elapsed_ns\":{},\"seed\":\"0x{:016x}\",\"shared\":\"0x{:016x}\",\"query\":\"{}\",\"query_hash\":\"0x{:016x}\",\"bucket_mask\":{},\"target_bucket\":{},\"target_tag\":{},\"max_trials\":{},\"max_seconds\":{},\"deadline_check_interval\":{}}}\n",
            result.status,
            result.trials,
            result.keys.len(),
            result.keys.len() - 1,
            result.elapsed.as_nanos(),
            self.seed,
            self.shared,
            canonical(self.query),
            target,
            self.mask,
            target & self.mask,
            target >> 57,
            self.max_trials,
            self.max_seconds,
            DEADLINE_INTERVAL
        )
    }
}

fn catalog(keys: &[Key]) -> Result<String> {
    if keys.len() != CATALOG_KEYS {
        return Err("incomplete catalog".into());
    }
    let list = keys
        .iter()
        .map(|k| format!("\"{}\"", canonical(*k)))
        .collect::<Vec<_>>()
        .join(",");
    let result = format!("{{\"schema_version\":1,\"keys\":[{list}]}}\n");
    if result.len() > CATALOG_CAP {
        return Err("catalog size cap".into());
    }
    Ok(result)
}

struct OwnedTemporary(std::path::PathBuf);
impl Drop for OwnedTemporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn install(output: &Path, data: &[u8]) -> Result<()> {
    if data.len() > CATALOG_CAP {
        return Err("catalog size cap".into());
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // create_new owns the temp; hard_link is an atomic no-replace install, even
    // against a concurrent creator or a dangling symlink at the destination.
    for counter in 0..64 {
        let path = parent.join(format!(
            ".hash-probe-catalog-{}-{counter}.tmp",
            std::process::id()
        ));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("cannot create owned temporary file".into()),
        };
        let owned = OwnedTemporary(path);
        let written = file.write_all(data).and_then(|()| file.sync_all());
        drop(file);
        written.map_err(|_| "cannot write catalog temporary file")?;
        fs::hard_link(&owned.0, output)
            .map_err(|_| "cannot install catalog without overwriting destination")?;
        return Ok(());
    }
    Err("temporary filename attempts exhausted".into())
}

fn emit(data: &str, cap: usize) -> Result<()> {
    if data.len() > cap {
        return Err("JSON output size cap".into());
    }
    io::stdout()
        .lock()
        .write_all(data.as_bytes())
        .map_err(|_| "cannot write JSON output".into())
}

fn run(args: Arguments) -> Result<bool> {
    let seed = scalar(args.get("--seed")?)?;
    let shared = scalar(args.get("--shared")?)?;
    if args.mode == "hash" {
        if args.keys.is_empty() {
            return Err("hash requires 1..=1024 keys".into());
        }
        let rows = args
            .keys
            .iter()
            .map(|k| {
                format!(
                    "{{\"key\":\"{}\",\"hash\":\"0x{:016x}\"}}",
                    canonical(*k),
                    hash(*k, seed, shared)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        emit(
            &format!(
                "{{\"schema_version\":1,\"seed\":\"0x{seed:016x}\",\"shared\":\"0x{shared:016x}\",\"hashes\":[{rows}]}}\n"
            ),
            HASH_CAP,
        )?;
        return Ok(true);
    }
    let search = Search::parse(&args, seed, shared)?;
    let output = args.get("--output")?;
    if output.is_empty() {
        return Err("output path cannot be empty".into());
    }
    let output = Path::new(output);
    if fs::symlink_metadata(output).is_ok() {
        return Err("output already exists".into());
    }
    let outcome = search.run();
    let metadata = search.metadata(&outcome);
    if metadata.len() > METADATA_CAP {
        return Err("search metadata size cap".into());
    }
    if outcome.status == "complete" {
        install(output, catalog(&outcome.keys)?.as_bytes())?;
    }
    emit(&metadata, METADATA_CAP)?;
    Ok(outcome.status == "complete")
}

fn main() -> ExitCode {
    let args = env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|_| "arguments must be UTF-8".to_string())
        })
        .collect::<Result<Vec<_>>>();
    match args.and_then(Arguments::parse).and_then(run) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(message) => {
            eprintln!("hash-probe-catalog: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_product_and_little_endian_limbs() {
        assert_eq!(fold(u64::MAX, u64::MAX), u64::MAX);
        assert_eq!(fold(1 << 63, 2), 1);
        assert_eq!(
            key("0x0000000000000004000000000000000300000000000000020000000000000001").unwrap(),
            [1, 2, 3, 4]
        );
        assert_eq!(hash([1, 2, 3, 4], 0, 0), 4);
        assert_eq!(
            canonical([1, 2, 3, 4]),
            "0x0000000000000004000000000000000300000000000000020000000000000001"
        );
    }

    #[test]
    fn expired_monotonic_budget_cannot_produce_catalog() {
        let search = Search {
            seed: 0,
            shared: 0,
            query: [0, 0, 0, 0x1122334455667788],
            mask: 1,
            max_trials: 1000,
            max_seconds: 1,
        };
        let result = search.run_with_clock(Instant::now() - Duration::from_secs(2));
        assert_eq!(
            (result.status, result.trials, result.keys.len()),
            ("time_limit", 0, 1)
        );
        assert!(catalog(&result.keys).is_err());
    }

    #[test]
    fn atomic_install_itself_refuses_replacement_and_cleans_temporary() {
        let root = env::temp_dir().join(format!("hash-probe-install-unit-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let output = root.join("catalog.json");
        install(&output, b"original").unwrap();
        assert!(install(&output, b"replacement").is_err());
        assert_eq!(fs::read(&output).unwrap(), b"original");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_file(output).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
