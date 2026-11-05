//! Finds and kills this process's `sp1-gpu-server` children through `/proc` (Linux).
//!
//! `sp1-sdk` spawns one `sp1-gpu-server` child per CUDA device with
//! `CUDA_VISIBLE_DEVICES=<device id>` and does not expose its PID. The watchdog uses this to tell
//! whether a device's server is still running and to kill one that stopped answering.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};
use tracing::warn;

const SERVER_BINARY: &str = "sp1-gpu-server";

/// Whether `/proc` is available to inspect child processes.
pub(super) fn supported() -> bool {
    Path::new("/proc/self/stat").exists()
}

/// Live (non-zombie) `sp1-gpu-server` children of this process serving `device_id`.
pub(super) fn server_pids(device_id: u32) -> Vec<i32> {
    let parent = std::process::id();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| is_server_for_device(*pid, parent, device_id))
        .collect()
}

/// Sends `SIGKILL` to every `sp1-gpu-server` child serving `device_id`; returns their PIDs.
pub(super) fn kill_servers(device_id: u32) -> Vec<i32> {
    server_pids(device_id)
        .into_iter()
        .filter(|pid| kill_server(*pid, device_id))
        .collect()
}

/// Waits until every PID in `pids` has exited (zombie or gone), so its socket listener and GPU
/// memory are released. Returns the PIDs still running after `timeout`.
pub(super) fn wait_exited(pids: &[i32], timeout: Duration) -> Vec<i32> {
    const POLL: Duration = Duration::from_millis(100);
    let deadline = Instant::now() + timeout;
    loop {
        let running: Vec<i32> = pids
            .iter()
            .copied()
            .filter(|pid| is_running(*pid))
            .collect();
        if running.is_empty() || Instant::now() >= deadline {
            return running;
        }
        std::thread::sleep(POLL);
    }
}

/// Whether `pid` exists and has not exited yet (zombies have exited).
fn is_running(pid: i32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| parse_stat(&stat))
        .is_some_and(|(state, _)| !matches!(state, 'Z' | 'X' | 'x'))
}

/// Whether `pid` is still a live `sp1-gpu-server` child of this process serving `device_id`.
pub(super) fn is_server(pid: i32, device_id: u32) -> bool {
    is_server_for_device(pid, std::process::id(), device_id)
}

/// Sends `SIGKILL` to `pid` only if it is still a live `sp1-gpu-server` child serving
/// `device_id`, so a reused PID or a different server is never signalled.
pub(super) fn kill_server(pid: i32, device_id: u32) -> bool {
    is_server(pid, device_id)
        && Pid::from_raw(pid).is_some_and(|pid| kill_process(pid, Signal::KILL).is_ok())
}

fn is_server_for_device(pid: i32, parent: u32, device_id: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((state, parent_of_process)) = parse_stat(&stat) else {
        return false;
    };
    if state == 'Z' || parent_of_process != parent {
        return false;
    }
    let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    if !is_server_cmdline(&cmdline) {
        return false;
    }
    match fs::read(format!("/proc/{pid}/environ")) {
        Ok(environ) => environ_selects_device(&environ, device_id),
        // Unreadable environ (non-dumpable process, LSM policy): fall back to binary name and
        // parent. raiko2 runs at most one SP1 GPU device per process, so this stays unambiguous.
        Err(err) => {
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                warn!(
                    pid,
                    %err,
                    "Cannot read sp1-gpu-server environ; matching servers by binary name and parent only"
                );
            }
            true
        }
    }
}

/// Process state and parent PID from `/proc/<pid>/stat`. The command name may contain spaces
/// and parentheses, so fields are read after its closing parenthesis.
fn parse_stat(stat: &str) -> Option<(char, u32)> {
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let ppid = fields.next()?.parse().ok()?;
    Some((state, ppid))
}

fn is_server_cmdline(cmdline: &[u8]) -> bool {
    cmdline
        .split(|byte| *byte == 0)
        .next()
        .map(String::from_utf8_lossy)
        .is_some_and(|argv0| {
            Path::new(argv0.as_ref())
                .file_name()
                .is_some_and(|name| name == SERVER_BINARY)
        })
}

fn environ_selects_device(environ: &[u8], device_id: u32) -> bool {
    let expected = format!("CUDA_VISIBLE_DEVICES={device_id}");
    environ
        .split(|byte| *byte == 0)
        .any(|entry| entry == expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{environ_selects_device, is_server_cmdline, parse_stat};

    #[test]
    fn stat_fields_follow_the_command_name() {
        assert_eq!(
            parse_stat("4242 (sp1-gpu-server) S 17 4242 4242 0 -1"),
            Some(('S', 17))
        );
        assert_eq!(parse_stat("7 (odd name) (x)) Z 1 7 7 0 -1"), Some(('Z', 1)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn server_cmdline_matches_the_binary_name() {
        assert!(is_server_cmdline(b"/root/.sp1/bin/sp1-gpu-server\0"));
        assert!(is_server_cmdline(b"sp1-gpu-server\0--flag\0"));
        assert!(!is_server_cmdline(b"/usr/local/bin/raiko2\0"));
        assert!(!is_server_cmdline(b"/tmp/sp1-gpu-server-old\0"));
        assert!(!is_server_cmdline(b""));
    }

    #[test]
    fn environ_selects_exact_device() {
        let environ = b"HOME=/root\0CUDA_VISIBLE_DEVICES=1\0PATH=/bin\0";

        assert!(environ_selects_device(environ, 1));
        assert!(!environ_selects_device(environ, 0));
        assert!(!environ_selects_device(b"CUDA_VISIBLE_DEVICES=10\0", 1));
    }
}
