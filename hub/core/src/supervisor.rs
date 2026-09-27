//! Keeps one llama-server running: start on demand, restart with backoff after a crash.
//!
//! All spawning happens on the supervisor's own long-lived thread. On Linux the parent-death
//! signal is tied to the spawning thread, so spawning from short-lived threads would kill the
//! child when that thread exits.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::llama::{LlamaConfig, LlamaServer};
use crate::Error;

const START_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_RESTARTS: u32 = 5;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    Stopped,
    Starting,
    Ready { port: u16, pid: u32 },
    Restarting { attempt: u32 },
    Stopping,
    Failed { error: String },
}

enum Cmd {
    Start(LlamaConfig),
    Stop,
    Shutdown,
}

/// Connection details for the running server. Only the hub process ever sees the key.
#[derive(Clone)]
pub struct Endpoint {
    pub port: u16,
    pub api_key: String,
}

pub struct Supervisor {
    tx: Sender<Cmd>,
    state: Arc<Mutex<State>>,
    endpoint: Arc<Mutex<Option<Endpoint>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Supervisor {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::Stopped));
        let endpoint = Arc::new(Mutex::new(None));
        let (s, e) = (Arc::clone(&state), Arc::clone(&endpoint));
        let thread = thread::Builder::new()
            .name("llama-supervisor".into())
            .spawn(move || run(rx, s, e))
            .expect("failed to spawn supervisor thread");
        Supervisor { tx, state, endpoint, thread: Some(thread) }
    }

    /// Set the transitional state before queueing the command, so `wait_settled` called right
    /// after never sees the previous state.
    pub fn start(&self, cfg: LlamaConfig) {
        *self.state.lock().unwrap() = State::Starting;
        let _ = self.tx.send(Cmd::Start(cfg));
    }

    pub fn stop(&self) {
        *self.state.lock().unwrap() = State::Stopping;
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn endpoint(&self) -> Option<Endpoint> {
        self.endpoint.lock().unwrap().clone()
    }

    /// Block until the state is Ready or Failed, or the timeout passes.
    pub fn wait_settled(&self, timeout: Duration) -> State {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let s = self.state();
            if matches!(s, State::Ready { .. } | State::Failed { .. } | State::Stopped) || std::time::Instant::now() > deadline {
                return s;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(rx: Receiver<Cmd>, state: Arc<Mutex<State>>, endpoint: Arc<Mutex<Option<Endpoint>>>) {
    let set = |s: State| *state.lock().unwrap() = s;
    let mut server: Option<LlamaServer> = None;
    let mut config: Option<LlamaConfig> = None;
    let mut restarts = 0u32;

    let launch = |cfg: &LlamaConfig, server: &mut Option<LlamaServer>| -> Result<(), Error> {
        let s = LlamaServer::start(cfg, START_TIMEOUT)?;
        *endpoint.lock().unwrap() = Some(Endpoint { port: s.port, api_key: s.api_key.clone() });
        set(State::Ready { port: s.port, pid: s.pid() });
        *server = Some(s);
        Ok(())
    };

    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Cmd::Start(cfg)) => {
                server = None; // stops any previous model
                set(State::Starting);
                restarts = 0;
                match launch(&cfg, &mut server) {
                    Ok(()) => config = Some(cfg),
                    Err(e) => {
                        config = None;
                        set(State::Failed { error: e.to_string() });
                    }
                }
            }
            Ok(Cmd::Stop) => {
                server = None;
                config = None;
                *endpoint.lock().unwrap() = None;
                set(State::Stopped);
            }
            Ok(Cmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                drop(server.take());
                *endpoint.lock().unwrap() = None;
                set(State::Stopped);
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                let crashed = server.as_mut().is_some_and(|s| !s.is_running());
                if !crashed {
                    continue;
                }
                server = None;
                *endpoint.lock().unwrap() = None;
                let Some(cfg) = config.clone() else { continue };
                if restarts >= MAX_RESTARTS {
                    config = None;
                    set(State::Failed { error: format!("llama-server crashed {MAX_RESTARTS} times; not restarting") });
                    continue;
                }
                restarts += 1;
                set(State::Restarting { attempt: restarts });
                thread::sleep(Duration::from_secs(u64::from(restarts.min(5))));
                if let Err(e) = launch(&cfg, &mut server) {
                    set(State::Failed { error: e.to_string() });
                    config = None;
                }
            }
        }
    }
}
