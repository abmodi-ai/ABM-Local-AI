use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

pub use super::unix::{kill_pid, pid_alive, request_stop};

pub const NAME: &str = "macos";

/// macOS has no parent-death signal. A tiny watchdog process polls the hub's pid and kills
/// llama-server if the hub disappears (including a crash or force-quit). It lives in its own
/// process group so it outlives signals aimed at the hub's group.
pub struct Guard {
    watchdog: Child,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.watchdog.kill();
        let _ = self.watchdog.wait();
    }
}

pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Application Support/ABM/LocalAI")
}

pub fn prepare_child(cmd: &mut Command) {
    cmd.process_group(0);
}

pub fn bind_child_to_hub(child: &Child) -> io::Result<Guard> {
    let hub = std::process::id();
    let target = child.id();
    let script = format!(
        "while kill -0 {hub} 2>/dev/null; do sleep 1; done; \
         kill -TERM {target} 2>/dev/null; sleep 5; kill -KILL {target} 2>/dev/null"
    );
    let watchdog = Command::new("/bin/sh")
        .args(["-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    Ok(Guard { watchdog })
}
