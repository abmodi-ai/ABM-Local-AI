"""Shared helpers for the fetch scripts: host target detection, resumable download, sha256.

Standard library only (Python 3.9+) so the scripts run on a fresh Windows, macOS or Linux machine.
"""

from __future__ import annotations

import hashlib
import platform
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".cache"


def host_target() -> str:
    system = platform.system()
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(machine)
    if arch is None:
        sys.exit(f"unsupported CPU architecture: {machine}")
    if system == "Windows":
        return f"{arch}-pc-windows-msvc"
    if system == "Darwin":
        return f"{arch}-apple-darwin"
    if system == "Linux":
        return f"{arch}-unknown-linux-gnu"
    sys.exit(f"unsupported OS: {system}")


def sha256_file(path: Path, chunk: int = 8 * 2**20) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while b := f.read(chunk):
            h.update(b)
    return h.hexdigest()


def download(url: str, dest: Path, sha256: str, size: int | None = None) -> Path:
    """Download url to dest (resuming a partial file) and verify its sha256. Idempotent."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.exists() and sha256_file(dest) == sha256:
        print(f"cached   {dest.name}")
        return dest
    part = dest.with_name(dest.name + ".part")
    have = part.stat().st_size if part.exists() else 0
    req = urllib.request.Request(url, headers={"User-Agent": "abm-local-ai-fetch"})
    if have:
        req.add_header("Range", f"bytes={have}-")
    with urllib.request.urlopen(req, timeout=60) as r:  # noqa: S310 - fixed https hosts from lock files
        if have and r.status != 206:  # server ignored the range; start over
            have = 0
        total = size or (int(r.headers.get("Content-Length", 0)) + have)
        with part.open("ab" if have else "wb") as f:
            done = have
            last = -1
            while chunk := r.read(4 * 2**20):
                f.write(chunk)
                done += len(chunk)
                pct = int(done * 100 / total) if total else 0
                if pct != last and pct % 5 == 0:
                    print(f"\r{dest.name}: {pct:3d}%", end="", flush=True)
                    last = pct
    print()
    digest = sha256_file(part)
    if digest != sha256:
        part.unlink()
        sys.exit(f"sha256 mismatch for {dest.name}: expected {sha256}, got {digest}")
    part.replace(dest)
    print(f"verified {dest.name}")
    return dest
