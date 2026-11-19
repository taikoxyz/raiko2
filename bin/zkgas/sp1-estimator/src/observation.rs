//! Optional bounded historical register observations; canonical gas remains independent.
use std::{collections::HashSet, sync::Arc};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sp1_core_executor::{
    CompressedMemory, CompressedPages, CoreVM, ExecutionMode, Opcode, Program, SP1CoreOpts,
    SplicingVMEnum, TraceChunkRaw,
};

pub(crate) const PLAN_LIMIT: u64 = 64 * 1024;
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    elf_sha256: String,
    instruction_limit: u64,
    watches: Vec<Watch>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Watch {
    label: String,
    pc: String,
    registers: Vec<u8>,
    sample_limit: u32,
    #[serde(default)]
    skip_hits: u64,
}

#[derive(Serialize)]
struct WatchReport {
    #[serde(flatten)]
    watch: Watch,
    total_hits: u64,
    samples: Vec<Sample>,
    samples_truncated: bool,
}

#[derive(Serialize)]
struct Sample {
    hit: u64,
    instruction: u64,
    pc: String,
    clock_before: u64,
    clock_after: u64,
    pc_after: String,
    registers_before: Vec<u64>,
    registers_after: Vec<u64>,
}

#[derive(Serialize)]
pub(crate) struct ObservationReport {
    schema_version: u32,
    plan_sha256: String,
    elf_sha256: String,
    instruction_limit: u64,
    processed_instructions: u64,
    complete: bool,
    samples_complete: bool,
    status: &'static str,
    error: Option<String>,
    completed_chunks: u64,
    watches: Vec<WatchReport>,
}

#[derive(Clone, PartialEq, Eq)]
struct Boundary {
    pc: u64,
    clock: u64,
    registers: [u64; 32],
    halted: bool,
}
impl Boundary {
    fn from_core<M: ExecutionMode>(core: &CoreVM<'_, M>) -> Self {
        Self {
            pc: core.pc(),
            clock: core.clk(),
            registers: core.registers().map(|r| r.value),
            halted: core.is_done(),
        }
    }
}

pub(crate) struct Observation {
    report: ObservationReport,
    // Sorted small index preserves declared report order and avoids scanning all watches.
    pcs: Vec<(u64, usize)>,
    boundary: Option<Boundary>,
    active: bool,
}

impl Observation {
    pub(crate) fn parse(bytes: &[u8], elf: &str, program: &Program) -> Result<Self> {
        ensure!(
            bytes.len() as u64 <= PLAN_LIMIT,
            "observation plan exceeds 64KiB"
        );
        let plan: Plan = serde_json::from_slice(bytes).context("parse observation plan")?;
        ensure!(
            plan.schema_version == 1,
            "unsupported observation plan schema"
        );
        ensure!(
            plan.elf_sha256 == elf,
            "observation plan ELF identity mismatch"
        );
        ensure!(
            (1..=1_000_000_000).contains(&plan.instruction_limit),
            "invalid observation instruction limit"
        );
        ensure!(
            (1..=64).contains(&plan.watches.len()),
            "invalid observation watch count"
        );
        let mut labels = HashSet::new();
        let mut pcs = Vec::new();
        let mut samples = 0usize;
        let mut output_bound = 16 * 1024usize;
        for (index, watch) in plan.watches.iter().enumerate() {
            let label = watch.label.as_bytes();
            ensure!(
                (1..=64).contains(&label.len())
                    && (label[0].is_ascii_alphabetic() || label[0] == b'_')
                    && label
                        .iter()
                        .all(|b| b.is_ascii_alphanumeric() || *b == b'_'),
                "invalid observation label"
            );
            ensure!(labels.insert(&watch.label), "duplicate observation label");
            let pc = watch.pc.as_bytes();
            ensure!(
                pc.len() == 18
                    && pc.starts_with(b"0x")
                    && pc[2..]
                        .iter()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)),
                "invalid observation PC encoding"
            );
            let pc = u64::from_str_radix(&watch.pc[2..], 16)?;
            // Program::fetch divides/truncates and subtracts unchecked. Validate first.
            let offset = pc
                .checked_sub(program.pc_base)
                .context("observation PC below program")?;
            ensure!(
                offset % 4 == 0 && offset / 4 < program.instructions.len() as u64,
                "observation PC is not an executable instruction"
            );
            ensure!(
                !matches!(
                    program.instructions[(offset / 4) as usize].opcode,
                    Opcode::UNIMP | Opcode::EBREAK
                ),
                "observation PC has no executable instruction"
            );
            ensure!(
                !pcs.iter().any(|(known, _)| *known == pc),
                "duplicate observation PC"
            );
            pcs.push((pc, index));
            ensure!(
                !watch.registers.is_empty() && watch.registers.len() <= 32,
                "invalid observation registers"
            );
            let mut registers = HashSet::new();
            ensure!(
                watch
                    .registers
                    .iter()
                    .all(|r| *r < 32 && registers.insert(*r)),
                "duplicate or invalid observation register"
            );
            ensure!(
                watch.sample_limit <= 1024,
                "invalid observation sample limit"
            );
            samples += watch.sample_limit as usize;
            ensure!(
                samples <= 2048,
                "observation aggregate sample limit exceeds 2048"
            );
            // JSON u64s use at most20 bytes. Metadata/keys/clocks/PCs <=512/sample,
            // two register arrays <=42 bytes/register. Additional watch metadata <=1024.
            output_bound += 1024 + watch.sample_limit as usize * (512 + 42 * watch.registers.len());
        }
        ensure!(
            output_bound <= OUTPUT_LIMIT,
            "observation conservative output bound exceeds 4MiB"
        );
        pcs.sort_unstable();
        Ok(Self {
            report: ObservationReport {
                schema_version: 1,
                plan_sha256: crate::io::sha256(bytes),
                elf_sha256: plan.elf_sha256,
                instruction_limit: plan.instruction_limit,
                processed_instructions: 0,
                complete: false,
                samples_complete: true,
                status: "running",
                error: None,
                completed_chunks: 0,
                watches: plan
                    .watches
                    .into_iter()
                    .map(|watch| WatchReport {
                        watch,
                        total_hits: 0,
                        samples: Vec::new(),
                        samples_truncated: false,
                    })
                    .collect(),
            },
            pcs,
            boundary: None,
            active: true,
        })
    }

    fn before_step(
        &mut self,
        pc: u64,
        clock: u64,
        registers: impl Fn(u8) -> u64,
    ) -> Result<Option<(usize, Sample)>> {
        let Ok(index) = self.pcs.binary_search_by_key(&pc, |(pc, _)| *pc) else {
            return Ok(None);
        };
        let index = self.pcs[index].1;
        let watch = &mut self.report.watches[index];
        watch.total_hits = watch
            .total_hits
            .checked_add(1)
            .context("observation hit count overflow")?;
        if watch.total_hits <= watch.watch.skip_hits {
            return Ok(None);
        }
        if watch.samples.len() >= watch.watch.sample_limit as usize {
            watch.samples_truncated = true;
            self.report.samples_complete = false;
            return Ok(None);
        }
        Ok(Some((
            index,
            Sample {
                hit: watch.total_hits,
                instruction: self
                    .report
                    .processed_instructions
                    .checked_add(1)
                    .context("observation instruction count overflow")?,
                pc: watch.watch.pc.clone(),
                clock_before: clock,
                clock_after: 0,
                pc_after: String::new(),
                registers_before: watch
                    .watch
                    .registers
                    .iter()
                    .map(|r| registers(*r))
                    .collect(),
                registers_after: Vec::new(),
            },
        )))
    }

    fn after_step(
        &mut self,
        pending: Option<(usize, Sample)>,
        pc: u64,
        clock: u64,
        registers: impl Fn(u8) -> u64,
    ) -> Result<()> {
        if let Some((index, mut sample)) = pending {
            let watch = &mut self.report.watches[index];
            sample.clock_after = clock;
            sample.pc_after = format!("0x{pc:016x}");
            sample.registers_after = watch
                .watch
                .registers
                .iter()
                .map(|r| registers(*r))
                .collect();
            watch.samples.push(sample);
        }
        self.report.processed_instructions = self
            .report
            .processed_instructions
            .checked_add(1)
            .context("observation instruction count overflow")?;
        Ok(())
    }

    fn fail(&mut self, status: &'static str, error: String) {
        self.active = false;
        self.report.status = status;
        self.report.samples_complete = false;
        self.report.error = Some(error.chars().take(512).collect());
    }

    fn can_step(&mut self, done: bool, trace_end: bool) -> bool {
        if done || trace_end {
            return false;
        }
        if self.report.processed_instructions == self.report.instruction_limit {
            self.active = false;
            self.report.status = "instruction_limit";
            self.report.samples_complete = false;
            return false;
        }
        true
    }
    fn check_start(&self, start: &Boundary) -> Result<()> {
        if let Some(last) = &self.boundary {
            ensure!(
                !last.halted && last == start,
                "observation chunk boundary discontinuity"
            );
        }
        Ok(())
    }
    fn complete_chunk(&mut self, end: Boundary, unread: usize) -> Result<()> {
        ensure!(
            unread == 0,
            "observation trace ended with unread oracle entries"
        );
        self.boundary = Some(end);
        self.report.completed_chunks = self
            .report
            .completed_chunks
            .checked_add(1)
            .context("observation chunk count overflow")?;
        Ok(())
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }

    pub(crate) fn consume_chunk(
        &mut self,
        chunk: &TraceChunkRaw,
        program: Arc<Program>,
        opts: SP1CoreOpts,
    ) {
        if !self.active {
            return;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Unique-address/page accounting belongs only to this held chunk; no full
            // ExecutionRecord or earlier trace is retained. External PSS guards remain required.
            let mut addresses = CompressedMemory::new();
            let mut pages = CompressedPages::new();
            let mut vm =
                SplicingVMEnum::new(chunk, program, &mut addresses, &mut pages, [0; 4], opts);
            self.consume(&mut vm)
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.fail("error", error.to_string()),
            Err(_) => self.fail("error", "public SP1 observation replay panicked".into()),
        }
    }

    fn consume(&mut self, vm: &mut SplicingVMEnum<'_>) -> Result<()> {
        // Both arms call the public typed step. No private instruction dispatch is copied.
        macro_rules! replay {
            ($vm:expr) => {{
                let vm = $vm;
                (|| -> Result<()> {
                    let start = Boundary::from_core(&vm.core);
                    self.check_start(&start)?;
                    loop {
                        // An exhausted trace must never execute another instruction/read.
                        if !self.can_step(vm.core.is_done(), vm.core.is_trace_end()) {
                            break;
                        }
                        ensure!(
                            vm.core.clk() < vm.core.clk_end,
                            "observation clock passed trace end"
                        );
                        let pending = self.before_step(vm.core.pc(), vm.core.clk(), |r| {
                            vm.core.registers()[r as usize].value
                        })?;
                        vm.execute_instruction().map_err(|e| {
                            anyhow::anyhow!("public SP1 observation step failed: {e:?}")
                        })?;
                        self.after_step(pending, vm.core.pc(), vm.core.clk(), |r| {
                            vm.core.registers()[r as usize].value
                        })?;
                    }
                    if self.active {
                        self.complete_chunk(
                            Boundary::from_core(&vm.core),
                            vm.core.mem_reads.len(),
                        )?;
                    }
                    Ok(())
                })()
            }};
        }
        match vm {
            SplicingVMEnum::Supervisor(vm) => replay!(vm),
            SplicingVMEnum::User(vm) => replay!(vm),
        }
    }

    pub(crate) fn finish(mut self) -> Result<serde_json::Value> {
        if self.active {
            if self.boundary.as_ref().is_some_and(|b| b.halted) {
                self.report.complete = true;
                self.report.status = "complete";
            } else {
                self.fail(
                    "error",
                    "canonical execution ended without an observed halt".into(),
                );
            }
        }
        let value = serde_json::to_value(&self.report)?;
        ensure!(
            serde_json::to_vec(&value)?.len() <= OUTPUT_LIMIT,
            "observation encoded output exceeds 4MiB"
        );
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp1_core_executor::{Instruction, Opcode};
    fn program() -> Program {
        Program::new(
            vec![Instruction::new(Opcode::ADD, 1, 1, 1, false, true); 3],
            0x1000,
            0x1000,
        )
    }
    fn plan() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"elf_sha256":"a".repeat(64),"instruction_limit":100,
            "watches":[{"label":"load_seed","pc":"0x0000000000001000","registers":[0,1,31],"sample_limit":2,"skip_hits":1}]})
    }
    fn parse(value: &serde_json::Value) -> Result<Observation> {
        Observation::parse(
            &serde_json::to_vec(value).unwrap(),
            &"a".repeat(64),
            &program(),
        )
    }
    #[test]
    fn valid_plan_accepts_optional_skip_and_binds_elf() {
        assert!(parse(&plan()).is_ok());
        let mut value = plan();
        value["watches"][0]
            .as_object_mut()
            .unwrap()
            .remove("skip_hits");
        assert!(parse(&value).is_ok());
        assert!(
            Observation::parse(
                &serde_json::to_vec(&value).unwrap(),
                &"b".repeat(64),
                &program()
            )
            .is_err()
        );
    }
    #[test]
    fn strict_fields_domains_and_instruction_addresses() {
        let invalid = [
            ("schema_version", serde_json::json!(true)),
            ("schema_version", serde_json::json!(2)),
            ("instruction_limit", serde_json::json!(0)),
            ("instruction_limit", serde_json::json!(1_000_000_001u64)),
            ("instruction_limit", serde_json::json!(-1)),
            ("unknown", serde_json::json!(0)),
            ("watches", serde_json::json!([])),
        ];
        for (key, val) in invalid {
            let mut value = plan();
            value[key] = val;
            assert!(parse(&value).is_err(), "{key}");
        }
        let invalid = [
            ("pc", serde_json::json!("0x0000000000000ffc")),
            ("pc", serde_json::json!("0x0000000000001001")),
            ("pc", serde_json::json!("0x000000000000100c")),
            ("pc", serde_json::json!("0x000000000000100A")),
            ("pc", serde_json::json!(4096)),
            ("pc", serde_json::json!("0x1000")),
            ("label", serde_json::json!("bad-label")),
            ("label", serde_json::json!("9bad")),
            ("label", serde_json::json!("a".repeat(65))),
            ("registers", serde_json::json!([true])),
            ("registers", serde_json::json!([1, 1])),
            ("registers", serde_json::json!([32])),
            ("registers", serde_json::json!([])),
            ("sample_limit", serde_json::json!(1025)),
            ("skip_hits", serde_json::json!(-1)),
            ("unknown", serde_json::json!(0)),
        ];
        for (key, val) in invalid {
            let mut value = plan();
            value["watches"][0][key] = val;
            assert!(parse(&value).is_err(), "{key}");
        }
    }
    #[test]
    fn duplicate_fields_and_watches_are_rejected() {
        let bytes = serde_json::to_string(&plan()).unwrap();
        let duplicate = bytes.replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        );
        assert!(Observation::parse(duplicate.as_bytes(), &"a".repeat(64), &program()).is_err());
        let duplicate = bytes.replacen(
            "\"sample_limit\":2",
            "\"sample_limit\":2,\"sample_limit\":2",
            1,
        );
        assert!(Observation::parse(duplicate.as_bytes(), &"a".repeat(64), &program()).is_err());
        let mut value = plan();
        let mut watch = value["watches"][0].clone();
        watch["label"] = serde_json::json!("other");
        value["watches"].as_array_mut().unwrap().push(watch);
        assert!(parse(&value).is_err());
        value["watches"][1]["pc"] = serde_json::json!("0x0000000000001004");
        value["watches"][1]["label"] = serde_json::json!("load_seed");
        assert!(parse(&value).is_err());
    }
    #[test]
    fn plan_byte_and_aggregate_sample_bounds() {
        assert!(Observation::parse(&vec![b' '; 65537], &"a".repeat(64), &program()).is_err());
        let mut value = plan();
        value["watches"][0]["sample_limit"] = serde_json::json!(1024);
        for (label, pc) in [
            ("two", "0x0000000000001004"),
            ("three", "0x0000000000001008"),
        ] {
            let mut watch = value["watches"][0].clone();
            watch["label"] = serde_json::json!(label);
            watch["pc"] = serde_json::json!(pc);
            value["watches"].as_array_mut().unwrap().push(watch);
        }
        assert!(parse(&value).is_err());
    }
    #[test]
    fn sampling_retains_selected_pre_post_registers_after_skips_and_counts_all_hits() {
        let mut obs = parse(&plan()).unwrap();
        for hit in 1..=5u64 {
            let pending = obs
                .before_step(0x1000, hit * 8, |r| hit * 100 + u64::from(r))
                .unwrap();
            obs.after_step(pending, 0x1004, hit * 8 + 8, |r| hit * 1000 + u64::from(r))
                .unwrap();
        }
        let watch = &obs.report.watches[0];
        assert_eq!(watch.total_hits, 5);
        assert_eq!(watch.samples.len(), 2);
        assert_eq!(watch.samples[0].hit, 2);
        assert_eq!(watch.samples[1].hit, 3);
        assert_eq!(watch.samples[0].registers_before, vec![200, 201, 231]);
        assert_eq!(watch.samples[0].registers_after, vec![2000, 2001, 2031]);
        assert_eq!(watch.samples[0].clock_before, 16);
        assert_eq!(watch.samples[0].clock_after, 24);
        assert_eq!(watch.samples[0].pc, "0x0000000000001000");
        assert_eq!(watch.samples[0].pc_after, "0x0000000000001004");
        assert_eq!(obs.report.processed_instructions, 5);
        let pending = obs
            .before_step(0x1008, 48, |_| {
                panic!("unwatched registers must not be read")
            })
            .unwrap();
        assert!(pending.is_none());
        obs.after_step(pending, 0x100c, 56, |_| {
            panic!("unwatched registers must not be read")
        })
        .unwrap();
        assert_eq!(obs.report.processed_instructions, 6);
    }
    #[test]
    fn terminal_traces_never_step_and_instruction_limit_is_incomplete() {
        let mut obs = parse(&plan()).unwrap();
        assert!(!obs.can_step(false, true));
        assert!(!obs.can_step(true, false));
        assert!(obs.can_step(false, false));
        obs.report.processed_instructions = 100;
        assert!(!obs.can_step(false, false));
        let report = obs.finish().unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["status"], "instruction_limit");
    }
    fn boundary() -> Boundary {
        Boundary {
            pc: 0x1004,
            clock: 9,
            registers: [7; 32],
            halted: false,
        }
    }
    #[test]
    fn chunk_boundaries_require_identical_pc_clock_registers_and_empty_cursor() {
        let mut obs = parse(&plan()).unwrap();
        assert!(obs.complete_chunk(boundary(), 1).is_err());
        obs.complete_chunk(boundary(), 0).unwrap();
        assert_eq!(obs.report.completed_chunks, 1);
        assert!(obs.check_start(&boundary()).is_ok());
        for index in 0..3 {
            let mut next = boundary();
            match index {
                0 => next.pc += 4,
                1 => next.clock += 8,
                _ => next.registers[31] += 1,
            };
            assert!(obs.check_start(&next).is_err());
        }
        let mut halt = boundary();
        halt.halted = true;
        obs.complete_chunk(halt.clone(), 0).unwrap();
        assert!(obs.check_start(&halt).is_err());
        let report = obs.finish().unwrap();
        assert_eq!(report["complete"], true);
        assert_eq!(report["status"], "complete");
    }
    #[test]
    fn diagnostic_failure_and_missing_halt_remain_explicit_and_bounded() {
        let mut obs = parse(&plan()).unwrap();
        obs.fail("error", "failure".repeat(1000));
        let report = obs.finish().unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["status"], "error");
        assert!(report["error"].as_str().unwrap().len() <= 512);
        let report = parse(&plan()).unwrap().finish().unwrap();
        assert_eq!(report["complete"], false);
        assert!(report["error"].as_str().unwrap().contains("halt"));
    }
    #[test]
    fn counters_fail_checked_and_skips_do_not_overflow() {
        let mut value = plan();
        value["watches"][0]["skip_hits"] = serde_json::json!(u64::MAX);
        let mut obs = parse(&value).unwrap();
        assert!(
            obs.before_step(0x1000, 1, |_| panic!("skip must not read registers"))
                .unwrap()
                .is_none()
        );
        obs.report.watches[0].total_hits = u64::MAX;
        assert!(obs.before_step(0x1000, 1, |_| 0).is_err());
        obs.report.processed_instructions = u64::MAX;
        assert!(obs.after_step(None, 0, 0, |_| 0).is_err());
    }

    #[test]
    fn zero_sample_limit_counts_hits_without_retaining_events() {
        let mut value = plan();
        value["watches"][0]["sample_limit"] = serde_json::json!(0);
        value["watches"][0]["skip_hits"] = serde_json::json!(0);
        let mut obs = parse(&value).unwrap();
        for _ in 0..3 {
            assert!(
                obs.before_step(0x1000, 1, |_| panic!(
                    "count-only watch must not read registers"
                ))
                .unwrap()
                .is_none()
            );
            obs.after_step(None, 0x1004, 9, |_| 0).unwrap();
        }
        assert_eq!(obs.report.watches[0].total_hits, 3);
        assert!(obs.report.watches[0].samples.is_empty());
        assert!(obs.report.watches[0].samples_truncated);
        assert!(!obs.report.samples_complete);
        let mut end = boundary();
        end.halted = true;
        obs.complete_chunk(end, 0).unwrap();
        let report = obs.finish().unwrap();
        assert_eq!(report["complete"], true);
        assert_eq!(report["samples_complete"], false);
    }
    #[test]
    fn watch_count_and_nonexecuting_pc_are_rejected() {
        let instructions = vec![Instruction::new(Opcode::ADD, 1, 1, 1, false, true); 65];
        let program = Program::new(instructions, 0x1000, 0x1000);
        let mut value = plan();
        let watches=(0..65).map(|index|serde_json::json!({"label":format!("watch_{index}"),"pc":format!("0x{:016x}",0x1000+index*4),"registers":[1],"sample_limit":0})).collect::<Vec<_>>();
        value["watches"] = serde_json::json!(watches);
        assert!(
            Observation::parse(
                &serde_json::to_vec(&value).unwrap(),
                &"a".repeat(64),
                &program
            )
            .is_err()
        );
        value["watches"].as_array_mut().unwrap().pop();
        assert!(
            Observation::parse(
                &serde_json::to_vec(&value).unwrap(),
                &"a".repeat(64),
                &program
            )
            .is_ok()
        );
        let mut program = program;
        program.instructions[0].opcode = Opcode::UNIMP;
        assert!(
            Observation::parse(
                &serde_json::to_vec(&value).unwrap(),
                &"a".repeat(64),
                &program
            )
            .is_err()
        );
    }
    #[test]
    fn maximum_output_bound_contains_full_register_samples() {
        let mut value = plan();
        value["watches"][0]["registers"] = serde_json::json!((0..32).collect::<Vec<_>>());
        value["watches"][0]["sample_limit"] = serde_json::json!(1024);
        value["watches"][0]["skip_hits"] = serde_json::json!(0);
        let mut second = value["watches"][0].clone();
        second["label"] = serde_json::json!("second");
        second["pc"] = serde_json::json!("0x0000000000001004");
        value["watches"].as_array_mut().unwrap().push(second);
        let mut obs = parse(&value).unwrap();
        for _ in 0..1025 {
            for pc in [0x1000, 0x1004] {
                let pending = obs.before_step(pc, u64::MAX, |_| u64::MAX).unwrap();
                obs.after_step(pending, u64::MAX, u64::MAX, |_| u64::MAX)
                    .unwrap();
            }
        }
        let mut end = boundary();
        end.halted = true;
        obs.complete_chunk(end, 0).unwrap();
        let report = obs.finish().unwrap();
        assert!(serde_json::to_vec(&report).unwrap().len() < OUTPUT_LIMIT);
        assert_eq!(report["complete"], true);
        assert_eq!(report["samples_complete"], false);
        assert_eq!(
            report["watches"][0]["samples"].as_array().unwrap().len(),
            1024
        );
        assert_eq!(report["watches"][0]["total_hits"], 1025);
        assert_eq!(report["watches"][0]["samples_truncated"], true);
    }
}
