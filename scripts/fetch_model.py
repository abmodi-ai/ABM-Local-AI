"""Download a model listed in bench/models.json into .cache/models/ and verify its sha256.

    python scripts/fetch_model.py small standard     # the M0 candidates
    python scripts/fetch_model.py ci-tiny            # 105 MB model used by CI smoke tests

Prints the local path of each model. Downloads resume if interrupted.
"""

from __future__ import annotations

import argparse
import json
import sys

from _common import CACHE, ROOT, download

MODELS = ROOT / "bench" / "models.json"


def main() -> None:
    catalog = json.loads(MODELS.read_text())["models"]
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("names", nargs="+", choices=sorted(catalog))
    args = ap.parse_args()
    for name in args.names:
        m = catalog[name]
        if m["license"] not in ("Apache-2.0", "MIT"):
            sys.exit(f"{name}: licence {m['license']} is not permitted")
        url = f"https://huggingface.co/{m['repo']}/resolve/main/{m['file']}"
        path = download(url, CACHE / "models" / m["file"], m["sha256"], m["size"])
        print(path)


if __name__ == "__main__":
    main()
