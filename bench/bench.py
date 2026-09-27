"""M0 benchmark: speed and RAM of candidate model packs against the brief's section 10 targets.

    python scripts/fetch_llama.py && python scripts/fetch_model.py small standard
    python bench/bench.py small standard                 # hardware as-is (Metal on Apple Silicon)
    python bench/bench.py small --cpu-only --threads 4   # rough proxy for an ordinary 4-core PC

Targets (8 GB RAM, no GPU): first token <= 3 s; a 200-token explanation <= 20 s. First-token time
is measured for a short (~400-token) and a long (~1,700-token) prompt, because on a CPU it is
dominated by prompt length and the brief does not fix one. The script runs
llama-server with the same flags the hub uses, one warm-up request and then --runs measured
requests per workload with prompt caching off, so every run pays for the full prompt.

Writes bench/results/<host>-<timestamp>.json and prints a markdown table for docs/spike-results.md.
Standard library only, but install psutil: it gives peak RSS on Windows and peak *private*
memory (USS) everywhere. Compare private memory, not RSS, when judging what fits in 8 GB: RSS
includes model pages the OS can drop and re-read.
"""

from __future__ import annotations

import argparse
import http.client
import json
import os
import platform
import secrets
import socket
import statistics
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MODELS = json.loads((ROOT / "bench" / "models.json").read_text())["models"]
TTFT_TARGET_S = 3.0
EXPLAIN_TARGET_S = 20.0

try:
    import psutil  # type: ignore
except ImportError:  # pragma: no cover - optional
    psutil = None

# A realistic Invoice Analytics-style request: evidence lines (~70 tokens each), then a short task.
def evidence(lines: int) -> str:
    return "\n".join(
    f"- Invoice INV-{2024000 + i}, supplier Northwind Medical Supplies, dated 2025-0{1 + i % 9}-{10 + i % 18}, "
    f"line {i % 4 + 1}: {['sterile gloves (box of 100)', 'saline 500 ml', 'gauze pads', 'syringes 5 ml'][i % 4]}, "
    f"qty {5 + i % 7}, unit price {12.5 + (i % 5) * 3.25:.2f}, total {(5 + i % 7) * (12.5 + (i % 5) * 3.25):.2f} USD"
    for i in range(lines)
)
SYSTEM = (
    "You explain billing anomalies to an accounts-payable clerk. Use only the evidence provided. "
    "Cite invoice numbers for every amount or date you mention. Be concise and neutral."
)
TASK = ("Flag: INV-2024004 and INV-2024016 may be duplicate billing for the same delivery. "
        "Explain in plain English why this was flagged and what the clerk should check next.")
EXPLAIN = f"Evidence:\n{evidence(24)}\n\n{TASK}"  # ~1,700 tokens: a full evidence pack
SHORT = f"Evidence:\n{evidence(4)}\n\n{TASK}"  # ~400 tokens: a single flagged pair
SCHEMA = {
    "type": "object",
    "properties": {
        "verdict": {"type": "string", "enum": ["likely_duplicate", "likely_legitimate", "needs_review"]},
        # Bounded lengths keep the constrained output inside max_tokens; unbounded strings let a
        # model run out of tokens mid-JSON (finish_reason "length"), which is invalid output.
        "reasons": {"type": "array", "items": {"type": "string", "maxLength": 160}, "minItems": 1, "maxItems": 3},
        "cited_invoices": {"type": "array", "items": {"type": "string", "maxLength": 12}, "maxItems": 4},
    },
    "required": ["verdict", "reasons", "cited_invoices"],
    "additionalProperties": False,
}


def machine() -> dict:
    info = {
        "os": f"{platform.system()} {platform.release()}",
        "arch": platform.machine(),
        "cpu": platform.processor() or platform.machine(),
        "logical_cpus": os.cpu_count(),
        "ram_gb": None,
        "host": socket.gethostname().split(".")[0],
    }
    try:
        if sys.platform == "darwin":
            info["cpu"] = subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"], capture_output=True, text=True).stdout.strip()
            info["ram_gb"] = round(int(subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, text=True).stdout) / 2**30, 1)
            info["os"] = f"macOS {platform.mac_ver()[0]}"
        elif sys.platform.startswith("linux"):
            for line in Path("/proc/cpuinfo").read_text().splitlines():
                if line.startswith("model name"):
                    info["cpu"] = line.split(":", 1)[1].strip()
                    break
            for line in Path("/proc/meminfo").read_text().splitlines():
                if line.startswith("MemTotal"):
                    info["ram_gb"] = round(int(line.split()[1]) / 2**20, 1)
            try:
                rel = dict(l.split("=", 1) for l in Path("/etc/os-release").read_text().splitlines() if "=" in l)
                info["os"] = rel.get("PRETTY_NAME", info["os"]).strip('"')
            except OSError:
                pass
        elif psutil:
            info["ram_gb"] = round(psutil.virtual_memory().total / 2**30, 1)
        if sys.platform == "win32":
            info["os"] = f"Windows {platform.release()} ({platform.version()})"
    except Exception:  # noqa: BLE001 - machine facts are best effort
        pass
    return info


def rss_bytes(pid: int) -> int | None:
    if psutil:
        try:
            return psutil.Process(pid).memory_info().rss
        except psutil.Error:
            return None
    if sys.platform != "win32":
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
        return int(out) * 1024 if out else None
    return None


def private_bytes(pid: int) -> int | None:
    """Unique set size: memory only this process holds. Unlike RSS it excludes clean, file-backed
    model pages the OS can drop and re-read under memory pressure. Needs psutil."""
    if sys.platform == "darwin":  # psutil can't read USS here; footprint(1) reports dirty memory
        out = subprocess.run(["footprint", str(pid)], capture_output=True, text=True).stdout
        for tok in out.split("Footprint:")[1:2]:
            num, unit = tok.split()[:2]
            return int(float(num) * {"KB": 2**10, "MB": 2**20, "GB": 2**30}.get(unit, 1))
        return None
    if not psutil:
        return None
    try:
        return psutil.Process(pid).memory_full_info().uss
    except psutil.Error:
        return None


class RamSampler(threading.Thread):
    def __init__(self, pid: int) -> None:
        super().__init__(daemon=True)
        self.pid, self.peak, self.peak_private, self._done = pid, 0, 0, threading.Event()

    def run(self) -> None:
        i = 0
        while not self._done.wait(0.25):
            r = rss_bytes(self.pid)
            if r:
                self.peak = max(self.peak, r)
            if i % 4 == 0:  # USS is slower to read; once a second is enough
                u = private_bytes(self.pid)
                if u:
                    self.peak_private = max(self.peak_private, u)
            i += 1

    def stop(self) -> tuple[int, int]:
        self._done.set()
        self.join()
        return self.peak, self.peak_private


class Server:
    def __init__(self, llama_dir: Path, model: Path, args: argparse.Namespace) -> None:
        exe = llama_dir / ("llama-server.exe" if sys.platform == "win32" else "llama-server")
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            self.port = s.getsockname()[1]
        self.key = secrets.token_hex(32)
        cmd = [str(exe), "-m", str(model), "--host", "127.0.0.1", "--port", str(self.port),
               "--ctx-size", str(args.ctx), "--parallel", "1", "--no-webui", "--no-slots"]
        if args.threads:
            cmd += ["--threads", str(args.threads)]
        if args.cpu_only:
            cmd += ["--n-gpu-layers", "0"]
        cmd += args.server_arg
        env = {**os.environ, "LLAMA_API_KEY": self.key}
        flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0  # type: ignore[attr-defined]
        t0 = time.perf_counter()
        self.proc = subprocess.Popen(cmd, cwd=llama_dir, env=env, stdout=subprocess.DEVNULL,
                                     stderr=subprocess.PIPE, creationflags=flags)
        self.ram = RamSampler(self.proc.pid)
        self.ram.start()
        while True:
            if self.proc.poll() is not None:
                err = self.proc.stderr.read().decode(errors="replace")[-2000:] if self.proc.stderr else ""
                sys.exit(f"llama-server exited during load:\n{err}")
            try:
                if self._get("/health") == 200:
                    break
            except OSError:
                pass
            if time.perf_counter() - t0 > 300:
                self.proc.kill()
                sys.exit("llama-server did not load within 300 s")
            time.sleep(0.2)
        self.load_s = time.perf_counter() - t0
        threading.Thread(target=self._drain, daemon=True).start()

    def _drain(self) -> None:
        assert self.proc.stderr
        for _ in self.proc.stderr:
            pass

    def _conn(self, timeout: float = 600) -> http.client.HTTPConnection:
        return http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)

    def _get(self, path: str) -> int:
        c = self._conn(2)
        c.request("GET", path)
        status = c.getresponse().status
        c.close()
        return status

    def chat(self, body: dict, stream: bool = False) -> dict:
        body = {**body, "stream": stream, "cache_prompt": False, "temperature": 0, "seed": 42,
                "chat_template_kwargs": {"enable_thinking": False}}
        c = self._conn()
        t0 = time.perf_counter()
        c.request("POST", "/v1/chat/completions", json.dumps(body),
                  {"Content-Type": "application/json", "Authorization": f"Bearer {self.key}"})
        r = c.getresponse()
        if r.status != 200:
            sys.exit(f"llama-server returned {r.status}: {r.read()[:500]!r}")
        if not stream:
            data = json.loads(r.read())
            data["_wall_s"] = time.perf_counter() - t0
            return data
        ttft = None
        text = []
        timings = {}
        for raw in r:
            line = raw.decode().strip()
            if not line.startswith("data: ") or line == "data: [DONE]":
                continue
            ev = json.loads(line[6:])
            timings = ev.get("timings", timings)
            delta = (ev.get("choices") or [{}])[0].get("delta", {})
            piece = delta.get("content") or ""
            if piece and ttft is None:
                ttft = time.perf_counter() - t0
            text.append(piece)
        c.close()
        return {"ttft_s": ttft, "wall_s": time.perf_counter() - t0, "text": "".join(text), "timings": timings}

    def close(self) -> tuple[int, int]:
        try:
            self.proc.terminate()
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        finally:
            peak = self.ram.stop()
        return peak


def messages(user: str) -> list[dict]:
    return [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}]


def bench_model(name: str, path: Path, llama_dir: Path, args: argparse.Namespace) -> dict:
    print(f"\n== {name}: {path.name}", flush=True)
    srv = Server(llama_dir, path, args)
    print(f"loaded in {srv.load_s:.1f} s", flush=True)
    try:
        # Warm-up (first request after load pays one-off costs such as Metal shader compilation).
        srv.chat({"messages": messages("Say OK."), "max_tokens": 4})

        ttft_short, short_tokens = [], 0
        for _ in range(args.runs):
            r = srv.chat({"messages": messages(SHORT), "max_tokens": 8}, stream=True)
            ttft_short.append(r["ttft_s"])
            short_tokens = r["timings"].get("prompt_n", short_tokens)
        print(f"  first token, {short_tokens}-token prompt: {statistics.median(ttft_short):.2f} s", flush=True)

        ttft, explain, prompt_tps, gen_tps, prompt_tokens = [], [], [], [], 0
        for i in range(args.runs):
            r = srv.chat({"messages": messages(EXPLAIN), "max_tokens": 200, "ignore_eos": True}, stream=True)
            t = r["timings"]
            ttft.append(r["ttft_s"])
            explain.append(r["wall_s"])
            prompt_tps.append(t.get("prompt_per_second", 0))
            gen_tps.append(t.get("predicted_per_second", 0))
            prompt_tokens = t.get("prompt_n", prompt_tokens)
            print(f"  run {i + 1}: first token {r['ttft_s']:.2f} s, 200 tokens {r['wall_s']:.1f} s, "
                  f"{t.get('predicted_per_second', 0):.1f} tok/s", flush=True)
        sample = r["text"]

        valid, schema_s = 0, []
        for _ in range(args.runs):
            r = srv.chat({
                "messages": messages(EXPLAIN + "\nAnswer as JSON."),
                "max_tokens": 400,
                "response_format": {"type": "json_schema", "json_schema": {"name": "triage", "schema": SCHEMA, "strict": True}},
            })
            schema_s.append(r["_wall_s"])
            try:
                out = json.loads(r["choices"][0]["message"]["content"])
                valid += int(set(out) == set(SCHEMA["required"]) and out["verdict"] in SCHEMA["properties"]["verdict"]["enum"])
            except (json.JSONDecodeError, KeyError, TypeError):
                pass
    finally:
        peak, peak_private = srv.close()

    med = statistics.median
    return {
        "pack": name,
        "file": path.name,
        "size_gb": round(path.stat().st_size / 2**30, 2),
        "load_s": round(srv.load_s, 2),
        "short_prompt_tokens": short_tokens,
        "ttft_short_s_median": round(med(ttft_short), 2),
        "prompt_tokens": prompt_tokens,
        "ttft_s_median": round(med(ttft), 2),
        "ttft_s_max": round(max(ttft), 2),
        "explain_200_s_median": round(med(explain), 2),
        "prompt_tok_per_s": round(med(prompt_tps), 1),
        "gen_tok_per_s": round(med(gen_tps), 1),
        "schema_valid": f"{valid}/{args.runs}",
        "schema_s_median": round(med(schema_s), 2),
        "peak_rss_gb": round(peak / 2**30, 2) if peak else None,
        "peak_private_gb": round(peak_private / 2**30, 2) if peak_private else None,
        "meets_ttft_short": med(ttft_short) <= TTFT_TARGET_S,
        "meets_ttft": med(ttft) <= TTFT_TARGET_S,
        "meets_explain": med(explain) <= EXPLAIN_TARGET_S,
        "sample_output": sample[:400],
    }


def table(results: list[dict], info: dict, args: argparse.Namespace) -> str:
    mode = "CPU only" if args.cpu_only else "default (GPU if available)"
    threads = args.threads or "auto"
    if args.server_arg:
        mode += " · extra: " + " ".join(args.server_arg)
    head = (f"**{info['host']}** · {info['os']} · {info['cpu']} · {info['logical_cpus']} logical CPUs · "
            f"{info['ram_gb']} GB RAM · mode: {mode} · threads: {threads} · llama.cpp {args.build}\n\n")
    rows = ["| Pack | Load | First token, short prompt (≤3 s) | First token, long prompt | 200-token answer, long prompt (≤20 s) | Prompt tok/s | Gen tok/s | Schema valid | Peak RSS | Peak private |",
            "|---|---|---|---|---|---|---|---|---|---|"]
    for r in results:
        ok = lambda b: "✅" if b else "❌"  # noqa: E731
        rows.append(
            f"| {r['pack']} ({r['size_gb']} GB) | {r['load_s']} s | {r['ttft_short_s_median']} s {ok(r['meets_ttft_short'])} | "
            f"{r['ttft_s_median']} s {ok(r['meets_ttft'])} | "
            f"{r['explain_200_s_median']} s {ok(r['meets_explain'])} | {r['prompt_tok_per_s']} | {r['gen_tok_per_s']} | "
            f"{r['schema_valid']} | {r['peak_rss_gb']} GB | {r['peak_private_gb'] or 'n/a (install psutil)'} GB |"
        )
    r0 = results[0]
    return (head + "\n".join(rows) + f"\n\nShort prompt {r0['short_prompt_tokens']} tokens, long prompt {r0['prompt_tokens']} tokens; "
            f"medians of {args.runs} runs after one warm-up, prompt cache off.\n")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("packs", nargs="+", help="names from bench/models.json, or paths to .gguf files")
    ap.add_argument("--llama-dir", type=Path, default=ROOT / "hub" / "app" / "resources" / "llama")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--ctx", type=int, default=8192)
    ap.add_argument("--threads", type=int)
    ap.add_argument("--cpu-only", action="store_true", help="no GPU offload (-ngl 0)")
    ap.add_argument("--server-arg", action="append", default=[], help="extra llama-server argument (repeatable)")
    args = ap.parse_args()

    stamp = args.llama_dir / "llama.json"
    if not stamp.exists():
        sys.exit("llama.cpp not staged; run: python scripts/fetch_llama.py")
    args.build = json.loads(stamp.read_text())["build"]

    results = []
    for p in args.packs:
        if p in MODELS:
            path = ROOT / ".cache" / "models" / MODELS[p]["file"]
            if not path.exists():
                sys.exit(f"{p} not downloaded; run: python scripts/fetch_model.py {p}")
        else:
            path = Path(p)
        results.append(bench_model(p, path, args.llama_dir, args))

    info = machine()
    md = table(results, info, args)
    out_dir = ROOT / "bench" / "results"
    out_dir.mkdir(exist_ok=True)
    ts = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = out_dir / f"{info['host']}-{ts}.json"
    out.write_text(json.dumps({"machine": info, "args": {k: str(v) for k, v in vars(args).items()},
                               "results": results, "markdown": md}, indent=2) + "\n")
    print("\n" + md)
    print(f"saved {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
