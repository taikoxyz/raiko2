//! Finds, checks, and kills this process's `sp1-gpu-server` children through `/proc` (Linux).
//!
//! `sp1-sdk` spawns one `sp1-gpu-server` child per CUDA device with
//! `CUDA_VISIBLE_DEVICES=<device id>` and does not expose its PID. The watchdog uses this to tell
//! whether a device's server is still running and to kill one that stopped answering. The pure
//! parsing lives in [`super::super::cuda_support`] so CI tests it without CUDA.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};
use tracing::warn;

use super::super::cuda_support::{
    environ_selects_device, has_exited, is_binary_cmdline, parse_stat,
};

const SERVER_BINARY: &str = "sp1-gpu-server";
const POLL: Duration = Duration::from_millis(100);

/// Whether `/proc` is available to inspect child processes.
pub(super) fn supported() -> bool {
    Path::new("/proc/self/stat").exists()
}

/// Checks that the `sp1-gpu-server` binary sp1-sdk will start exists and reports exactly
/// `expected_version`, within `timeout`.
///
/// sp1-sdk otherwise downloads a server from GitHub with no timeout (and needs `systemctl` on a
/// version mismatch), which would leave a start blocked with nothing for the watchdog to kill.
pub(super) fn check_server_binary(expected_version: &str, timeout: Duration) -> Result<(), String> {
    let path = server_binary_path()?;
    if !path.is_file() {
        return Err(format!(
            "{} is missing; install the pinned sp1-gpu-server {expected_version} (Dockerfile.cuda \
             does) instead of letting sp1-sdk download one",
            path.display()
        ));
    }
    let mut child = Command::new(&path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("failed to run {} --version: {err}", path.display()))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} --version did not finish within {timeout:?}",
                    path.display()
                ));
            }
            Err(err) => return Err(format!("failed to wait for {}: {err}", path.display())),
        }
    };
    let mut version = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut version);
    }
    if !status.success() || version.trim() != expected_version {
        return Err(format!(
            "{} --version reported {:?} ({status}); sp1-sdk needs exactly {expected_version}",
            path.display(),
            version.trim()
        ));
    }
    Ok(())
}

/// Where sp1-sdk 6.8.1 starts the server from: `$HOME/.sp1/bin/sp1-gpu-server`.
fn server_binary_path() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join(".sp1/bin").join(SERVER_BINARY))
        .ok_or_else(|| "HOME is not set; sp1-sdk cannot locate sp1-gpu-server".to_string())
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

/// Whether `pid` exists and has not exited yet (zombies have exited).
fn is_running(pid: i32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| parse_stat(&stat))
        .is_some_and(|(state, _)| !has_exited(state))
}

fn is_server_for_device(pid: i32, parent: u32, device_id: u32) -> bool {
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((state, parent_of_process)) = parse_stat(&stat) else {
        return false;
    };
    if has_exited(state) || parent_of_process != parent {
        return false;
    }
    let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    if !is_binary_cmdline(&cmdline, SERVER_BINARY) {
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
