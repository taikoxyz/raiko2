use alethia_reth_evm::zk_gas::{schedule::FAILSAFE_MULTIPLIER, unzen::UNZEN_ZK_GAS_SCHEDULE};
use anyhow::{Context, Result, ensure};
use serde::Serialize;

#[derive(Serialize)]
struct ScheduleExport {
    opcodes: Vec<OpcodeMultiplier>,
    precompiles: Vec<PrecompileMultiplier>,
}

#[derive(Serialize)]
struct OpcodeMultiplier {
    opcode: String,
    multiplier: u16,
}

#[derive(Serialize)]
struct PrecompileMultiplier {
    address: String,
    multiplier: u16,
}

fn build_schedule_export() -> Result<ScheduleExport> {
    let opcodes: Vec<OpcodeMultiplier> = UNZEN_ZK_GAS_SCHEDULE
        .opcode_multipliers
        .iter()
        .enumerate()
        .filter(|(_, multiplier)| **multiplier != FAILSAFE_MULTIPLIER)
        .map(|(opcode, &multiplier)| OpcodeMultiplier {
            opcode: format!("0x{opcode:02x}"),
            multiplier,
        })
        .collect();
    let precompiles: Vec<PrecompileMultiplier> = UNZEN_ZK_GAS_SCHEDULE
        .precompile_multipliers
        .iter()
        .map(|(address, multiplier)| PrecompileMultiplier {
            address: format!("{address:#x}"),
            multiplier: *multiplier,
        })
        .collect();
    ensure!(!opcodes.is_empty(), "Unzen opcode schedule is empty");
    ensure!(
        !precompiles.is_empty(),
        "Unzen precompile schedule is empty"
    );
    Ok(ScheduleExport {
        opcodes,
        precompiles,
    })
}

pub(crate) fn run() -> Result<()> {
    let output = build_schedule_export()?;
    println!(
        "{}",
        serde_json::to_string(&output).context("serialize Unzen zk-gas schedule")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, str::FromStr};

    use alloy::primitives::Address;

    use super::*;

    #[test]
    fn exported_entries_match_the_pinned_schedule_by_identifier() {
        let output = build_schedule_export().unwrap();

        assert!(!output.opcodes.is_empty());
        assert!(!output.precompiles.is_empty());
        let exported_opcodes = output
            .opcodes
            .iter()
            .map(|entry| {
                assert!(
                    entry.opcode.starts_with("0x"),
                    "opcode identifier must start with 0x: {}",
                    entry.opcode
                );
                (
                    usize::from_str_radix(entry.opcode.trim_start_matches("0x"), 16).unwrap(),
                    entry.multiplier,
                )
            })
            .collect::<Vec<_>>();
        let expected_opcodes = UNZEN_ZK_GAS_SCHEDULE
            .opcode_multipliers
            .iter()
            .enumerate()
            .filter(|(_, multiplier)| **multiplier != FAILSAFE_MULTIPLIER)
            .map(|(opcode, &multiplier)| (opcode, multiplier))
            .collect::<Vec<_>>();
        assert_eq!(exported_opcodes, expected_opcodes);

        let exported_precompiles = output
            .precompiles
            .iter()
            .map(|entry| {
                assert!(
                    entry.address.starts_with("0x"),
                    "precompile address must start with 0x: {}",
                    entry.address
                );
                assert_eq!(
                    entry.address.len(),
                    42,
                    "precompile address must be a 20-byte hex string: {}",
                    entry.address
                );
                (Address::from_str(&entry.address).unwrap(), entry.multiplier)
            })
            .collect::<Vec<_>>();
        let expected_precompiles = UNZEN_ZK_GAS_SCHEDULE
            .precompile_multipliers
            .iter()
            .map(|(address, multiplier)| (*address, *multiplier))
            .collect::<Vec<_>>();
        assert_eq!(exported_precompiles, expected_precompiles);
    }

    #[test]
    fn exporter_alethia_rev_matches_workspace() {
        let root = crate::util::repo_root();
        let workspace: toml::Value =
            toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
        let xtask: toml::Value =
            toml::from_str(&fs::read_to_string(root.join("xtask/Cargo.toml")).unwrap()).unwrap();
        let workspace_rev = workspace["workspace"]["dependencies"]["alethia-reth-chainspec"]["rev"]
            .as_str()
            .expect("workspace must pin an alethia-reth revision");
        let xtask_rev = xtask["dependencies"]["alethia-reth-evm"]["rev"]
            .as_str()
            .expect("xtask must pin an alethia-reth revision");

        assert_eq!(
            xtask_rev, workspace_rev,
            "xtask alethia-reth rev must match workspace"
        );
    }
}
