# /// script
# requires-python = ">=3.10"
# dependencies = ["pdfminer.six==20250506"]
# ///
"""Build the evaluation knowledge pack: download the sources in eval/sources.json, extract their
text and split it into citable passages.

    uv run eval/build_pack.py            # -> .cache/eval/pack/{passages.jsonl, manifest.json}

Passage ids are stable and human-readable so questions can say which passage must be cited:
  part11:11.10(e)      21 CFR 11.10, paragraph (e)
  annex11:4.4          Annex 11 clause 4.4
  mhra-di:6.13#1       MHRA guidance, first window of section 6.13
  fda-di:p12           FDA data-integrity Q&A, window 12
Structured sources (Part 11 XML, Annex 11 clauses) are split on their own structure; the others
are split into ~180-word windows at paragraph boundaries, each labelled with its nearest heading.
"""

from __future__ import annotations

import hashlib
import html
import json
import re
import urllib.request
from io import BytesIO
from pathlib import Path

from pdfminer.high_level import extract_text

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / "eval" / "sources.json"
OUT = ROOT / ".cache" / "eval" / "pack"
RAW = ROOT / ".cache" / "eval" / "raw"
WINDOW_WORDS = 180
MAX_WORDS = 260


def fetch(src: dict) -> bytes:
    RAW.mkdir(parents=True, exist_ok=True)
    cached = RAW / src["id"]
    if cached.exists():
        return cached.read_bytes()
    req = urllib.request.Request(src["url"], headers={"User-Agent": "Mozilla/5.0 abm-local-ai-eval", "Accept-Encoding": "gzip"})
    with urllib.request.urlopen(req, timeout=60) as r:  # noqa: S310 - fixed URLs from sources.json
        data = r.read()
        if r.headers.get("Content-Encoding") == "gzip":
            import gzip

            data = gzip.decompress(data)
    cached.write_bytes(data)
    return data


def clean(s: str) -> str:
    s = s.replace("­", "").replace("ﬁ", "fi").replace("ﬂ", "fl")
    return re.sub(r"\s+", " ", s).strip()


def windows(paras: list[str], size: int = WINDOW_WORDS) -> list[str]:
    """Group paragraphs into windows of about `size` words, splitting very long paragraphs."""
    out, cur, n = [], [], 0
    for p in paras:
        words = p.split()
        while len(words) > MAX_WORDS:  # a single huge paragraph: cut it
            head, words = words[:size], words[size:]
            if cur:
                out.append(" ".join(cur))
                cur, n = [], 0
            out.append(" ".join(head))
        if n + len(words) > size and cur:
            out.append(" ".join(cur))
            cur, n = [], 0
        cur.append(" ".join(words))
        n += len(words)
    if cur:
        out.append(" ".join(cur))
    return out


def part11(data: bytes, src: dict) -> list[dict]:
    x = data.decode("utf-8")
    passages = []
    for m in re.finditer(r'<DIV8 N="([^"]+)"[^>]*>(.*?)</DIV8>', x, re.S):
        num, body = m.group(1), m.group(2)
        head = clean(html.unescape(re.sub(r"<[^>]+>", " ", re.search(r"<HEAD>(.*?)</HEAD>", body, re.S).group(1))))
        title = head.replace("§", "").strip()
        paras = [clean(html.unescape(re.sub(r"<[^>]+>", " ", p))) for p in re.findall(r"<P>(.*?)</P>", body, re.S)]
        paras = [p for p in paras if p]
        # Split on lettered paragraphs "(a) ..." so citations can be precise.
        groups: list[tuple[str, list[str]]] = []
        for p in paras:
            lm = re.match(r"^\(([a-z])\)", p)
            if lm or not groups:
                groups.append((f"({lm.group(1)})" if lm else "", [p]))
            else:
                groups[-1][1].append(p)
        small = sum(len(" ".join(g[1]).split()) for g in groups) <= MAX_WORDS
        if small:
            groups = [("", [p for _, g in groups for p in g])]
        for label, ps in groups:
            for i, text in enumerate(windows(ps)):
                pid = f"part11:{num}{label}" + (f"#{i + 1}" if i else "")
                passages.append({"id": pid, "section": f"21 CFR {title}{' ' + label if label else ''}", "text": text})
    return passages


ANNEX11_CLAUSE = re.compile(r"^(\d{1,2}(?:\.\d{1,2})?)\.?(?:\s+(.*))?$")


def annex11(data: bytes, src: dict) -> list[dict]:
    t = extract_text(BytesIO(data))
    t = t[t.find("Principle") :]
    t = t[: t.find("Glossary")] if "Glossary" in t else t
    lines = [clean(l) for l in t.splitlines()]
    passages, cur_id, cur_title, buf, titles = [], "Principle", "Principle", [], {}
    phase_heads = {"General", "Project Phase", "Operational Phase"}

    def flush():
        text = clean(" ".join(buf))
        if text:
            for i, w in enumerate(windows([text])):
                passages.append({"id": f"annex11:{cur_id}" + (f"#{i + 1}" if i else ""), "section": f"Annex 11 {cur_title}", "text": w})

    for line in lines:
        if not line or line in phase_heads or line == "Principle":
            continue
        m = ANNEX11_CLAUSE.match(line)
        if m and (not buf or len(m.group(1)) <= 5):
            num, rest = m.group(1), m.group(2) or ""
            top = num.split(".")[0]
            if 1 <= int(top) <= 17:
                # The PDF sometimes emits a clause's first line before its number: lines after the
                # previous clause's last full stop belong to the new clause.
                carry: list[str] = []
                if not rest and buf:
                    ends = [i for i, l in enumerate(buf) if l.rstrip().endswith((".", ":", ";"))]
                    cut = ends[-1] + 1 if ends else 0
                    carry, buf = buf[cut:], buf[:cut]
                flush()
                buf = carry
                cur_id = num
                if "." not in num:  # clause heading like "4. Validation"; text may follow on next lines
                    titles[top] = rest.strip() or titles.get(top, "")
                    cur_title = f"§{num} {titles[top]}".strip()
                    if rest and len(rest.split()) > 6:  # heading and text on one line
                        buf.append(rest)
                else:
                    cur_title = f"§{num} ({titles.get(top, '')})"
                    buf.append(rest)
                continue
        if cur_id.isdigit() and not titles.get(cur_id) and len(line.split()) <= 6:
            titles[cur_id] = line  # heading on its own line after "1."
            cur_title = f"§{cur_id} {line}"
            continue
        buf.append(line)
    flush()
    return passages


HEADING = re.compile(r"^((?:\d+\.\s?)+\d*\.?|[IVX]+\.|[A-H]\.)\s+([A-Z‘“\"][^.]{2,100})$")


def generic_pdf(data: bytes, src: dict) -> list[dict]:
    t = extract_text(BytesIO(data))
    start = src.get("body_starts_at")
    if start:  # skip the table of contents
        hits = list(re.finditer(start["pattern"], t))
        t = t[: 400] + "\n\n" + t[hits[start["occurrence"]].start() :]
    # Drop the table of contents (dotted leaders) and page headers/footers.
    lines = [clean(l) for l in t.splitlines()]
    lines = [l for l in lines if not re.search(r"\.{5,}\s*\d+$", l) and not re.match(r"^(Page \d+ of \d+|\d+)$", l)]
    lines = [l for l in lines if not l.startswith("MHRA GXP Data Integrity Guidance and Definitions; Revision")]
    text = "\n".join(lines)
    paras = [clean(p) for p in re.split(r"\n\s*\n", text)]
    paras = [p for p in paras if len(p.split()) >= 3]
    # Skip the table of contents: if the first heading appears again later, start there.
    heads = [i for i, p in enumerate(paras) if len(p) < 110 and HEADING.match(p)]
    if heads:
        first = paras[heads[0]]
        again = [i for i in heads[1:] if paras[i] == first]
        if again:
            paras = paras[: heads[0]][:3] + paras[again[-1] :]  # keep the title lines
    sections: list[tuple[str, list[str]]] = [("Front matter", [])]
    for p in paras:
        m = HEADING.match(p) if len(p) < 110 and src.get("headings", True) else None
        if m:
            sections.append((clean(p), []))
        else:
            sections[-1][1].append(p)
    passages, n = [], 0
    for head, ps in sections:
        num = re.match(r"^((?:\d+\.\s?)+\d*|[IVX]+\.|[A-H]\.)", head)
        key = re.sub(r"[\s.]+$", "", num.group(1)).replace(" ", "") if num else None
        for i, w in enumerate(windows(ps)):
            n += 1
            pid = f"{src['id']}:{key}" + (f"#{i + 1}" if i else "") if key else f"{src['id']}:p{n}"
            label = head if src.get("headings", True) else f"part {n}"
            passages.append({"id": pid, "section": label, "text": w})
    # Guarantee unique ids (a heading number can repeat, e.g. in a glossary).
    seen: dict[str, int] = {}
    for p in passages:
        if p["id"] in seen:
            seen[p["id"]] += 1
            p["id"] = f"{p['id']}~{seen[p['id']]}"
        else:
            seen[p["id"]] = 1
    return passages


PARSERS = {"ecfr-xml": part11, "annex11-pdf": annex11, "pdf": generic_pdf}


def main() -> None:
    reg = json.loads(SOURCES.read_text())
    OUT.mkdir(parents=True, exist_ok=True)
    all_passages, manifest = [], []
    for src in reg["sources"]:
        data = fetch(src)
        ps = PARSERS[src["format"]](data, src)
        for p in ps:
            p.update({"source": src["id"], "title": src["title"], "licence": src["licence"], "url": src["url"]})
            p["words"] = len(p["text"].split())
        all_passages += ps
        manifest.append({"id": src["id"], "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data), "passages": len(ps)})
        print(f"{src['id']:14} {len(ps):3} passages  sha256 {manifest[-1]['sha256'][:16]}")
    ids = [p["id"] for p in all_passages]
    assert len(ids) == len(set(ids)), "duplicate passage ids"
    with (OUT / "passages.jsonl").open("w") as f:
        for p in all_passages:
            f.write(json.dumps(p, ensure_ascii=False) + "\n")
    pack_hash = hashlib.sha256((OUT / "passages.jsonl").read_bytes()).hexdigest()
    (OUT / "manifest.json").write_text(json.dumps({"pack_sha256": pack_hash, "sources": manifest}, indent=2) + "\n")
    print(f"{len(all_passages)} passages -> {OUT.relative_to(ROOT)} (pack {pack_hash[:12]})")


if __name__ == "__main__":
    main()
