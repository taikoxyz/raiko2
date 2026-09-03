use alethia_reth_evm::zk_gas::{schedule::FAILSAFE_MULTIPLIER, unzen::UNZEN_ZK_GAS_SCHEDULE};
use anyhow::{Context, Result};
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

pub(crate) fn run() -> Result<()> {
    let opcodes = UNZEN_ZK_GAS_SCHEDULE
        .opcode_multipliers
        .iter()
        .enumerate()
        .filter(|(_, multiplier)| **multiplier != FAILSAFE_MULTIPLIER)
        .map(|(opcode, &multiplier)| OpcodeMultiplier {
            opcode: format!("0x{opcode:02x}"),
            multiplier,
        })
        .collect();
    let precompiles = UNZEN_ZK_GAS_SCHEDULE
        .precompile_multipliers
        .iter()
        .map(|(address, multiplier)| PrecompileMultiplier {
            address: format!("{address:#x}"),
            multiplier: *multiplier,
        })
        .collect();
    let output = ScheduleExport {
        opcodes,
        precompiles,
    };
    println!(
        "{}",
        serde_json::to_string(&output).context("serialize Unzen zk-gas schedule")?
    );
    Ok(())
}
