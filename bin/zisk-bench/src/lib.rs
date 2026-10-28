//! Validation for CPU-only ZisK profiling reports.

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::BTreeMap;

/// Run only the emulator and validate its artifacts, retaining raw diagnostics.
pub fn execute_cpu(
    emulator: &std::path::Path,
    elf: &std::path::Path,
    out: &std::path::Path,
    expected: &[u8; 32],
    max_steps: u64,
) -> Result<(WeightedCost, f64)> {
    use std::{
        fs::{self, File},
        process::{Command, Stdio},
        time::Instant,
    };
    let started = Instant::now();
    let status = Command::new(emulator)
        .arg("--elf")
        .arg(elf)
        .arg("--inputs")
        .arg(out.join("input.bin"))
        .arg("--output")
        .arg(out.join("output.bin"))
        .arg("--max-steps")
        .arg(max_steps.to_string())
        .args(["-X", "--no-thousands-sep", "--csv-separator", ","])
        .arg("--save-stats")
        .arg(out.join("stats.csv"))
        .stdin(Stdio::null())
        .stdout(File::create(out.join("emulator.stdout.log"))?)
        .stderr(File::create(out.join("emulator.stderr.log"))?)
        .status()
        .context("launch CPU emulator")?;
    let elapsed = started.elapsed().as_secs_f64();
    ensure!(
        status.success(),
        "emulator exited {status}; inspect logs in {}",
        out.display()
    );
    let output = fs::read(out.join("output.bin")).context("read emulator public output")?;
    let stats =
        fs::read_to_string(out.join("stats.csv")).context("read emulator cost statistics")?;
    let costs = validate_execution(status.success(), &output, expected, &stats, max_steps)?;
    Ok((costs, elapsed))
}

/// Operation-weighted emulator cost, distinct from setup-backed cells and SP1 gas.
#[derive(Debug, Serialize)]
pub struct WeightedCost {
    pub steps: u64,
    pub total: u64,
    pub main: u64,
    pub opcodes: u64,
    pub precompiles: u64,
    pub memory: u64,
    pub base: u64,
}

/// Parse pinned `ziskemu --save-stats --csv-separator ,` output strictly.
pub fn parse_stats(stats: &str, max_steps: u64) -> Result<WeightedCost> {
    let mut steps = None;
    let mut costs = BTreeMap::new();
    for line in stats.lines() {
        let fields: Vec<_> = line.split(',').collect();
        match fields.as_slice() {
            ["STEPS", count] => {
                ensure!(steps.is_none(), "duplicate STEPS");
                steps = Some(count.parse::<u64>().context("invalid steps")?);
            }
            [
                "COST",
                name @ ("MAIN" | "OPCODES" | "PRECOMPILES" | "MEMORY" | "BASE" | "TOTAL"),
                value,
                ..,
            ] => {
                let value = value
                    .parse::<u64>()
                    .with_context(|| format!("invalid {name} cost"))?;
                ensure!(
                    costs.insert(*name, value).is_none(),
                    "duplicate {name} cost"
                );
            }
            _ => {}
        }
    }
    let cost = |key| {
        costs
            .get(key)
            .copied()
            .with_context(|| format!("missing {key} cost"))
    };
    let parsed = WeightedCost {
        steps: steps.context("missing STEPS")?,
        total: cost("TOTAL")?,
        main: cost("MAIN")?,
        opcodes: cost("OPCODES")?,
        precompiles: cost("PRECOMPILES")?,
        memory: cost("MEMORY")?,
        base: cost("BASE")?,
    };
    ensure!(
        parsed.steps > 0 && parsed.steps < max_steps,
        "zero steps or execution step limit reached"
    );
    let sum = [
        parsed.main,
        parsed.opcodes,
        parsed.precompiles,
        parsed.memory,
        parsed.base,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .context("cost sum overflow")?;
    ensure!(sum == parsed.total, "inconsistent weighted cost total");
    Ok(parsed)
}

/// Reject partial/failed execution even if it happened to write a matching prefix.
pub fn validate_execution(
    success: bool,
    output: &[u8],
    expected: &[u8; 32],
    stats: &str,
    max_steps: u64,
) -> Result<WeightedCost> {
    ensure!(success, "emulator process failed; inspect emulator logs");
    ensure!(
        output.len() == 256,
        "expected 256-byte emulator public output"
    );
    let slots: Vec<_> = output
        .chunks_exact(4)
        .map(|chunk| u64::from(u32::from_le_bytes(chunk.try_into().expect("four bytes"))))
        .collect();
    let digest = raiko2_primitives_shasta::zisk::digest_from_public_slots(&slots)?;
    ensure!(&digest == expected, "native/guest output mismatch");
    parse_stats(stats, max_steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATS: &str = "STEPS,10\nCOST,COST DISTRIBUTION,COST,%\nCOST,MAIN,20,\nCOST,OPCODES,30,\nCOST,PRECOMPILES,40,\nCOST,MEMORY,50,\nCOST,BASE,60,\nCOST,TOTAL,200,\n";

    #[test]
    fn parses_weighted_cost_without_calling_it_gas_or_cells() {
        let stats = parse_stats(STATS, 11).unwrap();
        assert_eq!(stats.steps, 10);
        assert_eq!(stats.total, 200);
        assert_eq!(stats.precompiles, 40);
        assert!(parse_stats(STATS, 10).is_err());
    }

    #[test]
    fn missing_invalid_duplicate_and_inconsistent_costs_are_rejected() {
        assert!(parse_stats("", 100).is_err());
        assert!(parse_stats(&STATS.replace("200", "201"), 100).is_err());
        assert!(parse_stats(&STATS.replace("COST,MEMORY,50,\n", ""), 100).is_err());
        assert!(parse_stats(&STATS.replace("COST,MAIN,20", "COST,MAIN,garbage"), 100).is_err());
        assert!(parse_stats(&format!("{STATS}COST,MAIN,20,\n"), 100).is_err());
    }

    #[test]
    fn failure_truncated_output_and_mismatched_digest_are_rejected() {
        assert!(validate_execution(false, &[0; 256], &[0; 32], STATS, 100).is_err());
        assert!(validate_execution(true, &[0; 32], &[0; 32], STATS, 100).is_err());
        assert!(validate_execution(true, &[0; 256], &[1; 32], STATS, 100).is_err());
        assert!(validate_execution(true, &[0; 256], &[0; 32], STATS, 100).is_ok());
        let mut output = [0; 256];
        output[255] = 1;
        assert!(validate_execution(true, &output, &[0; 32], STATS, 100).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn cpu_process_preserves_logs_and_rejects_nonzero_exit_even_with_valid_files() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let emulator = dir.path().join("test-emulator.sh");
        fs::write(
            &emulator,
            "#!/bin/sh\nprintf 'diagnostic\\n' >&2\nexit 42\n",
        )
        .unwrap();
        fs::set_permissions(&emulator, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(dir.path().join("output.bin"), [0; 256]).unwrap();
        fs::write(dir.path().join("stats.csv"), STATS).unwrap();
        let err = execute_cpu(
            &emulator,
            &dir.path().join("guest.elf"),
            dir.path(),
            &[0; 32],
            100,
        )
        .unwrap_err();
        assert!(err.to_string().contains("42"));
        assert_eq!(
            fs::read_to_string(dir.path().join("emulator.stderr.log")).unwrap(),
            "diagnostic\n"
        );
    }

    #[test]
    #[cfg(unix)]
    fn cpu_process_validates_artifacts_after_successful_exit() {
        use std::{fs, os::unix::fs::PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let emulator = dir.path().join("test-emulator.sh");
        fs::write(&emulator, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&emulator, fs::Permissions::from_mode(0o700)).unwrap();
        let elf = dir.path().join("guest.elf");
        assert!(execute_cpu(&emulator, &elf, dir.path(), &[0; 32], 100).is_err());
        fs::write(dir.path().join("output.bin"), [0; 256]).unwrap();
        fs::write(dir.path().join("stats.csv"), "malformed").unwrap();
        assert!(execute_cpu(&emulator, &elf, dir.path(), &[0; 32], 100).is_err());
        fs::write(dir.path().join("stats.csv"), STATS).unwrap();
        assert!(execute_cpu(&emulator, &elf, dir.path(), &[1; 32], 100).is_err());
        let (costs, _) = execute_cpu(&emulator, &elf, dir.path(), &[0; 32], 100).unwrap();
        assert_eq!(costs.total, 200);
    }
}
