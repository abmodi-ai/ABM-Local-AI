"""Turn graded answers into the side-by-side report and apply the agreed decision rule.

    uv run eval/report.py            # -> docs/eval/<questions>-<date>.md and prints it

Decision rule (agreed before any results were seen, see docs/validation-model-plan.md):
  Hard gates, all required:
    citation check passes on >= 95% of answers that make claims
    answers with a made-up (unsupported) claim <= 5%
    200-token answer <= 20 s on the 12 GB reference PC (Mac CPU proxy until that PC is measured)
  Weighted score (primary numbers use the retrieved mode, averaged over both prompt wordings):
    correct facts 40 · no made-up claims 25 · correct refusals 15 · fine-tuning gain 10 · speed 10
    Fine-tuning gain is not measured yet, so the other weights are rescaled to sum to 100.
    Speed: 1.0 at <= 10 s for the 200-token answer, falling linearly to 0 at 20 s.
  A difference of 3 points or less is a tie; a tie goes to Granite 4.2 3B (faster, documented data).
"""

from __future__ import annotations

import json
import random
import statistics
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from common import CONTESTANTS, ROOT, WORK  # noqa: E402

WEIGHTS = {"facts": 40, "no_made_up": 25, "refusal": 15, "speed": 10}  # fine-tune gain (10) pending
TIE_POINTS = 3.0
TIE_WINNER = "granite-4.2-3b"


def speed_from_bench() -> dict[str, float]:
    """200-token answer time (s), CPU only, from the most recent bench run that has both models."""
    best: dict[str, float] = {}
    for f in sorted((ROOT / "bench" / "results").glob("*.json")):
        d = json.loads(f.read_text())
        if "cpu_only" in d.get("args", {}) and d["args"]["cpu_only"] == "True":
            for r in d["results"]:
                best[r["pack"]] = r["explain_200_s_median"]
    return best


def mean(xs):
    xs = [x for x in xs if x is not None]
    return statistics.mean(xs) if xs else None


def pct(x):
    return "—" if x is None else f"{100 * x:.0f}%"


def summarise(rows: list[dict], questions: dict) -> dict:
    ans = [r for r in rows if questions[r["qid"]]["answerable"]]
    una = [r for r in rows if not questions[r["qid"]]["answerable"]]
    answered = [r for r in ans if not r["refused"]]
    graded = [r for r in answered if "claims" in r]
    facts = [(sum(r["facts_present"]) / len(r["facts_present"])) if "facts_present" in r and r["facts_present"] else 0.0
             for r in ans if not r["refused"]] + [0.0 for r in ans if r["refused"]]
    claims = [c for r in graded for c in r["claims"]]
    making_claims = answered + [r for r in una if not r["refused"]]
    return {
        "n": len(rows),
        "facts": mean(facts),
        "made_up_answers": mean([not all(c["supported"] for c in r["claims"]) for r in graded]),
        "unsupported_claims": mean([not c["supported"] for c in claims]),
        "refusal": mean([r["refused"] for r in una]),
        "exact_refusal": mean([r["exact_refusal"] for r in una]),
        "false_refusal": mean([r["refused"] for r in ans]),
        "citation_pass": mean([r["citation_pass"] for r in making_claims]),
        "cited_required": mean([r["cited_required"] for r in answered]),
        "lexical_support": mean([r["lexical_support"] for r in answered]),
        "truncated": mean([r["truncated"] for r in rows]),
        "degenerate": mean([r["degenerate"] for r in rows]),
        "seconds_metal": mean([r["seconds"] for r in rows]),
        "grader_failures": sum(1 for r in answered if "claims" not in r),
    }


def per_question(rows: list[dict], questions: dict) -> dict[str, float]:
    """Per-question composite on the weighted score's quality parts, for paired bootstrap."""
    out: dict[str, list[float]] = {}
    for r in rows:
        q = questions[r["qid"]]
        if q["answerable"]:
            f = 0.0 if r["refused"] else (sum(r.get("facts_present", [])) / max(1, len(q["key_facts"])))
            ok = 1.0 if r["refused"] else float(all(c["supported"] for c in r.get("claims", [{"supported": False}])))
            s = (WEIGHTS["facts"] * f + WEIGHTS["no_made_up"] * ok) / (WEIGHTS["facts"] + WEIGHTS["no_made_up"])
        else:
            s = float(r["refused"])
        out.setdefault(r["qid"], []).append(s)
    return {k: statistics.mean(v) for k, v in out.items()}


def bootstrap_diff(a: dict, b: dict, n: int = 5000) -> tuple[float, float, float]:
    keys = sorted(set(a) & set(b))
    rng = random.Random(3)
    diffs = []
    for _ in range(n):
        ks = [rng.choice(keys) for _ in keys]
        diffs.append(statistics.mean(a[k] - b[k] for k in ks))
    diffs.sort()
    return statistics.mean(a[k] - b[k] for k in keys), diffs[int(0.025 * n)], diffs[int(0.975 * n)]


def main() -> None:
    meta = json.loads((WORK / "runs" / "meta.json").read_text())
    questions = {json.loads(l)["id"]: json.loads(l) for l in (ROOT / meta["questions"]).open()}
    graded = [json.loads(l) for l in (WORK / "graded.jsonl").open()]
    speed = speed_from_bench()
    keys = [k for k in CONTESTANTS if any(g["contestant"] == k for g in graded)]

    table, primary, pq = {}, {}, {}
    for k in keys:
        rows = [g for g in graded if g["contestant"] == k]
        for mode in ("retrieved", "gold"):
            for prompt in ("A", "B"):
                table[(k, mode, prompt)] = summarise([r for r in rows if r["mode"] == mode and r["prompt"] == prompt], questions)
        primary[k] = summarise([r for r in rows if r["mode"] == "retrieved"], questions)
        pq[k] = per_question([r for r in rows if r["mode"] == "retrieved"], questions)

    scores, gates = {}, {}
    total_w = sum(WEIGHTS.values())
    for k in keys:
        p = primary[k]
        t200 = speed.get(k)
        sp = None if t200 is None else max(0.0, min(1.0, (20 - t200) / 10))
        parts = {"facts": p["facts"] or 0, "no_made_up": 1 - (p["made_up_answers"] or 0), "refusal": p["refusal"] or 0, "speed": sp or 0}
        scores[k] = (100 * sum(WEIGHTS[x] * parts[x] for x in WEIGHTS) / total_w, parts, t200)
        gates[k] = {
            "citation check ≥ 95%": (p["citation_pass"] or 0) >= 0.95,
            "made-up answers ≤ 5%": (p["made_up_answers"] or 0) <= 0.05,
            "200 tokens ≤ 20 s (CPU proxy)": t200 is not None and t200 <= 20,
        }

    name = lambda k: CONTESTANTS[k]["name"]  # noqa: E731
    L = [f"# Base-model bake-off: {', '.join(name(k) for k in keys)}", "",
         f"Questions: `{meta['questions']}` ({len(questions)} questions, {sum(q['answerable'] for q in questions.values())} answerable) · "
         f"knowledge pack `{meta['pack']}` · {meta['k']} passages per question · generated {meta['time']} · "
         "grader gpt-oss-120b (blind, shuffled)", ""]
    if "pilot" in meta["questions"]:
        L += ["> **Pilot set, not decision-grade.** These questions were written from the public sources to prove the test "
              "works. The model decision should be made on the subject-matter expert's 300-question set, with the expert "
              "reviewing the grader's sample (`.cache/eval/review/sme_sample.jsonl`).", ""]
    L += ["## Result", ""]
    ranked = sorted(keys, key=lambda k: -scores[k][0])
    for k in ranked:
        s, parts, t200 = scores[k]
        g = gates[k]
        L.append(f"- **{name(k)}: {s:.1f} / 100** · gates: " + ", ".join(f"{'✅' if ok else '❌'} {n}" for n, ok in g.items()))
    if len(ranked) == 2:
        a, b = ranked
        gap = scores[a][0] - scores[b][0]
        passing = [k for k in ranked if all(gates[k].values())]
        if len(passing) == 1:
            verdict = f"**{name(passing[0])}**: the only model that passes every hard gate."
        elif not passing:
            verdict = "**Neither model passes every hard gate.** Pick by the score only for further work, not for release."
            verdict += f" Higher score: {name(a)}." if gap > TIE_POINTS else f" Scores are within {TIE_POINTS:.0f} points: tie → {name(TIE_WINNER)}."
        elif gap <= TIE_POINTS:
            verdict = f"**Tie ({gap:.1f} points) → {name(TIE_WINNER)}**, per the agreed tie rule."
        else:
            verdict = f"**{name(a)}** by {gap:.1f} points."
        d, lo, hi = bootstrap_diff(pq[a], pq[b])
        L += ["", f"Decision under the agreed rule: {verdict}", "",
              f"Paired per-question quality difference ({name(a)} − {name(b)}): {100 * d:+.1f} points, "
              f"95% bootstrap interval {100 * lo:+.1f} to {100 * hi:+.1f}"
              + (" (includes zero: not a clear difference on this set)." if lo <= 0 <= hi else ".")]

    L += ["", "## Scores (retrieved passages, both prompt wordings)", "",
          "| | " + " | ".join(name(k) for k in keys) + " |", "|---|" + "---|" * len(keys)]
    rows = [
        ("Correct facts (answerable)", lambda p, k: pct(p["facts"])),
        ("Answers with a made-up claim", lambda p, k: pct(p["made_up_answers"])),
        ("Unsupported claims (all claims)", lambda p, k: pct(p["unsupported_claims"])),
        ("Correct refusals (unanswerable)", lambda p, k: pct(p["refusal"])),
        ("…using the exact refusal sentence", lambda p, k: pct(p["exact_refusal"])),
        ("Wrong refusals (answerable)", lambda p, k: pct(p["false_refusal"])),
        ("Citation check passes", lambda p, k: pct(p["citation_pass"])),
        ("Cited a required passage", lambda p, k: pct(p["cited_required"])),
        ("Answers cut off at the token limit", lambda p, k: pct(p["truncated"])),
        ("…of which looping (repeating a line)", lambda p, k: pct(p["degenerate"])),
        ("200-token answer, CPU 6 threads (s)", lambda p, k: f"{scores[k][2]}" if scores[k][2] else "—"),
        ("Seconds per answer (Metal, this Mac)", lambda p, k: f"{p['seconds_metal']:.1f}"),
        ("Weighted score", lambda p, k: f"**{scores[k][0]:.1f}**"),
    ]
    for label, fn in rows:
        L.append(f"| {label} | " + " | ".join(fn(primary[k], k) for k in keys) + " |")

    L += ["", "## By mode and prompt wording", "",
          "Gold = the required passages are guaranteed to be present (tests reading); retrieved = real search (tests the whole system).", "",
          "| Model | Mode | Prompt | Correct facts | Made-up answers | Correct refusals | Wrong refusals | Citation pass |", "|---|---|---|---|---|---|---|---|"]
    for (k, mode, prompt), p in table.items():
        L.append(f"| {name(k)} | {mode} | {prompt} | {pct(p['facts'])} | {pct(p['made_up_answers'])} | {pct(p['refusal'])} | {pct(p['false_refusal'])} | {pct(p['citation_pass'])} |")

    L += ["", "## By question category (retrieved)", "", "| Category | " + " | ".join(name(k) for k in keys) + " |", "|---|" + "---|" * len(keys)]
    for cat in sorted({q["category"] for q in questions.values()}):
        cells = []
        for k in keys:
            rs = [g for g in graded if g["contestant"] == k and g["mode"] == "retrieved" and questions[g["qid"]]["category"] == cat]
            p = summarise(rs, questions)
            cells.append(pct(p["refusal"]) + " refused" if cat == "unanswerable" else pct(p["facts"]) + " facts")
        L.append(f"| {cat} | " + " | ".join(cells) + " |")

    L += ["", "## Notes", "",
          "- Weighted score excludes the fine-tuning-gain criterion (10 points) until pilot LoRAs are trained; weights are rescaled.",
          "- Speed comes from `bench/bench.py` CPU-only runs on the Mac, a proxy until the 12 GB Windows reference PC is measured.",
          f"- Grader failures (answers the grader couldn't score): " + ", ".join(f"{name(k)} {primary[k]['grader_failures']}" for k in keys) + ".",
          "- Raw answers: `.cache/eval/runs/`; graded: `.cache/eval/graded.jsonl`; expert sample: `.cache/eval/review/sme_sample.jsonl`."]
    md = "\n".join(L) + "\n"
    out_dir = ROOT / "docs" / "eval"
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / f"{Path(meta['questions']).stem}-{time.strftime('%Y-%m-%d')}.md"
    out.write_text(md)
    print(md)
    print(f"saved {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
