use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, TerminateProcess, CREATE_NO_WINDOW, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};

pub const NAME: &str = "windows";

/// A Job Object with kill-on-close. The OS closes the handle when the hub exits for any reason,
/// which terminates every process in the job.
pub struct Guard {
    job: HANDLE,
}

// SAFETY: a job handle is a process-wide kernel handle with no thread affinity.
unsafe impl Send for Guard {}
unsafe impl Sync for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: we own this handle.
        unsafe {
            CloseHandle(self.job);
        }
    }
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("ABM").join("LocalAI")
}

pub fn prepare_child(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

pub fn bind_child_to_hub(child: &Child) -> io::Result<Guard> {
    // SAFETY: straightforward Win32 calls; every handle is checked and owned by Guard.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let guard = Guard { job };
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        if AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(guard)
    }
}

pub fn request_stop(_child: &Child) {
    // No console to send Ctrl-Break to (CREATE_NO_WINDOW); the caller terminates after a grace period.
}

pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: handle checked and closed.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE as u32
    }
}

pub fn kill_pid(pid: u32) {
    // SAFETY: handle checked and closed.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}
