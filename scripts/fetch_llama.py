"""Fetch the pinned llama.cpp build for one target and stage only what llama-server needs.

    python scripts/fetch_llama.py                    # host target -> hub/app/resources/llama/
    python scripts/fetch_llama.py --target x86_64-unknown-linux-gnu --out /tmp/llama

The release archive is verified against runtime/llama.lock.json before extraction. From it we keep
llama-server, its shared libraries (including every CPU-variant backend, which ggml picks at
runtime) and the licence. Library symlinks are replaced by a regular file under the name the
loader looks up (e.g. libllama.0.dylib, libllama.so.0), because installers and code signing handle
plain files more reliably than symlinks.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import stat
import sys
import tarfile
import zipfile
from pathlib import Path, PurePosixPath

from _common import CACHE, ROOT, download, host_target

LOCK = ROOT / "runtime" / "llama.lock.json"
DEFAULT_OUT = ROOT / "hub" / "app" / "resources" / "llama"

SHARED_LIB = re.compile(r"\.(dll|dylib)$|\.so(\.\d+)*$")
# Other tools' implementation libraries: large and unused by llama-server.
OTHER_TOOL_IMPL = re.compile(r"^(lib)?llama-(?!server-impl)[a-z0-9-]+-impl\.")


def wanted(name: str) -> bool:
    if name in ("llama-server", "llama-server.exe") or name.startswith("LICENSE"):
        return True
    return bool(SHARED_LIB.search(name)) and not OTHER_TOOL_IMPL.match(name)


def stage_tar(archive: Path, out: Path) -> list[str]:
    with tarfile.open(archive) as tf:
        members = {PurePosixPath(m.name).name: m for m in tf.getmembers() if m.isfile() or m.issym()}
        symlink_targets = {
            name: PurePosixPath(m.linkname).name for name, m in members.items() if m.issym()
        }
        written = []
        for name, m in sorted(members.items()):
            if not wanted(name):
                continue
            if m.issym():
                target = members.get(symlink_targets[name])
                if target is None or not target.isfile():
                    continue  # second-level alias such as libllama.dylib -> libllama.0.dylib
                src = target
            else:
                if name in symlink_targets.values():
                    continue  # fully versioned file; staged under its loader name instead
                src = m
            data = tf.extractfile(src)
            assert data is not None
            dest = out / name
            with dest.open("wb") as f:
                shutil.copyfileobj(data, f)
            if src.mode & stat.S_IXUSR or name == "llama-server":
                dest.chmod(0o755)
            written.append(name)
        return written


def stage_zip(archive: Path, out: Path) -> list[str]:
    written = []
    with zipfile.ZipFile(archive) as zf:
        for info in zf.infolist():
            name = PurePosixPath(info.filename).name
            if info.is_dir() or not wanted(name):
                continue
            with zf.open(info) as src, (out / name).open("wb") as dest:
                shutil.copyfileobj(src, dest)
            written.append(name)
    return sorted(written)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--target", default=host_target())
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = ap.parse_args()

    lock = json.loads(LOCK.read_text())
    entry = lock["targets"].get(args.target)
    if entry is None:
        sys.exit(f"no pinned llama.cpp build for {args.target}; see {LOCK.relative_to(ROOT)}")
    url = f"https://github.com/{lock['project']}/releases/download/{lock['build']}/{entry['asset']}"
    archive = download(url, CACHE / "llama" / entry["asset"], entry["sha256"])

    if args.out.exists():
        shutil.rmtree(args.out)
    args.out.mkdir(parents=True)
    files = stage_zip(archive, args.out) if archive.suffix == ".zip" else stage_tar(archive, args.out)
    server = "llama-server.exe" if "windows" in args.target else "llama-server"
    if server not in files:
        sys.exit(f"{server} not found in {entry['asset']}")
    stamp = {"build": lock["build"], "version": lock["version"], "target": args.target, "files": files}
    (args.out / "llama.json").write_text(json.dumps(stamp, indent=2) + "\n")
    size = sum((args.out / f).stat().st_size for f in files) / 2**20
    print(f"staged llama.cpp {lock['build']} for {args.target}: {len(files)} files, {size:.1f} MB -> {args.out}")


if __name__ == "__main__":
    main()
