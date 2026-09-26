use alethia_reth_evm::zk_gas::{schedule::FAILSAFE_MULTIPLIER, unzen::UNZEN_ZK_GAS_SCHEDULE};
use anyhow::{Context, Result, ensure};
use raiko2_primitives::chain_spec::TaikoFork;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize, Clone)]
struct ScheduleExport {
    block_limit: u64,
    failsafe_multiplier: u16,
    opcodes: Vec<OpcodeMultiplier>,
    precompile_fallback_multiplier: u16,
    precompiles: Vec<PrecompileMultiplier>,
    schedule_sha256: String,
    spawn_estimates: SpawnEstimates,
    tx_intrinsic_zk_gas: u64,
    version_identity: VersionIdentity,
}

#[derive(Serialize, Clone)]
struct VersionIdentity {
    taiko_fork: TaikoFork,
    production_schedule: &'static str,
    ethereum_upgrade: &'static str,
    revm_spec_id: String,
}

#[derive(Serialize, Clone)]
struct OpcodeMultiplier {
    explicit: bool,
    opcode: String,
    multiplier: u16,
}

#[derive(Serialize, Clone)]
struct PrecompileMultiplier {
    explicit: bool,
    address: String,
    multiplier: u16,
}

#[derive(Serialize, Clone)]
struct SpawnEstimates {
    call: u64,
    callcode: u64,
    create: u64,
    create2: u64,
    delegatecall: u64,
    staticcall: u64,
}

#[derive(Serialize)]
struct ScheduleHashInput<'a> {
    block_limit: u64,
    failsafe_multiplier: u16,
    opcodes: &'a [OpcodeMultiplier],
    precompile_fallback_multiplier: u16,
    precompiles: &'a [PrecompileMultiplier],
    spawn_estimates: &'a SpawnEstimates,
    tx_intrinsic_zk_gas: u64,
}

fn build_schedule_export() -> Result<ScheduleExport> {
    let opcodes: Vec<OpcodeMultiplier> = UNZEN_ZK_GAS_SCHEDULE
        .opcode_multipliers
        .iter()
        .enumerate()
        .map(|(opcode, &multiplier)| OpcodeMultiplier {
            explicit: multiplier != FAILSAFE_MULTIPLIER,
            opcode: format!("0x{opcode:02x}"),
            multiplier,
        })
        .collect();
    let precompiles: Vec<PrecompileMultiplier> = UNZEN_ZK_GAS_SCHEDULE
        .precompile_multipliers
        .iter()
        .map(|(address, multiplier)| PrecompileMultiplier {
            explicit: true,
            address: format!("{address:#x}"),
            multiplier: *multiplier,
        })
        .collect();
    ensure!(!opcodes.is_empty(), "Unzen opcode schedule is empty");
    ensure!(
        !precompiles.is_empty(),
        "Unzen precompile schedule is empty"
    );
    let spawn_estimates = SpawnEstimates {
        call: UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.call,
        callcode: UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.callcode,
        create: UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.create,
        create2: UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.create2,
        delegatecall: UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.delegatecall,
        staticcall: UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.staticcall,
    };
    let mut output = ScheduleExport {
        block_limit: UNZEN_ZK_GAS_SCHEDULE.block_limit,
        failsafe_multiplier: FAILSAFE_MULTIPLIER,
        opcodes,
        precompile_fallback_multiplier: FAILSAFE_MULTIPLIER,
        precompiles,
        schedule_sha256: String::new(),
        spawn_estimates,
        tx_intrinsic_zk_gas: UNZEN_ZK_GAS_SCHEDULE.tx_intrinsic_zk_gas,
        version_identity: VersionIdentity {
            taiko_fork: TaikoFork::Unzen,
            production_schedule: "UNZEN_ZK_GAS_SCHEDULE",
            ethereum_upgrade: "Fusaka",
            revm_spec_id: format!("{:?}", TaikoFork::Unzen.revm_spec_id()),
        },
    };
    output.schedule_sha256 = schedule_sha256(&output)?;
    Ok(output)
}

fn schedule_sha256(output: &ScheduleExport) -> Result<String> {
    let canonical = serde_json::to_vec(&ScheduleHashInput {
        block_limit: output.block_limit,
        failsafe_multiplier: output.failsafe_multiplier,
        opcodes: &output.opcodes,
        precompile_fallback_multiplier: output.precompile_fallback_multiplier,
        precompiles: &output.precompiles,
        spawn_estimates: &output.spawn_estimates,
        tx_intrinsic_zk_gas: output.tx_intrinsic_zk_gas,
    })
    .context("serialize complete Unzen schedule identity")?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
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
            .map(|(opcode, &multiplier)| (opcode, multiplier))
            .collect::<Vec<_>>();
        assert_eq!(exported_opcodes, expected_opcodes);
        assert!(output.opcodes.iter().all(|row| {
            let opcode = usize::from_str_radix(row.opcode.trim_start_matches("0x"), 16).unwrap();
            row.explicit
                == (UNZEN_ZK_GAS_SCHEDULE.opcode_multipliers[opcode] != FAILSAFE_MULTIPLIER)
        }));

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
        assert!(output.precompiles.iter().all(|row| row.explicit));
    }

    #[test]
    fn export_preserves_complete_schedule_identity_and_hashes_all_fields() {
        let output = build_schedule_export().unwrap();

        assert_eq!(output.opcodes.len(), 256);
        for (opcode, row) in output.opcodes.iter().enumerate() {
            assert_eq!(row.opcode, format!("0x{opcode:02x}"));
            assert_eq!(
                row.explicit,
                UNZEN_ZK_GAS_SCHEDULE.opcode_multipliers[opcode] != FAILSAFE_MULTIPLIER
            );
            assert_eq!(
                row.multiplier,
                UNZEN_ZK_GAS_SCHEDULE.opcode_multipliers[opcode]
            );
        }
        assert_eq!(output.failsafe_multiplier, FAILSAFE_MULTIPLIER);
        assert_eq!(output.precompile_fallback_multiplier, FAILSAFE_MULTIPLIER);
        assert_eq!(output.block_limit, UNZEN_ZK_GAS_SCHEDULE.block_limit);
        assert_eq!(
            output.tx_intrinsic_zk_gas,
            UNZEN_ZK_GAS_SCHEDULE.tx_intrinsic_zk_gas
        );
        assert_eq!(
            output.spawn_estimates.call,
            UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.call
        );
        assert_eq!(
            output.spawn_estimates.create2,
            UNZEN_ZK_GAS_SCHEDULE.spawn_estimates.create2
        );

        let canonical = serde_json::to_vec(&output).unwrap();
        let mut changed = build_schedule_export().unwrap();
        changed.block_limit += 1;
        assert_ne!(canonical, serde_json::to_vec(&changed).unwrap());
        assert_ne!(output.schedule_sha256, schedule_sha256(&changed).unwrap());
    }

    #[test]
    fn export_binds_unzen_fusaka_to_the_shared_osaka_runtime_mapping() {
        let output = build_schedule_export().unwrap();
        let exported = serde_json::to_value(output).unwrap();

        assert_eq!(
            exported["version_identity"],
            serde_json::json!({
                "taiko_fork": "Unzen",
                "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
                "ethereum_upgrade": "Fusaka",
                "revm_spec_id": "OSAKA",
            })
        );
        assert_eq!(
            exported["version_identity"]["revm_spec_id"],
            format!(
                "{:?}",
                raiko2_primitives::chain_spec::TaikoFork::Unzen.revm_spec_id()
            )
        );
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
