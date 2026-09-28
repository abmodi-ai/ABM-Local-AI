# Evaluation: choosing the base model

This test decides which base model the validation assistant is built on (see
`docs/validation-model-plan.md`). Both models get identical evidence, prompts and settings; only
the model changes. The decision rule was agreed before any results were seen and is written at the
top of `report.py`.

```sh
uv run eval/build_pack.py                     # knowledge pack from eval/sources.json (public, licence-checked)
python scripts/fetch_model.py qwen3.5-4b granite-4.2-3b embed-qwen3-0.6b judge-gpt-oss-120b
python3 eval/run.py                           # answers: 2 modes x 2 prompt wordings per question
python3 eval/grade.py                         # code checks + blind grader (gpt-oss-120b, ~64 GB RAM)
python3 eval/report.py                        # -> docs/eval/<questions>-<date>.md
```

## Parts

| File | What it does |
|---|---|
| `sources.json` | The knowledge-pack sources and their licences (green = reuse allowed) |
| `build_pack.py` | Downloads the sources and splits them into citable passages (`part11:11.10(e)`, `annex11:9`, …) |
| `questions/pilot.jsonl` | 42 pilot questions written from the sources: 34 answerable, 8 deliberately not covered |
| `common.py` | llama-server processes, hybrid retrieval (Qwen3 Embedding + BM25), the two prompt wordings |
| `run.py` | Generates answers with the gold passages and with real retrieval |
| `grade.py` | Code checks (citations, refusals, looping) and the blind grader (facts, supported claims) |
| `report.py` | Scores, hard gates, weighted score, paired bootstrap, and the decision |

## Question format

One JSON object per line:

```json
{"id": "a11-09", "category": "requirement", "answerable": true,
 "question": "What does EU GMP Annex 11 require for audit trails?",
 "key_facts": ["...", "..."], "must_cite": ["annex11:9"]}
```

`category` is `requirement`, `judgement`, `terminology` or `unanswerable`. For unanswerable
questions, `key_facts` and `must_cite` are empty, and the correct answer is the refusal sentence.

## Replacing the pilot with the expert's set

The subject-matter expert's 300 questions use the same format in `questions/sme.jsonl`:

```sh
python3 eval/run.py --questions eval/questions/sme.jsonl && python3 eval/grade.py && python3 eval/report.py
```

The expert then reviews `.cache/eval/review/sme_sample.jsonl`, which holds a random 20% of
answers plus every case where the code checks and the grader disagree. They fill in
`expert_verdict` and `expert_notes`. If the expert and the grader often disagree, the expert's
grades decide.
