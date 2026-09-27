use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};

pub use super::unix::{kill_pid, pid_alive, request_stop};

pub const NAME: &str = "linux";

pub struct Guard;

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"));
    base.join("abm").join("localai")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn prepare_child(cmd: &mut Command) {
    // Own process group, so a terminal Ctrl-C aimed at the hub doesn't bypass its shutdown path.
    cmd.process_group(0);
    // SAFETY: getpid is async-signal-safe.
    let hub_pid = unsafe { libc::getpid() };
    // SAFETY: only async-signal-safe calls (prctl, getppid, _exit) run between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            // The kernel sends SIGKILL when the spawning *thread* exits. The supervisor spawns
            // from long-lived threads only (see supervisor.rs).
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != hub_pid {
                libc::_exit(1); // the hub died before prctl took effect
            }
            Ok(())
        });
    }
}

pub fn bind_child_to_hub(_child: &Child) -> io::Result<Guard> {
    Ok(Guard) // done at spawn time by PR_SET_PDEATHSIG
}
