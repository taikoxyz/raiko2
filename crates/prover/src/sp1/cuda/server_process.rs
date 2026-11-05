//! Finds and kills this process's `sp1-gpu-server` children through `/proc` (Linux).
//!
//! `sp1-sdk` spawns one `sp1-gpu-server` child per CUDA device with
//! `CUDA_VISIBLE_DEVICES=<device id>` and does not expose its PID. The watchdog uses this to tell
//! whether a device's server is still running and to kill one that stopped answering.

use std::fs;
use std::path::Path;

use rustix::process::{Pid, Signal, kill_process};

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

/// Sends `SIGKILL` to every `sp1-gpu-server` child serving `device_id`; returns how many.
pub(super) fn kill_servers(device_id: u32) -> usize {
    server_pids(device_id)
        .into_iter()
        .filter_map(Pid::from_raw)
        .filter(|pid| kill_process(*pid, Signal::KILL).is_ok())
        .count()
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
    let Ok(environ) = fs::read(format!("/proc/{pid}/environ")) else {
        return false;
    };
    is_server_cmdline(&cmdline) && environ_selects_device(&environ, device_id)
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
