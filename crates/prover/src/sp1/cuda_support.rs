//! CUDA-independent parts of local SP1 GPU proving.
//!
//! Everything here is plain logic with no `sp1-cuda` dependency, so it is compiled and tested in
//! every SP1 build (including CI's CPU lanes), even though only `cuda` builds use most of it.
#![cfg_attr(not(feature = "cuda"), allow(dead_code))]

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use sp1_sdk::{Elf, SP1Stdin, blocking::Prover};

use crate::sp1_config::MAX_CUDA_TIMEOUT_SECS;

/// Server-reported errors on this many distinct inputs in a row retire a GPU client.
pub(super) const MAX_DISTINCT_FAILING_INPUTS: usize = 3;
/// How many timed-out inputs are remembered so their retries skip the GPU.
const TIMED_OUT_INPUTS_CAPACITY: usize = 256;

/// Executes `elf` on the CPU and fails if it needs more than `cycle_limit` cycles.
///
/// This is how GPU proofs enforce the SP1 cycle limit: `sp1-gpu-server` never receives it.
/// Keep the default execution context. In SP1 6.8.1 the only `max_cycles` check runs in the gas
/// accounting, so `calculate_gas(false)` silently disables the limit; deferred-proof verification
/// must stay on so invalid aggregation sub-proofs fail here.
pub(super) fn execute_within_cycle_limit<P: Prover>(
    prover: &P,
    elf: Elf,
    stdin: SP1Stdin,
    cycle_limit: u64,
) -> Result<(), String> {
    prover
        .execute(elf, stdin)
        .cycle_limit(cycle_limit)
        .run()
        .map(|_| ())
        .map_err(|err| format!("SP1 execution failed (cycle limit {cycle_limit}): {err}"))
}

/// A stable key for one proving input: the program's verifying-key digest plus the stdin.
pub(super) fn input_key(vk_digest: &str, stdin: &SP1Stdin) -> u64 {
    let mut hasher = DefaultHasher::new();
    vk_digest.hash(&mut hasher);
    match bincode::serialize(stdin) {
        Ok(bytes) => bytes.hash(&mut hasher),
        // Not expected; still key by program so the bookkeeping stays conservative.
        Err(err) => err.to_string().hash(&mut hasher),
    }
    hasher.finish()
}

/// Whether an `sp1-cuda` client error means the server connection is unusable.
///
/// `CudaClientError` does not expose its variant, so this matches its 6.8.1 `Display` text.
/// sp1-cuda reports both server `InternalError` and `ProverError` responses as
/// "The server returned an internal error"; those keep the client (the server is alive), and
/// every other error, including unknown ones, retires it so a dead server is never reused.
pub(super) fn is_server_connection_failure(error: &str) -> bool {
    const SERVER_REPORTED: [&str; 2] = [
        "The server returned an internal error",
        "The server returned a prover error",
    ];
    !SERVER_REPORTED
        .iter()
        .any(|reported| error.contains(reported))
}

pub(super) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "unknown panic".to_string())
}

/// The one `sp1-gpu-server` process a start produced. None means sp1-cuda handed back a
/// connection that was still referenced instead of starting a server.
pub(super) fn single_new_server(device_id: u32, started: &[i32]) -> Result<i32, String> {
    match started {
        [pid] => Ok(*pid),
        [] => Err(format!(
            "no new sp1-gpu-server started on CUDA device {device_id}; sp1-cuda reused a previous \
             connection that is still referenced"
        )),
        pids => Err(format!(
            "found several new sp1-gpu-server processes for CUDA device {device_id}: {pids:?}"
        )),
    }
}

/// `now + timeout`, capped so an oversized timeout can never overflow `Instant`.
pub(super) fn deadline_after(timeout: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(timeout)
        .or_else(|| now.checked_add(Duration::from_secs(MAX_CUDA_TIMEOUT_SECS)))
        .unwrap_or(now)
}

/// A server start, or a setup or proof request currently being served by a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct InFlight {
    pub(super) device_id: u32,
    /// Client generation; `None` for a server start, which has no client yet.
    pub(super) generation: Option<u64>,
    pub(super) server_pid: Option<i32>,
    /// Proof input, so a proof that times out is not retried on the GPU.
    pub(super) input: Option<u64>,
    pub(super) operation: &'static str,
    pub(super) deadline: Instant,
}

/// Requests the watchdog tracks, keyed by request id.
#[derive(Default)]
pub(super) struct InFlightRequests(HashMap<u64, InFlight>);

impl InFlightRequests {
    pub(super) fn insert(&mut self, id: u64, request: InFlight) {
        self.0.insert(id, request);
    }

    pub(super) fn remove(&mut self, id: u64) {
        self.0.remove(&id);
    }

    /// Removes and returns every request whose deadline has passed, so each expires once.
    pub(super) fn take_expired(&mut self, now: Instant) -> Vec<InFlight> {
        let expired: Vec<u64> = self
            .0
            .iter()
            .filter(|(_, request)| request.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        expired
            .into_iter()
            .filter_map(|id| self.0.remove(&id))
            .collect()
    }
}

/// Inputs whose GPU proof exceeded the deadline. The protocol has no progress signal, so a slow
/// proof cannot be told apart from a hung server; remembering the input stops its retries from
/// taking the device down again. Bounded, oldest first out.
#[derive(Default)]
pub(super) struct TimedOutInputs(VecDeque<u64>);

impl TimedOutInputs {
    pub(super) fn record(&mut self, input: u64) {
        if !self.0.contains(&input) {
            if self.0.len() == TIMED_OUT_INPUTS_CAPACITY {
                self.0.pop_front();
            }
            self.0.push_back(input);
        }
    }

    pub(super) fn contains(&self, input: u64) -> bool {
        self.0.contains(&input)
    }
}

/// Server-reported failures since the last success, counted per distinct input, so one input
/// that fails deterministically (retried alone) cannot retire the device by itself.
#[derive(Default)]
pub(super) struct FailingInputs(Vec<u64>);

impl FailingInputs {
    /// Records a failure; returns how many distinct inputs failed in a row.
    pub(super) fn record_failure(&mut self, input: u64) -> usize {
        if !self.0.contains(&input) {
            self.0.push(input);
        }
        self.0.len()
    }

    pub(super) fn record_success(&mut self) {
        self.0.clear();
    }
}

/// Process state and parent PID from `/proc/<pid>/stat`. The command name may contain spaces
/// and parentheses, so fields are read after its closing parenthesis.
pub(super) fn parse_stat(stat: &str) -> Option<(char, u32)> {
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let ppid = fields.next()?.parse().ok()?;
    Some((state, ppid))
}

/// Whether a process state means the process has exited.
pub(super) const fn has_exited(state: char) -> bool {
    matches!(state, 'Z' | 'X' | 'x')
}

/// Whether `/proc/<pid>/cmdline` belongs to `binary`.
pub(super) fn is_binary_cmdline(cmdline: &[u8], binary: &str) -> bool {
    cmdline
        .split(|byte| *byte == 0)
        .next()
        .map(String::from_utf8_lossy)
        .is_some_and(|argv0| {
            Path::new(argv0.as_ref())
                .file_name()
                .is_some_and(|name| name == binary)
        })
}

/// Whether `/proc/<pid>/environ` sets `CUDA_VISIBLE_DEVICES` to exactly `device_id`.
pub(super) fn environ_selects_device(environ: &[u8], device_id: u32) -> bool {
    let expected = format!("CUDA_VISIBLE_DEVICES={device_id}");
    environ
        .split(|byte| *byte == 0)
        .any(|entry| entry == expected.as_bytes())
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use sp1_sdk::{SP1Stdin, blocking::LightProver};

    use super::{
        FailingInputs, InFlight, InFlightRequests, MAX_DISTINCT_FAILING_INPUTS, TimedOutInputs,
        deadline_after, environ_selects_device, execute_within_cycle_limit, has_exited, input_key,
        is_binary_cmdline, is_server_connection_failure, panic_message, parse_stat,
        single_new_server,
    };

    fn opcode_lab_elf() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../guests/elf/sp1_opcode_lab.elf");
        std::fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    #[test]
    fn cycle_limit_is_enforced_by_the_pre_execution() {
        let prover = LightProver::new();
        let err = execute_within_cycle_limit(&prover, opcode_lab_elf().into(), SP1Stdin::new(), 1)
            .expect_err("a one-cycle limit must stop execution");

        // The SDK's own limit error, not an unrelated failure such as missing stdin.
        assert!(err.contains("exceeded cycle limit"), "{err}");
    }

    #[test]
    fn server_reported_errors_keep_the_client() {
        assert!(!is_server_connection_failure(
            "CudaClientError: The server returned an internal error \n out of memory"
        ));
        assert!(is_server_connection_failure(
            "CudaClientError: Failed to read the response: Kind(UnexpectedEof)"
        ));
        assert!(is_server_connection_failure("something new"));
    }

    #[test]
    fn panic_message_reads_string_payloads() {
        let owned: Box<dyn std::any::Any + Send> = Box::new("boom".to_string());
        let borrowed: Box<dyn std::any::Any + Send> = Box::new("bang");
        let other: Box<dyn std::any::Any + Send> = Box::new(7_u8);

        assert_eq!(panic_message(owned.as_ref()), "boom");
        assert_eq!(panic_message(borrowed.as_ref()), "bang");
        assert_eq!(panic_message(other.as_ref()), "unknown panic");
    }

    #[test]
    fn a_start_must_produce_exactly_one_new_server() {
        assert_eq!(single_new_server(1, &[4242]), Ok(4242));
        let reused =
            single_new_server(1, &[]).expect_err("no new server means a reused connection");
        assert!(reused.contains("reused"), "{reused}");
        assert!(single_new_server(1, &[1, 2]).is_err());
    }

    #[test]
    fn oversized_timeouts_do_not_overflow_the_deadline() {
        let before = Instant::now();
        assert!(deadline_after(Duration::MAX) >= before);
        assert!(deadline_after(Duration::from_secs(60)) > before);
    }

    #[test]
    fn expired_requests_are_taken_once() {
        let now = Instant::now();
        let request = |operation, deadline| InFlight {
            device_id: 9,
            generation: Some(1),
            server_pid: Some(4242),
            input: Some(7),
            operation,
            deadline,
        };
        let mut in_flight = InFlightRequests::default();
        in_flight.insert(
            1,
            request(
                "prove",
                now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            ),
        );
        in_flight.insert(2, request("setup", now + Duration::from_hours(1)));

        let expired = in_flight.take_expired(now);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].operation, "prove");
        assert_eq!(expired[0].server_pid, Some(4242));
        assert!(in_flight.take_expired(now).is_empty());
        in_flight.remove(2);
        assert!(
            in_flight
                .take_expired(now + Duration::from_hours(2))
                .is_empty()
        );
    }

    #[test]
    fn timed_out_inputs_are_remembered_and_bounded() {
        let mut timed_out = TimedOutInputs::default();
        timed_out.record(1);
        timed_out.record(1);
        assert!(timed_out.contains(1));
        for input in 2..=300 {
            timed_out.record(input);
        }
        assert!(!timed_out.contains(1), "oldest entries are evicted");
        assert!(timed_out.contains(300));
    }

    #[test]
    fn failures_count_distinct_inputs() {
        let mut failing = FailingInputs::default();
        assert_eq!(failing.record_failure(1), 1);
        assert_eq!(
            failing.record_failure(1),
            1,
            "the same input retried does not add up"
        );
        assert_eq!(failing.record_failure(2), 2);
        assert_eq!(failing.record_failure(3), MAX_DISTINCT_FAILING_INPUTS);
        failing.record_success();
        assert_eq!(failing.record_failure(3), 1);
    }

    #[test]
    fn input_keys_differ_by_program_and_input() {
        let mut stdin = SP1Stdin::new();
        stdin.write(&1_u64);
        let mut other = SP1Stdin::new();
        other.write(&2_u64);

        assert_eq!(input_key("vk", &stdin), input_key("vk", &stdin));
        assert_ne!(input_key("vk", &stdin), input_key("vk", &other));
        assert_ne!(input_key("vk", &stdin), input_key("other-vk", &stdin));
    }

    #[test]
    fn stat_fields_follow_the_command_name() {
        assert_eq!(
            parse_stat("4242 (sp1-gpu-server) S 17 4242 4242 0 -1"),
            Some(('S', 17))
        );
        assert_eq!(parse_stat("7 (odd name) (x)) Z 1 7 7 0 -1"), Some(('Z', 1)));
        assert_eq!(parse_stat("garbage"), None);
        assert!(has_exited('Z') && has_exited('X') && !has_exited('S') && !has_exited('D'));
    }

    #[test]
    fn server_cmdline_matches_the_binary_name() {
        let binary = "sp1-gpu-server";
        assert!(is_binary_cmdline(
            b"/root/.sp1/bin/sp1-gpu-server\0",
            binary
        ));
        assert!(is_binary_cmdline(b"sp1-gpu-server\0--flag\0", binary));
        assert!(!is_binary_cmdline(b"/usr/local/bin/raiko2\0", binary));
        assert!(!is_binary_cmdline(b"/tmp/sp1-gpu-server-old\0", binary));
        assert!(!is_binary_cmdline(b"", binary));
    }

    #[test]
    fn environ_selects_exact_device() {
        let environ = b"HOME=/root\0CUDA_VISIBLE_DEVICES=1\0PATH=/bin\0";

        assert!(environ_selects_device(environ, 1));
        assert!(!environ_selects_device(environ, 0));
        assert!(!environ_selects_device(b"CUDA_VISIBLE_DEVICES=10\0", 1));
    }
}
