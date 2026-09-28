"""Generate answers from each contestant model for every question, in two modes and two prompt
wordings.

    uv run eval/run.py                      # all contestants, questions/pilot.jsonl
    uv run eval/run.py --questions eval/questions/sme.jsonl --k 4

Modes:
  gold       the passages the question must cite, topped up with retrieved passages to k
             (tests reading and reasoning with the right evidence present)
  retrieved  the top-k retrieved passages only (tests the whole system as users will see it)
Unanswerable questions get retrieved passages in both modes; the right answer is the refusal.

Output: .cache/eval/runs/<contestant>.jsonl (one line per question x mode x prompt).
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

from common import CATALOG, CONTESTANTS, PROMPTS, ROOT, WORK, Retriever, Server, build_messages, load_passages, model_path, pack_hash


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--questions", type=Path, default=ROOT / "eval" / "questions" / "pilot.jsonl")
    ap.add_argument("--k", type=int, default=4, help="passages per question")
    ap.add_argument("--only", nargs="*", default=list(CONTESTANTS))
    args = ap.parse_args()

    questions = [json.loads(l) for l in args.questions.open()]
    passages = load_passages()
    by_id = {p["id"]: p for p in passages}

    # Retrieval once, shared by every contestant, so they all see identical evidence.
    retr = Retriever(passages)
    contexts = {}
    for q in questions:
        hits = retr.search(q["question"], args.k + len(q["must_cite"]))
        retrieved = [p for p, _ in hits][: args.k]
        gold = [by_id[i] for i in q["must_cite"]]
        topped = gold + [p for p in retrieved if p["id"] not in q["must_cite"]]
        contexts[q["id"]] = {
            "retrieved": retrieved,
            "gold": topped[: max(args.k, len(gold))] if q["answerable"] else retrieved,
            "retrieval_scores": [s for _, s in hits][: args.k],
        }
    retr.close()
    recall = [len(set(q["must_cite"]) & {p["id"] for p in contexts[q["id"]]["retrieved"]}) > 0 for q in questions if q["answerable"]]
    print(f"retrieval: a required passage is in the top {args.k} for {sum(recall)}/{len(recall)} answerable questions")

    out_dir = WORK / "runs"
    out_dir.mkdir(parents=True, exist_ok=True)
    meta = {"questions": str(args.questions.relative_to(ROOT)), "k": args.k, "pack": pack_hash()[:12], "time": time.strftime("%Y-%m-%dT%H:%M:%S")}
    (out_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    (out_dir / "contexts.json").write_text(json.dumps({k: {m: [p["id"] for p in v[m]] for m in ("gold", "retrieved")} for k, v in contexts.items()}, indent=1))

    for key in args.only:
        c = CONTESTANTS[key]
        extra = CATALOG[c["catalog"]]["server_args"]
        print(f"\n== {c['name']} ({key})", flush=True)
        n = 0
        with Server(model_path(key), extra) as srv, (out_dir / f"{key}.jsonl").open("w") as f:
            for q in questions:
                for mode in ("gold", "retrieved"):
                    ps = contexts[q["id"]][mode]
                    for prompt in PROMPTS:
                        text, info = srv.chat(build_messages(q["question"], ps, prompt), max_tokens=400)
                        f.write(json.dumps({"qid": q["id"], "mode": mode, "prompt": prompt, "passages": [p["id"] for p in ps],
                                            "answer": text, **info}, ensure_ascii=False) + "\n")
                        n += 1
                print(f"  {q['id']:8} done", flush=True)
        print(f"  {n} answers -> {(out_dir / (key + '.jsonl')).relative_to(ROOT)}")


if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).parent))
    main()
