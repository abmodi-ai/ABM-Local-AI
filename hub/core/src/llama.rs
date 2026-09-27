//! One llama-server child process: loopback only, random port, random API key.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::{http, platform, Error};

/// Lines of llama-server stderr kept in memory for error messages. Never written to disk.
const STDERR_TAIL: usize = 40;

#[derive(Clone, Debug)]
pub struct LlamaConfig {
    /// Directory holding llama-server and its shared libraries (see scripts/fetch_llama.py).
    pub llama_dir: PathBuf,
    pub model: PathBuf,
    pub ctx_size: u32,
    pub threads: Option<u32>,
    pub extra_args: Vec<String>,
}

impl LlamaConfig {
    pub fn new(llama_dir: impl Into<PathBuf>, model: impl Into<PathBuf>) -> Self {
        Self { llama_dir: llama_dir.into(), model: model.into(), ctx_size: 8192, threads: None, extra_args: vec![] }
    }

    pub fn server_binary(&self) -> PathBuf {
        server_binary(&self.llama_dir)
    }
}

pub fn server_binary(llama_dir: &Path) -> PathBuf {
    llama_dir.join(if cfg!(windows) { "llama-server.exe" } else { "llama-server" })
}

pub struct LlamaServer {
    child: Child,
    _guard: platform::ChildGuard,
    pub port: u16,
    pub api_key: String,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

fn random_key() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random source unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn free_loopback_port() -> Result<u16, Error> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

impl LlamaServer {
    /// Start llama-server and wait until /health returns 200 (model loaded) or `timeout` passes.
    pub fn start(cfg: &LlamaConfig, timeout: Duration) -> Result<Self, Error> {
        // Absolute paths: the child runs with its own directory as the working directory.
        let cfg = &LlamaConfig {
            llama_dir: std::path::absolute(&cfg.llama_dir)?,
            model: std::path::absolute(&cfg.model)?,
            ..cfg.clone()
        };
        let bin = cfg.server_binary();
        if !bin.is_file() {
            return Err(Error::new(format!("llama-server not found at {}", bin.display())));
        }
        if !cfg.model.is_file() {
            return Err(Error::new(format!("model file not found: {}", cfg.model.display())));
        }
        let port = free_loopback_port()?;
        let api_key = random_key();
        let mut cmd = Command::new(&bin);
        cmd.arg("-m")
            .arg(&cfg.model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // The key goes in the environment, not argv: argv is visible to every user in `ps`,
            // while another process's environment is readable only by its owner (or root).
            .env("LLAMA_API_KEY", &api_key)
            .args(["--ctx-size", &cfg.ctx_size.to_string()])
            // One sequence at a time: the hub's queue decides who runs next.
            .args(["--parallel", "1"])
            // No web UI, and no /slots endpoint (it can echo prompt text).
            .args(["--no-webui", "--no-slots"])
            .args(&cfg.extra_args)
            .current_dir(&cfg.llama_dir)
            .env_remove("LLAMA_ARG_HOST")
            .env_remove("LLAMA_ARG_PORT")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(t) = cfg.threads {
            cmd.args(["--threads", &t.to_string()]);
        }
        platform::prepare_child(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| Error::new(format!("failed to start llama-server: {e}")))?;
        let guard = match platform::bind_child_to_hub(&child) {
            Ok(g) => g,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::new(format!("failed to bind llama-server to the hub: {e}")));
            }
        };
        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
        if let Some(stderr) = child.stderr.take() {
            let tail = Arc::clone(&stderr_tail);
            thread::Builder::new()
                .name("llama-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let mut t = tail.lock().unwrap();
                        if t.len() == STDERR_TAIL {
                            t.pop_front();
                        }
                        t.push_back(line);
                    }
                })
                .ok();
        }
        let mut server = LlamaServer { child, _guard: guard, port, api_key, stderr_tail };
        server.wait_healthy(timeout)?;
        Ok(server)
    }

    fn wait_healthy(&mut self, timeout: Duration) -> Result<(), Error> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(Error::new(format!("llama-server exited during startup ({status}){}", self.tail_hint())));
            }
            if self.healthy() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(250));
        }
        self.stop();
        Err(Error::new(format!("llama-server did not become healthy in {}s{}", timeout.as_secs(), self.tail_hint())))
    }

    pub fn healthy(&self) -> bool {
        matches!(http::request(self.port, "GET", "/health", None, None, Duration::from_secs(2)), Ok(r) if r.status == 200)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail.lock().unwrap().iter().cloned().collect()
    }

    fn tail_hint(&self) -> String {
        // Give llama-server's reader thread a moment to collect the final lines.
        thread::sleep(Duration::from_millis(200));
        let tail = self.stderr_tail();
        let last: Vec<_> = tail.iter().rev().take(3).rev().cloned().collect();
        if last.is_empty() {
            String::new()
        } else {
            format!(": {}", last.join(" | "))
        }
    }

    /// POST a JSON body to an OpenAI-compatible endpoint and return the parsed JSON response.
    pub fn post_json(&self, path: &str, body: &serde_json::Value, timeout: Duration) -> Result<serde_json::Value, Error> {
        let bytes = serde_json::to_vec(body).map_err(|e| Error::new(e.to_string()))?;
        let r = http::request(self.port, "POST", path, Some(&self.api_key), Some(&bytes), timeout)?;
        if r.status != 200 {
            return Err(Error::new(format!("llama-server returned {}: {}", r.status, String::from_utf8_lossy(&r.body))));
        }
        r.json()
    }

    /// Stop gracefully (SIGTERM, then kill after 10 s). Safe to call more than once.
    pub fn stop(&mut self) {
        if !self.is_running() {
            return;
        }
        platform::request_stop(&self.child);
        let deadline = Instant::now() + Duration::from_secs(if cfg!(windows) { 0 } else { 10 });
        while Instant::now() < deadline {
            if !self.is_running() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        self.stop();
    }
}
