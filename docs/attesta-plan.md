# Plan: ABM Attesta, an offline assistant for validating AI and agentic AI systems

Status: draft for review, 2026-09-28 · Base model: Qwen3.5 4B (chosen in the pilot bake-off,
`docs/eval/pilot-2026-09-28.md`) · Minimum computer: 12 GB RAM, CPU only · ⚖ = needs counsel's
review · This supersedes the GAMP-only scope of `docs/validation-model-plan.md`; that plan's data
rules (§3) still apply.

---

## 1. The product

**Name: ABM Attesta** (working name; from "attest": to confirm that something is valid).
Alternatives if trademark clearance fails: *ABM Assura*, *ABM Provena*. ⚖ Before public use, run
a trademark clearance search in the US (USPTO), EU (EUIPO) and UK (UKIPO), in software and AI
classes 9 and 42. The name must not include "GAMP", "Annex", "FDA" or "EMA".

**What it does.** Helps quality, validation and IT people plan and review the validation of AI
systems used in regulated (GxP) work, including agentic AI (systems that plan and call tools). For
example:
- "Is this AI use critical under the Annex 22 draft?"
- "What acceptance criteria and test-data independence do we need for this defect classifier?"
- "Draft the intended-use statement for our deviation-triage assistant."
- "Which of the FDA–EMA principles does our monitoring plan not yet cover?"

**What it isn't.**
- It is **not** a validated GMP system and must not make GMP decisions. The Annex 22 draft (§1)
  says LLMs "should not be used in critical GMP applications". In non-critical use, qualified
  personnel must check the output: a human-in-the-loop.
- Attesta is an LLM, so it is positioned as a **non-critical advisory tool with a mandatory
  human reviewer**. This is stated in the app, in every generated document and in the model
  card.
- It also teaches this boundary to users validating their own generative or agentic systems.

**How it works (Option C).**
- A fine-tuned Qwen3.5 4B answers from a local **knowledge pack** of public regulatory texts.
- Every claim cites a passage, and code checks the citations before the user sees the answer.
- For structured work (classification, test plans), the model fills in forms and **code applies
  the rules**: the model proposes; code decides (§5).

## 2. Knowledge and training sources (no copyright infringement)

Rule (unchanged): train only on material whose licence allows commercial reuse and adaptation.
Record the provenance of every training example. Facts and ideas aren't copyrighted; a
publisher's wording can be.

### 2.1 Core sources (verified 2026-09-28)

| Source | Status | Terms |
|---|---|---|
| **EU GMP Annex 22 "Artificial Intelligence", consultation draft (7 July 2025)**: scope, intended use, acceptance criteria, test data and its independence, test execution, explainability, confidence, operation, glossary (~2,500 words) | 🟢 ⚖ | European Commission content is CC BY 4.0 (Commission Decision C(2019) 1655): reuse, including commercial, with acknowledgement. **It is a draft:** the final text is expected around Q4 2026, so the pack and training data must be versioned and refreshed when it's final |
| **FDA–EMA "Guiding principles of good AI practice in drug development" (January 2026)**: the 10 principles (~720 words) | 🟢 ⚖ | EMA copy: commercial reproduction allowed "provided that EMA is always acknowledged as the source… in each copy". FDA copy: US government work. Put the acknowledgement in the knowledge pack, the model card and NOTICE |
| EU GMP Annex 11 (2011), plus the revised Annex 11 and Chapter 4 consultation drafts (July 2025) | 🟢 | Commission CC BY 4.0 |
| 21 CFR Part 11; FDA Part 11 Scope & Application; FDA Data Integrity Q&A (already in the eval pack) | 🟢 | US government works, public domain |
| MHRA GXP Data Integrity Guidance | 🟢 | Open Government Licence v3.0 |

### 2.2 Supporting public AI guidance (to verify one by one before use)

| Source | Expected status | Why it matters |
|---|---|---|
| FDA draft guidance "Considerations for the Use of AI to Support Regulatory Decision-Making for Drug and Biological Products" (Jan 2025) | 🟢 likely (US government work) | Credibility-assessment framework, context of use |
| FDA / Health Canada / MHRA "Good Machine Learning Practice for Medical Device Development: Guiding Principles" | 🟢/🟡 (joint; check the Health Canada terms) | Lifecycle and monitoring |
| FDA guidance on Predetermined Change Control Plans for AI-enabled device software | 🟢 likely | Managing model changes |
| EMA reflection paper on AI in the medicinal product lifecycle (2024) | 🟢 (EMA terms: acknowledge the source) | EU regulatory view across the lifecycle |
| NIST AI Risk Management Framework 1.0 and the Generative AI Profile (NIST AI 600-1) | 🟢 likely (US government works) | Risk vocabulary for generative and agentic systems |
| EU AI Act, Regulation (EU) 2024/1689 (EUR-Lex) | 🟢 (EU legislation, free reuse) | Obligations for high-risk AI, human oversight, logging |
| OWASP Top 10 for LLM Applications | 🟡 ⚖ CC BY-SA: share-alike may attach to weights; **knowledge pack only, not training** | Security risks of agents (prompt injection, excessive agency) |
| 🔴 ISPE GAMP® AI Guide, ISO/IEC 42001, ISO/IEC 23894, IEEE standards, journals, books, paid courses | Copyright: never in training or the pack | Link to them only |

### 2.3 Original content we own

A contracted **AI-validation subject-matter expert (SME)** writes worked examples in their own
words, under an agreement that assigns copyright to ABM. That covers what the regulations don't
spell out: how to test an agent's tool use, prompt-injection testing, evaluating LLM outputs,
human-in-the-loop design. Plus scenario libraries: 30–50 realistic AI use cases across GxP
(visual inspection, deviation triage, batch-record review assistant, pharmacovigilance case
intake agent, and so on), with the SME's analysis of each.

## 3. The work, step by step (with the commands)

`[exists]` = already in the repo; `[new]` = to build. All steps run on the M5 Max (128 GB) unless
noted, so no data leaves the machine.

### Phase 0: foundations (weeks 1–2)

| # | Step | Command / tool |
|---|---|---|
| 0.1 | **Base-model spike:** confirm MLX-LM can LoRA-train the *text* part of Qwen3.5 4B (it is an image-text model), and that the fused result converts to GGUF and runs in the pinned llama.cpp. Fallback: Qwen3 4B Instruct 2507 (text-only; scored close in the bench) | `uv run --with mlx-lm mlx_lm.lora --model Qwen/Qwen3.5-4B --train --data train/spike --iters 50` `[new data]` |
| 0.2 | Source registry for AI guidance: licence, URL, date, draft/final | `eval/sources-ai.json` `[new]`, same format as `eval/sources.json` |
| 0.3 | Build the knowledge pack (add Annex 22 and FDA–EMA parsers) | `uv run eval/build_pack.py --sources eval/sources-ai.json` `[exists; add --sources and 2 parsers]` |
| 0.4 | Hire or contract the SME; copyright assignment and NDA ⚖ | — |
| 0.5 | Trademark search for "Attesta" ⚖ | — |

### Phase 1: evaluation set first (weeks 2–4, SME)

| # | Step | Command / tool |
|---|---|---|
| 1.1 | 300 questions in the eval format: 50% requirements (Annex 22 / principles / Annex 11), 25% judgement on scenarios, 10% terminology, 15% unanswerable. Plus a **tool-use set** of 60 tasks (§5) | `eval/questions/attesta-sme.jsonl` `[new]` |
| 1.2 | Baseline: Qwen3.5 4B + pack, no fine-tuning (this is the number to beat) | `python3 eval/run.py --questions eval/questions/attesta-sme.jsonl && python3 eval/grade.py && python3 eval/report.py` `[exists]` |
| 1.3 | Grader fix from the pilot: show the grader each passage's document title | `eval/grade.py` `[small change]` |
| 1.4 | SME reviews the grader's sample and records agreement | `.cache/eval/review/sme_sample.jsonl` `[exists]` |

### Phase 2: training data (weeks 4–8)

| # | Step | Command / tool |
|---|---|---|
| 2.1 | **Grounded generation:** gpt-oss-120b (Apache-2.0, local) writes questions and cited answers *from one passage at a time*, never from memory. Types and shares: grounded Q&A 40%, refusals 15%, scenario judgement 20%, tool-use traces 15%, terminology 5%, general instruction 5% | `python3 train/generate.py --generator judge-gpt-oss-120b --pack .cache/eval/pack --plan train/mix.yaml` `[new]` |
| 2.2 | **Filters:** reject any example with 8+ consecutive words not found in its source or in SME text (leak filter); reject invalid citations and duplicates; drop anything close to an evaluation question (contamination check, embedding similarity > 0.9) | `python3 train/filter.py` `[new]` |
| 2.3 | **SME review:** 100% of scenario and tool-use examples, and at least 20% of the rest | `python3 train/review.py` `[new]`: a local review page, accept/edit/reject |
| 2.4 | **Provenance ledger** for every example: source, section, licence, generator and version, prompt template, reviewer, date | `train/data/ledger.jsonl` `[new]` |
| 2.5 | Split into train and validation sets (the eval set is never included) | `python3 train/split.py` → `train/data/{train,valid}.jsonl` `[new]` |

Target: 3,000–6,000 examples for v1. The core regulatory texts are short (~3,200 words), so
depth comes from scenarios and SME material, not from repeating the texts.

### Phase 3: fine-tune and evaluate (weeks 8–10)

| # | Step | Command / tool |
|---|---|---|
| 3.1 | LoRA training (typical start: 16 layers, rank 16, learning rate 1e-5, 2–3 epochs, prompt masked so only answers are learned) | `uv run --with mlx-lm mlx_lm.lora --model Qwen/Qwen3.5-4B --train --data train/data --fine-tune-type lora --num-layers 16 --batch-size 4 --iters <≈3 epochs> --learning-rate 1e-5 --mask-prompt --grad-checkpoint --adapter-path train/adapters/attesta-v1` |
| 3.2 | Check on held-out data | `mlx_lm.lora … --test` |
| 3.3 | Merge the adapter into the base weights | `mlx_lm.fuse --model Qwen/Qwen3.5-4B --adapter-path train/adapters/attesta-v1 --save-path train/fused/attesta-v1` |
| 3.4 | Convert to GGUF and quantise to Q4_K_M (tools from the pinned llama.cpp build) | `python convert_hf_to_gguf.py train/fused/attesta-v1 --outtype f16` then `llama-quantize attesta-v1-f16.gguf attesta-4b-v1-Q4_K_M.gguf Q4_K_M` |
| 3.5 | Run the same evaluation as the baseline, plus the tool-use set, plus a **general-skills regression** check (it must not get worse at ordinary writing) | `python3 eval/run.py --only attesta-v1 …` `[exists; add contestant]` |
| 3.6 | Speed and memory on the 12 GB Windows reference PC | `python bench/bench.py attesta-v1 --runs 5` `[exists]` |
| 3.7 | Iterate on data, not hyperparameters, until the gates pass (§7) | — |

### Phase 4: product integration (weeks 8–12, in parallel)

| # | Step | Where |
|---|---|---|
| 4.1 | Model pack `attesta-4b-v1` + knowledge pack `ai-validation-pack` in the catalog, with licence, NOTICE (EMA/Commission acknowledgements, Qwen Apache-2.0) and the model card | `packs/catalog.json`, new pack-manifest fields |
| 4.2 | Hub tool-calling loop with step/time budgets and citation checks, i.e. the M3 harness (§5) | `hub/core` + SDK |
| 4.3 | Attesta window: chat with sources shown, a "Human review required" banner, **Report a problem** on every answer | `hub/app/ui` |
| 4.4 | Accounts and reports (§6) | `hub/core/account.rs`, `reports.rs`; backend §6.4 |
| 4.5 | Signing and notarisation for public release | M5 of the implementation plan |

### Phase 5: release and after (week 12 onward)

- **v1 release criteria:** §7.
- **Monthly:** triage inaccuracy reports and fix them in the knowledge pack or the training data;
  re-run the evaluation.
- **When Annex 22 is final (expected Q4 2026):** new pack version, re-generated training data
  for the changed clauses, attesta-v2.

## 4. Team and cost (estimate)

| Role | Effort |
|---|---|
| AI-validation SME (contract) | ~40–60 days over 12 weeks (eval set, review, scenarios) |
| ML engineer (data pipeline, training, evaluation) | 1 person, ~12 weeks |
| App/backend engineer (tool loop, accounts, reports) | 1 person, ~8 weeks |
| Counsel ⚖ | Licences, trademark, privacy policy, terms: ~5–10 days |

**Compute:**
- **Training:** the M5 Max is enough for LoRA on 4B, and for generation and grading with
  gpt-oss-120b, so there's no cloud GPU cost.
- **Backend:** small (§6.4).
- **Test hardware:** a 12 GB Windows reference PC and a 12 GB Linux reference PC.

## 5. Tool calls: what Attesta can call at runtime

An agentic assistant for validating agents should itself follow the rules it teaches:
- a fixed list of declared tools;
- step and time budgets;
- every tool is read-only or produces a draft;
- the model never "decides" a classification. It fills in a form, and code applies the
  published criteria.

| Tool | Input (JSON schema, bounded strings) | Output | Notes |
|---|---|---|---|
| `search_sources` | `query`, `k≤6`, optional `source` filter | passages with ids, titles, licence | Hybrid retrieval from the pilot (Qwen3 Embedding + BM25) |
| `get_passage` | `id` | full passage + neighbours | To read around a hit |
| `assess_annex22_applicability` | form: `used_in_gmp`, `impact_on_patient_safety/product_quality/data_integrity`, `learns_during_use`, `deterministic_output`, `generative_or_llm`, `human_in_the_loop`, free-text `context_of_use` | **Code** returns: in or out of Annex 22 scope, "not for critical use" flags, the clauses that apply, and questions still open | Encodes Annex 22 draft §1 and §3.3; versioned with the draft |
| `map_to_principles` | a plan or description the user pasted | per principle 1–10: covered / partly / missing, each with a citation | The model fills it in; code checks that each entry cites the principle's passage |
| `draft_intended_use` | form: task, inputs (sample space), subgroups, limitations, HITL responsibility | a draft in the Annex 22 §3 structure, marked DRAFT for SME approval | Output is a document draft, never an approval |
| `draft_test_plan` | model type, metrics, subgroups, risk | test plan skeleton: metrics and acceptance criteria (§4), test data and independence (§5–6), execution (§7), explainability and confidence (§8–9), operation and monitoring (§10) | Criteria stay blank for the SME to set: Annex 22 puts that responsibility on the SME |
| `agent_risk_checklist` | agent description: tools, permissions, autonomy, data | checklist: tool allowlist, least privilege, human approval points, logging, prompt-injection tests, step budgets, rollback | Sourced from NIST AI 600-1, the EU AI Act, OWASP (pack only) and SME material |
| `export_document` | document id, format (md/docx/pdf) | a file saved locally | Adds the "Advisory draft, human review required" footer and the source list |

Loop limits: at most 8 tool calls and 120 s per request (the Invoice Analytics harness values),
then a final answer with citations or a refusal. The training data includes tool-use traces
(Phase 2.1) so the model learns when to call which tool.

## 6. Privacy, accounts and reports

### 6.1 What changes from the brief, and what doesn't

- **Unchanged.** The hub still runs fully offline and still never stores prompts or answers on
  its own. There is still **no background telemetry**: nothing is sent automatically, ever.
- **New:** exactly two kinds of internet use, in addition to model downloads:
  1. **Account set-up (once, online).** Needed on first launch. After that, the app works
     offline indefinitely.
  2. **Reports (only when the user chooses to send one).**
     - *Crash reports:* after a crash, the next launch offers "Send crash report?", with a
       preview of exactly what's sent.
     - *Inaccuracy reports:* the **Report a problem** button on an answer.
- The brief's goal 3 ("no telemetry"), §6.4 (network) and the CI socket test are updated to allow
  only the account and report hosts, and only on these user actions.

### 6.2 What is collected

| Data | When | Contents | Never included |
|---|---|---|---|
| **Account** | Set-up | Email (verified), name, organisation, country, accepted terms/privacy version, created date | Passwords (held only by the identity provider), payment data (free product) |
| **Session** | Set-up | A signed token so the app can show "Signed in as…" offline. No expiry needed for offline use (the product is free) | — |
| **Crash report** | User clicks Send | App/OS/pack versions, backtrace with symbols, hardware class (RAM band, CPU class), llama-server exit code, **scrubbed** logs | Memory dumps, prompts, answers, file paths with usernames (scrubbed), IP address stored beyond abuse rate-limiting |
| **Inaccuracy report** | User clicks Report a problem | Question, answer, cited passage ids, pack and model versions, user's comment and category (wrong fact / bad citation / should have refused / other) | Anything the user removes in the preview. The app first **flags likely personal or confidential data** (emails, names, IDs, numbers) for the user to redact |

Separate choices, off by default, asked when a report is sent:
- "May ABM use this report to improve Attesta's training data?" (reports can be used to fix bugs
  without this).
- "May ABM contact me about this report?"

Apps that declare `data class: sensitive` (e.g. patient data) can send crash reports only. The
**Report a problem** button there sends only the metadata and the user's comment, never the
question or answer text.

### 6.3 Legal basics ⚖
- Privacy policy and terms; lawful basis:
  - accounts: contract;
  - reports: consent;
  - training use: separate explicit consent.
- Retention:
  - crash reports: 90 days;
  - inaccuracy reports: 24 months, or until the fix ships;
  - accounts: until deleted.
- Rights:
  - export and delete account data from the app;
  - withdraw training consent (removes the report from future training data; the provenance
    ledger makes this possible).
- Hosting in the EU, or with EU standard contractual clauses; a data-processing agreement with
  each provider.
- Not for children; business users only.

### 6.4 Backend (small)

| Need | Recommended | Why |
|---|---|---|
| Sign-in (email + magic link or OAuth; SSO later for companies) | A hosted identity provider (e.g. WorkOS, Auth0, Clerk or Supabase Auth). The desktop app uses the OAuth device-code or system-browser flow | We never handle passwords; enterprise SSO is available later |
| Report intake API | Cloudflare Workers + D1 (reports) + R2 (attachments). ABM already has a Cloudflare account | Small, cheap, EU-region data localisation available |
| Crash symbolication | Upload debug symbols per release; symbolise on the server | Readable backtraces without shipping debug info |
| Review console | A small internal page for triaging inaccuracy reports into "fix pack", "fix training data" or "not a bug" | Closes the loop to Phase 5 |

All endpoints are HTTPS with certificate pinning to ABM's hosts, per-account rate limits, and no
third-party analytics scripts.

## 7. Release gates for Attesta v1

| Gate | Threshold | Measured by |
|---|---|---|
| Citation check passes | ≥ 95% of answers | `eval/grade.py` |
| Answers with a made-up claim | ≤ 5% | Grader, with the SME confirming a sample |
| Correct facts (answerable) | ≥ 85% | Grader |
| Correct refusals (unanswerable) | ≥ 90% | Code |
| Tool-use tasks completed correctly | ≥ 85%; 0 calls to undeclared tools | Tool-use set |
| Annex 22 applicability decisions | 100% match the SME's answers on 50 scenarios (code decides; this tests the form-filling) | Scenario set |
| Beats the un-tuned baseline | On every quality gate above | Paired comparison |
| Speed on the 12 GB Windows PC | ≤ 3 s first token (short prompt), ≤ 20 s for a 200-token answer | `bench/bench.py` |
| Privacy | Zero network connections except account, report and pack hosts on user action; no prompt text on disk | CI socket and disk tests |

## 8. Main risks

| Risk | Mitigation |
|---|---|
| Annex 22 changes when it's final (1,300 consultation comments) | Versioned pack; the model cites clauses, so a pack refresh fixes most answers; a retrain plan for the final text |
| Qwen3.5 4B text-only fine-tuning doesn't work cleanly (multimodal architecture) | Phase 0.1 spike in week 1; fall back to Qwen3 4B Instruct 2507 |
| Too little source text for depth | SME scenarios and worked examples; tool-based structured outputs |
| Users treat Attesta's output as validated | "Advisory draft, human review required" on every answer and export; positioning in the terms |
| Reports contain confidential or patient data | Local detection and redaction before sending; text never sent from sensitive apps; consent choices |
| Copyright claim over training data | Only 🟢 sources; leak filter; provenance ledger; SME copyright assignment; counsel's review |
