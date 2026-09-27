//! Headless smoke test for the hub core. Runs in CI on Windows, macOS and Linux.
//!
//!     abm-hub-smoke --llama-dir hub/app/resources/llama --model .cache/models/SmolLM2-135M-Instruct-Q4_K_M.gguf
//!
//! Checks, in order:
//!  1. the supervisor starts llama-server and it becomes healthy;
//!  2. a chat completion constrained by a JSON schema returns schema-valid JSON;
//!  3. requests without the random API key are refused;
//!  4. the server is not reachable on a non-loopback address;
//!  5. llama-server is killed when the hub process is killed abruptly (no orphan);
//!  6. a clean stop leaves no process behind.
//!
//! Exits non-zero on the first failure and prints a JSON report on success.

use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use abm_hub_core::{http, platform, LlamaConfig, State, Supervisor};
use serde_json::json;

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("FAIL: {msg}");
    std::process::exit(1);
}

fn ok(step: &str) {
    eprintln!("ok   {step}");
}

struct Args {
    llama_dir: PathBuf,
    model: PathBuf,
    hold: bool,
}

fn parse_args() -> Args {
    let mut it = std::env::args().skip(1);
    let (mut llama_dir, mut model, mut hold) = (None, None, false);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--llama-dir" => llama_dir = it.next().map(PathBuf::from),
            "--model" => model = it.next().map(PathBuf::from),
            "--hold" => hold = true,
            other => fail(format!("unknown argument {other}")),
        }
    }
    let llama_dir = llama_dir.unwrap_or_else(|| fail("--llama-dir is required"));
    let model = model.unwrap_or_else(|| fail("--model is required"));
    Args { llama_dir, model, hold }
}

fn start(args: &Args) -> Supervisor {
    let sup = Supervisor::new();
    let mut cfg = LlamaConfig::new(&args.llama_dir, &args.model);
    cfg.ctx_size = 2048;
    sup.start(cfg);
    sup
}

/// Child mode for the orphan test: start llama-server, report its pid, then wait to be killed.
fn hold(args: &Args) -> ! {
    let sup = start(args);
    match sup.wait_settled(Duration::from_secs(180)) {
        State::Ready { pid, .. } => println!("LLAMA_PID {pid}"),
        other => fail(format!("hold: server not ready: {other:?}")),
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// The address other machines would use to reach this one (no packets are sent).
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1, never routed
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn wait_dead(pid: u32, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if !platform::pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn main() {
    let args = parse_args();
    if args.hold {
        hold(&args);
    }

    // 1. start
    let t0 = Instant::now();
    let sup = start(&args);
    let (port, pid) = match sup.wait_settled(Duration::from_secs(180)) {
        State::Ready { port, pid } => (port, pid),
        other => fail(format!("server did not start: {other:?}")),
    };
    let startup_ms = t0.elapsed().as_millis();
    ok(&format!("llama-server ready on 127.0.0.1:{port} (pid {pid}) in {startup_ms} ms"));
    let ep = sup.endpoint().unwrap_or_else(|| fail("no endpoint while ready"));

    // 2. schema-constrained chat completion
    let schema = json!({
        "type": "object",
        "properties": { "colour": { "type": "string" }, "count": { "type": "integer" } },
        "required": ["colour", "count"],
        "additionalProperties": false
    });
    let body = json!({
        "messages": [{ "role": "user", "content": "Name one colour and a number between 1 and 9." }],
        "max_tokens": 64,
        "temperature": 0,
        "response_format": { "type": "json_schema", "json_schema": { "name": "out", "schema": schema, "strict": true } }
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let t1 = Instant::now();
    let resp = http::request(ep.port, "POST", "/v1/chat/completions", Some(&ep.api_key), Some(&bytes), Duration::from_secs(120))
        .unwrap_or_else(|e| fail(format!("chat request failed: {e}")));
    let chat_ms = t1.elapsed().as_millis();
    if resp.status != 200 {
        fail(format!("chat returned {}: {}", resp.status, String::from_utf8_lossy(&resp.body)));
    }
    let v = resp.json().unwrap_or_else(|e| fail(e));
    let content = v["choices"][0]["message"]["content"].as_str().unwrap_or_else(|| fail("no content in response"));
    let parsed: serde_json::Value = serde_json::from_str(content).unwrap_or_else(|e| fail(format!("content is not JSON ({e}): {content}")));
    if !(parsed["colour"].is_string() && parsed["count"].is_i64() && parsed.as_object().map(|o| o.len()) == Some(2)) {
        fail(format!("content does not match the schema: {content}"));
    }
    ok(&format!("schema-valid JSON in {chat_ms} ms: {content}"));

    // 3. API key enforced
    let unauth = http::request(ep.port, "POST", "/v1/chat/completions", None, Some(&bytes), Duration::from_secs(10))
        .unwrap_or_else(|e| fail(format!("unauthenticated request errored: {e}")));
    if unauth.status != 401 {
        fail(format!("request without the API key returned {} (expected 401)", unauth.status));
    }
    let wrong = http::request(ep.port, "POST", "/v1/chat/completions", Some("wrong"), Some(&bytes), Duration::from_secs(10))
        .unwrap_or_else(|e| fail(format!("wrong-key request errored: {e}")));
    if wrong.status != 401 {
        fail(format!("request with a wrong API key returned {} (expected 401)", wrong.status));
    }
    ok("requests without the random key are refused (401)");

    // 4. not reachable from the network
    let lan = match lan_ip() {
        Some(ip) => {
            if TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_secs(2)).is_ok() {
                fail(format!("llama-server is reachable on non-loopback address {ip}:{port}"));
            }
            ok(&format!("not reachable on {ip}:{port}"));
            ip.to_string()
        }
        None => {
            ok("no non-loopback interface; reachability check skipped");
            "none".into()
        }
    };

    // 5. orphan test: kill a hub process abruptly and check its llama-server dies with it
    let exe = std::env::current_exe().unwrap();
    let mut holder = Command::new(exe)
        .args(["--hold", "--llama-dir"])
        .arg(&args.llama_dir)
        .arg("--model")
        .arg(&args.model)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| fail(format!("could not spawn hold process: {e}")));
    let mut line = String::new();
    BufReader::new(holder.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap_or_else(|e| fail(format!("hold process gave no pid: {e}")));
    let orphan_pid: u32 = line
        .trim()
        .strip_prefix("LLAMA_PID ")
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| fail(format!("unexpected hold output: {line:?}")));
    platform::kill_pid(holder.id());
    let _ = holder.wait();
    if !wait_dead(orphan_pid, Duration::from_secs(15)) {
        platform::kill_pid(orphan_pid);
        fail(format!("llama-server (pid {orphan_pid}) survived its hub being killed ({})", platform::NAME));
    }
    ok(&format!("llama-server died with its hub after a hard kill ({})", platform::NAME));

    // 6. clean stop
    sup.stop();
    if sup.wait_settled(Duration::from_secs(20)) != State::Stopped {
        fail("supervisor did not reach Stopped");
    }
    if !wait_dead(pid, Duration::from_secs(15)) {
        fail(format!("llama-server (pid {pid}) still running after stop"));
    }
    ok("clean stop leaves no process");

    let report = json!({
        "platform": platform::NAME,
        "arch": std::env::consts::ARCH,
        "startup_ms": startup_ms,
        "schema_chat_ms": chat_ms,
        "lan_ip_checked": lan,
        "result": "pass"
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
