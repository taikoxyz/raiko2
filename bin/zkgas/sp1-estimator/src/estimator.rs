use std::sync::Arc;

use anyhow::Result;
use sp1_core_executor::{
    DEFAULT_GAS_TRACE_CHUNK_SLOTS, DEFAULT_MEMORY_LIMIT, DEFAULT_TRACE_CHUNK_SLOTS,
    ELEMENT_THRESHOLD, GAS_TRACE_CHUNK_THRESHOLD, GasEstimatingVMEnum, HEIGHT_THRESHOLD,
    MINIMAL_TRACE_CHUNK_THRESHOLD, Program, SP1CoreOpts, ShardingThreshold,
};
use sp1_core_executor_runner::MinimalExecutorRunner;
use sp1_sdk::{ExecutionReport, SP1Stdin};

pub(crate) fn canonical_sp1_core_opts() -> SP1CoreOpts {
    canonicalize_sp1_core_opts(SP1CoreOpts::default())
}

pub(crate) fn canonicalize_sp1_core_opts(mut opts: SP1CoreOpts) -> SP1CoreOpts {
    opts.minimal_trace_chunk_threshold = MINIMAL_TRACE_CHUNK_THRESHOLD;
    opts.gas_trace_chunk_threshold = GAS_TRACE_CHUNK_THRESHOLD;
    opts.trace_chunk_slots = DEFAULT_TRACE_CHUNK_SLOTS;
    opts.gas_trace_chunk_slots = DEFAULT_GAS_TRACE_CHUNK_SLOTS;
    opts.memory_limit = DEFAULT_MEMORY_LIMIT;
    opts.shard_size = 1 << 24;
    opts.sharding_threshold = ShardingThreshold {
        element_threshold: ELEMENT_THRESHOLD,
        height_threshold: HEIGHT_THRESHOLD,
    };
    opts
}

pub(crate) fn execute_sp1_gas_estimator_with_opts(
    program: Arc<Program>,
    stdin: SP1Stdin,
    opts: SP1CoreOpts,
    resource_output: Option<std::fs::File>,
) -> Result<(sp1_sdk::SP1PublicValues, ExecutionReport)> {
    let before = resource_output
        .as_ref()
        .map(|_| crate::resources::baseline())
        .transpose()?;
    let mut runner = MinimalExecutorRunner::new(
        program.clone(),
        false,
        Some(opts.gas_trace_chunk_threshold),
        opts.memory_limit,
        opts.gas_trace_chunk_slots,
    );
    if let (Some(output), Some(before)) = (resource_output, before) {
        crate::resources::publish(output, &before)?;
    }
    for buffer in &stdin.buffer {
        runner.with_input(buffer);
    }

    let mut report = ExecutionReport::default();
    while let Some(chunk) = runner
        .try_execute_chunk()
        .map_err(|err| anyhow::anyhow!("execute minimal SP1 guest chunk: {err:?}"))?
    {
        let mut vm = GasEstimatingVMEnum::new(&chunk, program.clone(), [0u32; 4], opts.clone());
        report += vm
            .execute()
            .map_err(|err| anyhow::anyhow!("estimate SP1 guest gas: {err:?}"))?;
    }
    let public_values = sp1_sdk::SP1PublicValues::from(runner.public_values_stream());
    Ok((public_values, report))
}
