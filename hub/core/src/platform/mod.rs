//! The only OS-specific code in the hub. Everything else is shared across Windows, macOS and Linux.
//!
//! | Concern                         | Windows                    | macOS                          | Linux                               |
//! |---------------------------------|----------------------------|--------------------------------|-------------------------------------|
//! | Per-user data directory         | `%LOCALAPPDATA%\ABM\LocalAI` | `~/Library/Application Support/ABM/LocalAI` | `$XDG_DATA_HOME/abm/localai` |
//! | llama-server dies with the hub  | Job Object, kill-on-close  | watchdog process + own group   | `PR_SET_PDEATHSIG` + own group      |
//! | Hidden console for the child    | `CREATE_NO_WINDOW`         | n/a                            | n/a                                 |

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
use windows as imp;

/// Keeps the OS mechanism that ties a child's lifetime to the hub. Dropping it releases that
/// mechanism (on Windows this kills the child, so hold it for as long as the child should live).
pub struct ChildGuard(#[allow(dead_code)] imp::Guard);

/// Short platform name used in status output and logs.
pub const NAME: &str = imp::NAME;

/// Per-user directory for `endpoint.json`, installed packs and metadata-only logs.
pub fn data_dir() -> PathBuf {
    imp::data_dir()
}

/// Apply spawn-time settings so the child can be tied to the hub (call before `spawn`).
pub fn prepare_child(cmd: &mut Command) {
    imp::prepare_child(cmd)
}

/// Tie a spawned child's lifetime to this process, so it is killed even if the hub crashes.
pub fn bind_child_to_hub(child: &Child) -> io::Result<ChildGuard> {
    imp::bind_child_to_hub(child).map(ChildGuard)
}

/// Ask a child to exit cleanly (SIGTERM on Unix). Windows has no equivalent for console-less
/// processes, so there the caller falls back to `Child::kill` after the grace period.
pub fn request_stop(child: &Child) {
    imp::request_stop(child)
}

/// Whether a process with this pid is still running.
pub fn pid_alive(pid: u32) -> bool {
    imp::pid_alive(pid)
}

/// Kill a process by pid without waiting (used by the orphan test to simulate a hub crash).
pub fn kill_pid(pid: u32) {
    imp::kill_pid(pid)
}

#[cfg(unix)]
mod unix {
    use std::process::Child;

    pub fn request_stop(child: &Child) {
        // SAFETY: plain kill(2) on our own child's pid.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
    }

    pub fn pid_alive(pid: u32) -> bool {
        // SAFETY: signal 0 only checks for existence and permission.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if r == 0 {
            return !is_zombie(pid);
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    /// A killed child that nobody has reaped yet still answers kill(pid, 0).
    fn is_zombie(pid: u32) -> bool {
        std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim_start().starts_with('Z'))
            .unwrap_or(false)
    }

    pub fn kill_pid(pid: u32) {
        // SAFETY: plain kill(2).
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
}
