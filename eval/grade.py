"""Grade every answer: code checks first, then a blind grader model for facts and support.

    uv run eval/grade.py            # grades .cache/eval/runs/*.jsonl -> .cache/eval/graded.jsonl

Code checks (no model involved):
  refused            the answer is the refusal sentence (or clearly says the sources don't cover it)
  citation_pass      every factual sentence ends with a valid [n] citation
  lexical_support    share of cited sentences whose content words mostly appear in the cited passages
  cited_required     at least one cited passage is one the question requires
  truncated          the answer hit the token limit
  degenerate         cut off while repeating the same line (a looping answer)
Grader (gpt-oss-120b, a model family neither contestant belongs to):
  facts_present      which key facts the answer states correctly
  claims             each claim the answer makes, and whether the provided sources support it
The grader sees the question, the passages the model saw, the key facts and the answer, but never
which model wrote it, and answers from both models are graded in shuffled order.
Also writes .cache/eval/review/sme_sample.jsonl: a random 20% plus every answer where the code
checks and the grader disagree, for the subject-matter expert to review.
"""

from __future__ import annotations

import json
import random
import re
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from common import JUDGE, REFUSAL, ROOT, WORK, Server, load_passages, model_path, norm, tokens  # noqa: E402

CITE = re.compile(r"\[(\d+(?:\s*[,\]\[]\s*\d+)*)\]")
REFUSAL_LOOSE = re.compile(r"(do(es)? not (cover|contain|address|answer|provide|specify|mention))|(not (covered|addressed|specified|mentioned) in the (provided )?sources)")

JUDGE_SYSTEM = (
    "You grade answers written by an AI assistant for regulated pharmaceutical work. "
    "Judge strictly and only against the SOURCES given; outside knowledge does not count as support. "
    "Respond with JSON only."
)
JUDGE_SCHEMA = {
    "type": "object",
    "properties": {
        "facts": {"type": "array", "items": {"type": "object", "properties": {"n": {"type": "integer"}, "present": {"type": "boolean"}}, "required": ["n", "present"]}},
        "claims": {"type": "array", "maxItems": 12, "items": {"type": "object", "properties": {"claim": {"type": "string", "maxLength": 200}, "supported": {"type": "boolean"}}, "required": ["claim", "supported"]}},
    },
    "required": ["facts", "claims"],
}


def sentences(answer: str) -> list[str]:
    text = re.sub(r"^\s*([-*•]|\d+\.)\s+", "", answer, flags=re.M)
    parts = re.split(r"(?<=[.!?])\s+(?=[A-Z(])|\n+", text)
    return [p.strip() for p in parts if len(p.split()) >= 4 and norm(REFUSAL) not in norm(p)]


def code_checks(ans: dict, q: dict, passages: dict) -> dict:
    a = ans["answer"]
    # A refusal is an answer that is (almost) only the refusal. An answer with real content that
    # also contains the refusal sentence, e.g. repeated in a loop, is an answer, not a refusal.
    rest = norm(a).replace(norm(REFUSAL), " ")
    refused = (norm(REFUSAL) in norm(a) and len(rest.split()) < 15) or (len(a.split()) < 40 and bool(REFUSAL_LOOSE.search(a.lower())))
    lines = [norm(l) for l in a.splitlines() if l.strip()]
    degenerate = ans.get("finish") == "length" and any(lines.count(l) >= 3 for l in set(lines))
    sents = sentences(a)
    shown = ans["passages"]
    cited_sents, valid, support = 0, True, []
    cited_ids: set[str] = set()
    for s in sents:
        nums = [int(x) for grp in CITE.findall(s) for x in re.findall(r"\d+", grp)]
        if nums:
            cited_sents += 1
        if any(n < 1 or n > len(shown) for n in nums):
            valid = False
        ids = [shown[n - 1] for n in nums if 1 <= n <= len(shown)]
        cited_ids.update(ids)
        if ids:
            src = set(tokens(" ".join(passages[i]["text"] for i in ids)))
            words = [w for w in tokens(CITE.sub("", s)) if len(w) > 2]
            support.append(sum(w in src for w in words) / max(1, len(words)) >= 0.5)
    return {
        "refused": refused,
        "exact_refusal": a.strip() == REFUSAL,
        "factual_sentences": len(sents),
        "citation_coverage": round(cited_sents / len(sents), 3) if sents else None,
        "citation_pass": bool(sents) and cited_sents == len(sents) and valid,
        "lexical_support": round(sum(support) / len(support), 3) if support else None,
        "cited_required": bool(cited_ids & set(q["must_cite"])),
        "truncated": ans.get("finish") == "length",
        "degenerate": degenerate,
        "words": len(a.split()),
    }


def judge_prompt(ans: dict, q: dict, passages: dict) -> list[dict]:
    src = "\n\n".join(f"[{i + 1}] {passages[pid]['text']}" for i, pid in enumerate(ans["passages"]))
    facts = "\n".join(f"{i + 1}. {f}" for i, f in enumerate(q["key_facts"]))
    return [
        {"role": "system", "content": JUDGE_SYSTEM},
        {"role": "user", "content": (
            f"QUESTION:\n{q['question']}\n\nSOURCES:\n{src}\n\nKEY FACTS a complete answer should contain:\n{facts}\n\n"
            f"ANSWER TO GRADE:\n{ans['answer']}\n\n"
            "Tasks:\n"
            "1. For each key fact n, set present=true only if the answer states it correctly (paraphrase is fine).\n"
            "2. List each distinct factual claim in the answer (at most 12) and set supported=true only if the "
            "SOURCES support it. Claims contradicting or going beyond the sources are unsupported.\n"
            'Return JSON: {"facts":[{"n":1,"present":true},...],"claims":[{"claim":"...","supported":true},...]}'
        )},
    ]


def parse_json(text: str) -> dict:
    m = re.search(r"\{.*\}", text, re.S)
    return json.loads(m.group(0)) if m else {}


def main() -> None:
    runs = sorted((WORK / "runs").glob("*.jsonl"))
    if not runs:
        sys.exit("no runs; run: uv run eval/run.py")
    questions = {json.loads(l)["id"]: json.loads(l) for l in (ROOT / json.loads((WORK / "runs" / "meta.json").read_text())["questions"]).open()}
    passages = {p["id"]: p for p in load_passages()}
    items = []
    for f in runs:
        for line in f.open():
            a = json.loads(line)
            a["contestant"] = f.stem
            a.update(code_checks(a, questions[a["qid"]], passages))
            items.append(a)
    to_judge = [a for a in items if questions[a["qid"]]["answerable"] and not a["refused"]]
    random.Random(7).shuffle(to_judge)  # blind and interleaved across contestants
    print(f"{len(items)} answers; {len(to_judge)} need the grader", flush=True)

    with Server(model_path(JUDGE), ctx=32768, parallel=4) as srv:
        def grade(a: dict) -> None:
            q = questions[a["qid"]]
            for attempt in range(2):
                try:
                    text, _ = srv.chat(judge_prompt(a, q, passages), max_tokens=3000,
                                       chat_template_kwargs={"reasoning_effort": "medium"})
                    j = parse_json(text)
                    present = {int(x["n"]): bool(x["present"]) for x in j.get("facts", [])}
                    a["facts_present"] = [present.get(i + 1, False) for i in range(len(q["key_facts"]))]
                    a["claims"] = [{"claim": c["claim"], "supported": bool(c["supported"])} for c in j.get("claims", [])]
                    return
                except Exception as e:  # noqa: BLE001 - retry once, then record the failure
                    a["judge_error"] = str(e)[:200]
        done = 0
        with ThreadPoolExecutor(4) as pool:
            for _ in pool.map(grade, to_judge):
                done += 1
                if done % 20 == 0:
                    print(f"  graded {done}/{len(to_judge)}", flush=True)

    out = WORK / "graded.jsonl"
    with out.open("w") as f:
        for a in items:
            f.write(json.dumps(a, ensure_ascii=False) + "\n")

    # Expert review sample: 20% at random plus disagreements between code and grader.
    rng = random.Random(11)
    def disagree(a: dict) -> bool:
        if "claims" not in a or a.get("lexical_support") is None:
            return False
        all_ok = all(c["supported"] for c in a["claims"])
        return (a["lexical_support"] < 0.5 and all_ok) or (a["lexical_support"] == 1.0 and not all_ok)
    sample = [a for a in items if disagree(a)]
    rest = [a for a in items if a not in sample]
    sample += rng.sample(rest, max(1, len(items) // 5))
    review = WORK / "review"
    review.mkdir(exist_ok=True)
    with (review / "sme_sample.jsonl").open("w") as f:
        for a in sample:
            q = questions[a["qid"]]
            f.write(json.dumps({"review_id": f"{a['qid']}/{a['mode']}/{a['prompt']}/{abs(hash(a['contestant'])) % 1000:03d}",
                                "question": q["question"], "key_facts": q["key_facts"], "answerable": q["answerable"],
                                "passages": {pid: passages[pid]["text"] for pid in a["passages"]}, "answer": a["answer"],
                                "grader_facts": a.get("facts_present"), "grader_claims": a.get("claims"),
                                "expert_verdict": None, "expert_notes": None}, ensure_ascii=False) + "\n")
    errors = sum("judge_error" in a and "claims" not in a for a in items)
    print(f"graded -> {out.relative_to(ROOT)}; expert sample: {len(sample)} answers; grader failures: {errors}")


if __name__ == "__main__":
    main()
