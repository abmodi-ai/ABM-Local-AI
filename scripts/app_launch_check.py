"""Launch the installed hub app, confirm it runs its bundled llama-server, then hard-kill the app
and confirm llama-server dies with it. Used by CI on Windows, macOS and Linux (needs psutil).

    python scripts/app_launch_check.py --app "<path to installed hub executable>" --model <gguf> --root "<install dir>"
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

import psutil


def servers() -> list[psutil.Process]:
    out = []
    for p in psutil.process_iter(["name", "exe"]):
        if (p.info["name"] or "").lower().startswith("llama-server"):
            out.append(p)
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", type=Path, required=True)
    ap.add_argument("--model", type=Path, required=True)
    ap.add_argument("--root", type=Path, required=True, help="llama-server must run from under this directory")
    ap.add_argument("--timeout", type=float, default=90)
    args = ap.parse_args()

    if servers():
        sys.exit("FAIL: a llama-server is already running before the test")
    env = {**os.environ, "ABM_MODEL": str(args.model.resolve())}
    app = subprocess.Popen([str(args.app)], env=env)
    root = str(args.root.resolve()).lower()
    found = None
    deadline = time.time() + args.timeout
    while time.time() < deadline and found is None:
        if app.poll() is not None:
            sys.exit(f"FAIL: the app exited early with code {app.returncode}")
        for p in servers():
            try:
                exe = (p.exe() or "").lower()
                parents = [a.pid for a in p.parents()]
            except psutil.Error:
                continue
            if exe.startswith(root) and app.pid in parents:
                found = p
        time.sleep(0.5)
    if found is None:
        app.kill()
        sys.exit(f"FAIL: no llama-server from {args.root} started by the app within {args.timeout:.0f} s; running: "
                 f"{[(p.pid, p.info['exe']) for p in servers()]}")
    print(f"ok   app pid {app.pid} runs bundled llama-server pid {found.pid}: {found.exe()}")

    listening = []
    while time.time() < deadline and not listening:  # the port opens once the model has loaded
        listening = [c for c in found.net_connections(kind="inet") if c.status == psutil.CONN_LISTEN]
        time.sleep(0.5)
    bad = [c.laddr for c in listening if c.laddr.ip not in ("127.0.0.1", "::1")]
    if bad or not listening:
        app.kill()
        sys.exit(f"FAIL: llama-server listening on {[c.laddr for c in listening]} (expected loopback only)")
    print(f"ok   listening on loopback only: {[f'{c.laddr.ip}:{c.laddr.port}' for c in listening]}")

    app.kill()  # hard kill: TerminateProcess / SIGKILL, no clean shutdown path
    app.wait()
    try:
        found.wait(timeout=15)
    except psutil.TimeoutExpired:
        found.kill()
        sys.exit(f"FAIL: llama-server pid {found.pid} survived the app being killed")
    print("ok   llama-server died with the app after a hard kill")


if __name__ == "__main__":
    main()
