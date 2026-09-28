"""Shared pieces for the evaluation: llama-server processes, embeddings, retrieval and prompts.

Standard library only. Models run with the same llama.cpp build and flags as the hub, plus each
catalog entry's server_args (e.g. thinking off), so results reflect what users would get.
"""

from __future__ import annotations

import http.client
import json
import math
import os
import re
import secrets
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LLAMA_DIR = ROOT / "hub" / "app" / "resources" / "llama"
MODELS_DIR = ROOT / ".cache" / "models"
PACK = ROOT / ".cache" / "eval" / "pack"
WORK = ROOT / ".cache" / "eval"
BENCH_MODELS = json.loads((ROOT / "bench" / "models.json").read_text())["models"]
CATALOG = {m["id"]: m for m in json.loads((ROOT / "packs" / "catalog.json").read_text())["models"]}

# Contestants: bench/models.json key -> catalog id (for server_args) and display name.
CONTESTANTS = {
    "qwen3.5-4b": {"catalog": "qwen3.5-4b", "name": "Qwen3.5 4B"},
    "granite-4.2-3b": {"catalog": "granite-4.2-3b", "name": "Granite 4.2 3B"},
}
EMBEDDER = "embed-qwen3-0.6b"
JUDGE = "judge-gpt-oss-120b"

REFUSAL = "The provided sources do not cover this."


def model_path(key: str) -> Path:
    p = MODELS_DIR / BENCH_MODELS[key]["file"]
    if not p.exists():
        sys.exit(f"{key} not downloaded; run: python scripts/fetch_model.py {key}")
    return p


class Server:
    """A llama-server on loopback with a random key; stopped when the context exits."""

    def __init__(self, model: Path, extra: list[str] | None = None, ctx: int = 8192, parallel: int = 1) -> None:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            self.port = s.getsockname()[1]
        self.key = secrets.token_hex(24)
        exe = LLAMA_DIR / ("llama-server.exe" if sys.platform == "win32" else "llama-server")
        cmd = [str(exe), "-m", str(model), "--host", "127.0.0.1", "--port", str(self.port), "--ctx-size", str(ctx),
               "--parallel", str(parallel), "--no-webui", "--no-slots", *(extra or [])]
        self.proc = subprocess.Popen(cmd, cwd=LLAMA_DIR, env={**os.environ, "LLAMA_API_KEY": self.key},
                                     stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        t0 = time.time()
        while True:
            if self.proc.poll() is not None:
                err = self.proc.stderr.read().decode(errors="replace")[-1500:] if self.proc.stderr else ""
                sys.exit(f"llama-server failed to start for {model.name}:\n{err}")
            try:
                c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=2)
                c.request("GET", "/health")
                if c.getresponse().status == 200:
                    break
            except OSError:
                pass
            if time.time() - t0 > 600:
                self.close()
                sys.exit(f"{model.name} did not load within 10 minutes")
            time.sleep(0.3)
        # Drain stderr so the pipe never fills up.
        import threading

        threading.Thread(target=lambda: [None for _ in self.proc.stderr], daemon=True).start()

    def post(self, path: str, body: dict, timeout: float = 900) -> dict:
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        c.request("POST", path, json.dumps(body), {"Content-Type": "application/json", "Authorization": f"Bearer {self.key}"})
        r = c.getresponse()
        data = r.read()
        if r.status != 200:
            raise RuntimeError(f"llama-server {path} returned {r.status}: {data[:400]!r}")
        return json.loads(data)

    def chat(self, messages: list[dict], **kw) -> tuple[str, dict]:
        body = {"messages": messages, "temperature": 0, "seed": 42, "cache_prompt": False, **kw}
        t0 = time.perf_counter()
        d = self.post("/v1/chat/completions", body)
        msg = d["choices"][0]["message"]
        return (msg.get("content") or ""), {"seconds": round(time.perf_counter() - t0, 3), "usage": d.get("usage", {}),
                                            "finish": d["choices"][0].get("finish_reason")}

    def close(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()

    def __enter__(self) -> "Server":
        return self

    def __exit__(self, *a) -> None:
        self.close()


def load_passages() -> list[dict]:
    path = PACK / "passages.jsonl"
    if not path.exists():
        sys.exit("knowledge pack not built; run: uv run eval/build_pack.py")
    return [json.loads(l) for l in path.open()]


def pack_hash() -> str:
    return json.loads((PACK / "manifest.json").read_text())["pack_sha256"]


# Qwen3-Embedding expects an instruction on queries (not on documents).
QUERY_PREFIX = "Instruct: Given a question about GxP computerised-system regulations, retrieve passages that answer it\nQuery: "


def embed_all(texts: list[str], srv: Server, batch: int = 16) -> list[list[float]]:
    out = []
    for i in range(0, len(texts), batch):
        d = srv.post("/v1/embeddings", {"input": texts[i : i + batch]})
        out += [x["embedding"] for x in sorted(d["data"], key=lambda x: x["index"])]
    return [normalise(v) for v in out]


def normalise(v: list[float]) -> list[float]:
    n = math.sqrt(sum(x * x for x in v)) or 1.0
    return [x / n for x in v]


def cosine(a: list[float], b: list[float]) -> float:
    return sum(x * y for x, y in zip(a, b))


class Retriever:
    """Embeds the pack once (cached by pack hash) and returns the top-k passages for a query."""

    def __init__(self, passages: list[dict]) -> None:
        self.passages = passages
        self.srv = Server(model_path(EMBEDDER), ["--embeddings", "--pooling", "last", "--ubatch-size", "2048"], ctx=8192, parallel=4)
        cache = WORK / f"pack-embeddings-{pack_hash()[:12]}.json"
        if cache.exists():
            self.vecs = json.loads(cache.read_text())
        else:
            docs = [f"{p['section']}. {p['text']}" for p in passages]
            self.vecs = embed_all(docs, self.srv)
            cache.write_text(json.dumps(self.vecs))

        self.bm25 = BM25([f"{p['section']} {p['text']}" for p in passages])

    def search(self, query: str, k: int) -> list[tuple[dict, float]]:
        """Hybrid search: embeddings (meaning) fused with BM25 (exact terms) by reciprocal rank."""
        q = embed_all([QUERY_PREFIX + query], self.srv)[0]
        dense = sorted(range(len(self.passages)), key=lambda i: -cosine(q, self.vecs[i]))
        sparse = self.bm25.rank(query)
        fused: dict[int, float] = {}
        for ranking in (dense, sparse):
            for r, i in enumerate(ranking[:50]):
                fused[i] = fused.get(i, 0.0) + 1.0 / (60 + r)
        best = sorted(fused, key=lambda i: -fused[i])[:k]
        return [(self.passages[i], round(fused[i], 5)) for i in best]

    def close(self) -> None:
        self.srv.close()


STOP = set("a an and are as at be by can do does for from how in is it its of on or should that the this to under what when which who why with".split())


def tokens(s: str) -> list[str]:
    return [w for w in re.findall(r"[a-z0-9]+(?:[.-][a-z0-9]+)*", s.lower()) if w not in STOP]


class BM25:
    def __init__(self, docs: list[str], k1: float = 1.4, b: float = 0.75) -> None:
        self.docs = [tokens(d) for d in docs]
        self.avg = sum(map(len, self.docs)) / max(1, len(self.docs))
        self.k1, self.b = k1, b
        df: dict[str, int] = {}
        for d in self.docs:
            for w in set(d):
                df[w] = df.get(w, 0) + 1
        n = len(self.docs)
        self.idf = {w: math.log(1 + (n - f + 0.5) / (f + 0.5)) for w, f in df.items()}

    def rank(self, query: str) -> list[int]:
        q = tokens(query)
        scores = []
        for i, d in enumerate(self.docs):
            s = 0.0
            for w in q:
                tf = d.count(w)
                if tf:
                    s += self.idf.get(w, 0) * tf * (self.k1 + 1) / (tf + self.k1 * (1 - self.b + self.b * len(d) / self.avg))
            scores.append(s)
        return sorted(range(len(self.docs)), key=lambda i: -scores[i])


# Two wordings of the same instructions, to check a model isn't fragile to phrasing.
PROMPTS = {
    "A": (
        "You are a computerised-system validation assistant for regulated (GxP) work.\n"
        "Answer the question using ONLY the numbered sources below. Do not use outside knowledge.\n"
        "Put the source number in square brackets after every sentence that states a fact, e.g. [2].\n"
        f"If the sources do not contain the answer, reply with exactly this sentence and nothing else: {REFUSAL}\n"
        "Be concise: at most 150 words."
    ),
    "B": (
        "Role: assistant for GxP computerised systems and data integrity.\n"
        "Rules:\n"
        "1. Use only the information in the sources provided; never rely on memory.\n"
        "2. Every factual sentence must end with a citation such as [1] or [1][3].\n"
        f"3. When the sources don't answer the question, respond only with: {REFUSAL}\n"
        "4. Keep the answer under 150 words."
    ),
}


def build_messages(question: str, passages: list[dict], prompt: str) -> list[dict]:
    src = "\n\n".join(f"[{i + 1}] ({p['section']} — {p['title']})\n{p['text']}" for i, p in enumerate(passages))
    return [
        {"role": "system", "content": PROMPTS[prompt]},
        {"role": "user", "content": f"Sources:\n\n{src}\n\nQuestion: {question}"},
    ]


def norm(s: str) -> str:
    return re.sub(r"\s+", " ", s.lower().replace("’", "'").replace("“", '"').replace("”", '"')).strip()
