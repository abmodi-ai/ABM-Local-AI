# Plan: ABM Attesta, a knowledge chat assistant for validation and QA teams

Status: draft for review, updated 2026-09-28 · Base model: Qwen3.5 4B (chosen in the pilot bake-off,
`docs/eval/pilot-2026-09-28.md`) · Platforms: **macOS and Windows** · Minimum computer: 12 GB
RAM, CPU only · ⚖ = needs counsel's review

---

## 1. What Attesta is

**ABM Attesta** is a chat model for validation and QA engineering teams. It helps them:
- **generate ideas**: test scenarios, risks they may have missed, questions to ask a supplier;
- **draft documents**: validation plans, intended-use statements, test-plan outlines,
  requirement lists;
- **understand guidance**: explains the EU GMP Annex 22 draft, the FDA–EMA AI principles, Annex
  11 and Part 11, and cites where each point comes from;
- **work with their own material**: users can add their own documents (SOP drafts, URSs,
  templates) to a conversation. They're indexed on the computer and never leave it.

**What it isn't.** Attesta is a thinking and drafting aid. It is **not used in a GxP
environment**, doesn't make or approve GxP decisions, and isn't itself a validated system. The
app says so plainly ("Drafting aid: review everything before use") and so does the model card.
It therefore needs the **basic product validation** described in §7, not GxP computerised-system
validation. (The Annex 22 draft itself says LLMs should not be used in critical GMP applications;
Attesta's positioning keeps it clearly outside that.)

**Where it lives.** Attesta is a download inside ABM Local AI, like the other models in the
library. It is the first of a family of **ABM models**, each for a different job. They all share
the same hub, sign-in, reports and download system. §4 describes what the hub needs for that.

## 2. Context building and engineering

Attesta is a 4B model, so its quality depends on what's in its context. The hub, not the model,
decides what goes in, for each turn:

| Context part | Source | Budget (of ~12k tokens) |
|---|---|---|
| System instructions and Attesta's answer format | Built into the pack | ~600 |
| **Guidance passages** relevant to this turn | Knowledge pack: hybrid retrieval (Qwen3 Embedding + BM25, as in the pilot) | ~2,500 (4–6 passages) |
| **User's own documents** relevant to this turn | Local index of files the user added to the conversation | ~3,000 |
| Conversation so far | Recent turns verbatim; older turns replaced by a running summary | ~3,500 |
| Room for the answer | — | ~2,000 |

Mechanics:
- **Retrieval every turn**, using the question plus a short rewrite of it in context. Follow-ups
  such as "and for the test data?" still find the right passages.
- **Citations:** guidance passages are cited as `[A22 §6.1]` or `[FDA–EMA P8]`, and user documents
  as `[your file: URS-v3.docx p.4]`. Code checks each citation points to something actually in the
  context. Answers with broken citations are regenerated once, then shown with a warning.
- **Memory:** a conversation's summary and the user-document index are stored locally, encrypted
  with a key held in the OS keychain (macOS Keychain, Windows DPAPI). They're deleted with the
  conversation.
- **Context window:** 16k tokens at run time. That fits in 12 GB with Qwen3.5 4B (about 3.3 GB of
  private memory at 8k; the KV cache grows linearly) and leaves room for the OS.
- **Drafting templates:** "Draft a validation plan" fills a fixed outline (purpose, scope,
  intended use, roles, risk assessment, test approach, acceptance criteria, traceability,
  deliverables). Section by section, each part is grounded in guidance and the user's documents.
  Acceptance criteria are left as proposals for the team to decide.
- **Export:** Markdown or Word (.docx), with the sources used listed at the end.

## 3. Sources and training data (no copyright infringement)

The rules and source list are unchanged from the previous version of this plan. Only material
whose licence allows commercial reuse is used, and every training example is logged with where
it came from.

| Source | Status | Terms |
|---|---|---|
| EU GMP Annex 22 "Artificial Intelligence", consultation draft (7 July 2025) | 🟢 ⚖ | European Commission: CC BY 4.0 (attribution). **Draft:** final text expected ~Q4 2026; version the pack |
| FDA–EMA "Guiding principles of good AI practice in drug development" (Jan 2026) | 🟢 ⚖ | EMA: commercial reproduction allowed with EMA acknowledged in each copy; FDA copy is a US government work |
| EU GMP Annex 11 (2011) and the 2025 revision drafts; 21 CFR Part 11; FDA Part 11 and Data Integrity guidance; MHRA Data Integrity guidance | 🟢 | CC BY 4.0 / public domain / Open Government Licence |
| FDA AI-for-regulatory-decision-making draft guidance (2025), GMLP principles, PCCP guidance, EMA AI reflection paper, NIST AI RMF and AI 600-1, EU AI Act | 🟢 to verify one by one | Public domain / EU reuse / EMA terms |
| OWASP Top 10 for LLM Applications | 🟡 CC BY-SA: knowledge pack only, not training | ⚖ |
| ISPE GAMP® guides, ISO/IEC 42001 and 23894, journals, books | 🔴 excluded | Copyright |
| SME-written scenarios, worked examples and plan outlines | 🟢 owned by ABM | Copyright assigned by contract ⚖ |

Training data (3,000–6,000 examples) is generated **locally** by gpt-oss-120b (Apache-2.0),
grounded in one source passage at a time. It's filtered for copied text and for overlap with
the evaluation set, and reviewed by the SME. Because Attesta is a chat assistant, the mix is
weighted towards conversation:

| Type | Share |
|---|---|
| Multi-turn chats with follow-ups (tests context use) | 30% |
| Grounded Q&A with citations | 25% |
| Drafting (plan sections, intended use, test ideas) from guidance + a sample user document | 20% |
| "The sources don't cover this": refusals and asking for more information | 15% |
| General writing (to avoid losing ordinary skills) | 10% |

## 4. What ABM Local AI needs to host ABM models

These changes serve Attesta and every later ABM model.

1. **Pack bundles.** One library card installs a model file plus its knowledge pack plus its
   settings (system instructions, context budgets, templates). The catalog gets a
   `bundle` entry type with a `publisher: "ABM"` badge and an **ABM models** section at the top
   of the library.
2. **A chat window per model.** It replaces today's "Try it" box. It has:
   - conversations;
   - sources shown under each answer;
   - adding files;
   - export;
   - a **Report a problem** button on every answer.
3. **Local document indexing.** PDF, DOCX, TXT and MD: text extraction and embeddings on the
   device, using the same Qwen3 Embedding model.
4. **One model loaded at a time** (as today). Switching models stops one and starts the other,
   while conversations are kept.
5. **Versioned packs.** For example, the Annex 22 final text ships as a knowledge-pack update
   without retraining the model.

## 5. Sign-in: the computer's own login, no third-party services

There is no online account and no identity provider. The app uses the sign-in the user already
has on their computer:

| | macOS | Windows |
|---|---|---|
| How | LocalAuthentication framework: Touch ID, or the Mac login password as fallback | Windows Hello (`UserConsentVerifier`): face, fingerprint or PIN, with the Windows password as fallback |
| When | Opening ABM Local AI, and again after a configurable idle time (default: when the Mac/PC locks) | Same |
| Rust | `objc2-local-authentication` | `windows` crate, `Security::Credentials::UI` |

- **What the app knows about the user:** the OS account name and display name, read locally, so
  the app can show "Signed in as …".
- **An optional profile** the user can fill in (name, email, organisation) is used only to
  prefill reports. It's stored locally and sent only inside a report the user chooses to send.
- **Nothing identifies the user to ABM** unless they send a report.
- If the computer has no biometrics or Hello set up, the OS password prompt is used.
- IT admins can turn the lock off with a policy setting.

So no internet is needed for sign-in. The only internet use is:
- downloading ABM Local AI and model packs, the first time and on updates;
- sending a report, when the user chooses to.

## 6. Reports: crash and inaccuracy (only when the user sends them)

Nothing is sent in the background, ever. Two kinds of report, each previewed before sending:

| Report | Trigger | Contents | Never included |
|---|---|---|---|
| **Crash report** | After a crash, the next launch asks "Send crash report?" | App, OS, model and pack versions; symbolised backtrace; RAM band and CPU class; llama-server exit code; scrubbed log lines | Memory dumps, conversation text, file contents, file paths with user names, OS account name |
| **Inaccuracy report** | **Report a problem** on an answer | The question and answer, the source ids cited, model and pack versions, a category (wrong fact, bad citation, should have said "not covered", unhelpful, other), the user's comment, and optional name and email from the profile | Added user documents (only their file names, and only if the user ticks a box); anything the user removes in the preview. The app highlights likely personal or confidential data first |

- **Separate ticks, off by default:**
  - "ABM may use this report to improve Attesta";
  - "ABM may contact me about it".
- **Where reports go.** One HTTPS endpoint operated by ABM (`reports.abmodi.ai`, for example).
  It stores reports in ABM's own database. There's no analytics SDK and no third-party crash
  service in the app.
- **Hosting.** The endpoint can run on any infrastructure ABM controls. Which one is a decision
  for ABM (see open questions).
- **The brief changes:** "no telemetry" becomes "no background data collection; reports only
  when the user sends them". The CI network test allows only the pack-download and report hosts.
- **Legal basics ⚖:**
  - privacy notice shown before the first report;
  - retention: crash reports 90 days, inaccuracy reports 24 months;
  - deletion on request, using the report ID shown to the user;
  - business users only.

## 7. Basic validation of Attesta (product quality, not GxP)

Attesta ships when it passes these checks. They are recorded in a short release report kept
with each version:

| Check | Target | How |
|---|---|---|
| Correct facts on the 300-question set | ≥ 85% | `eval/` harness (grader + SME sample) |
| Answers with a made-up claim | ≤ 10% | Grader, SME-confirmed sample |
| Citations point to real context | ≥ 90% pass; failures retried or flagged | Code check |
| Says "not covered" when it should | ≥ 80% | Code check |
| Multi-turn: follow-ups use earlier context correctly | ≥ 80% on 50 scripted conversations | New conversation set |
| Drafting: SME rates plan drafts "useful starting point" | ≥ 4/5 average on 20 drafts | SME review |
| Better than the un-tuned base model | On all of the above | Paired comparison |
| Speed on a 12 GB Windows PC and a 12 GB Mac | First words ≤ 3 s; 200-token answer ≤ 20 s | `bench/bench.py` |
| Privacy | No network use except downloads and user-sent reports; conversation text only in the encrypted local store | CI tests |

Plus a **model card**: purpose, not-for-GxP-use statement, base model (Qwen3.5 4B,
Apache-2.0), data sources and licences, evaluation results, known limitations.

## 8. Steps and commands

`[exists]` = in the repo today; `[new]` = to build.

| Week | Step | Command / tool |
|---|---|---|
| 1 | Spike: LoRA on Qwen3.5 4B's text part, fuse, convert to GGUF, run in the pinned llama.cpp. Fallback: Qwen3 4B Instruct 2507 | `uv run --with mlx-lm mlx_lm.lora --model Qwen/Qwen3.5-4B --train --data train/spike --iters 50` `[new data]` |
| 1–2 | AI-guidance source registry and knowledge pack | `eval/sources-ai.json` `[new]`; `uv run eval/build_pack.py --sources eval/sources-ai.json` `[add --sources and parsers]` |
| 2–4 | SME writes 300 questions + 50 scripted conversations + 20 drafting tasks; baseline run | `python3 eval/run.py --questions eval/questions/attesta-sme.jsonl && python3 eval/grade.py && python3 eval/report.py` `[exists; add conversation mode]` |
| 4–8 | Generate, filter, review and log training data | `train/generate.py`, `train/filter.py`, `train/review.py`, `train/split.py` `[new]` |
| 8–9 | Train, test, fuse, convert, quantise | `mlx_lm.lora --model Qwen/Qwen3.5-4B --train --data train/data --fine-tune-type lora --num-layers 16 --batch-size 4 --learning-rate 1e-5 --mask-prompt --grad-checkpoint --adapter-path train/adapters/attesta-v1`, then `mlx_lm.fuse …`, `convert_hf_to_gguf.py … --outtype f16`, `llama-quantize … Q4_K_M` |
| 9–10 | Evaluate against the base model; speed on both 12 GB PCs; release report | `eval/` + `bench/bench.py` `[exists]` |
| 4–12 | Hub: bundles and ABM section, chat window, context builder, local document index, local sign-in, reports, report endpoint | `hub/core`, `hub/app` `[new]` |
| 12 | Publish the `abm-attesta-4b-v1` bundle in the library | `packs/catalog.json` |

## 9. Open questions

1. **Report endpoint hosting:** which infrastructure should ABM's report service run on? It has to
   be something ABM operates. Cloudflare is where ABM already has an account.
2. **Linux:** the hub still builds for Linux today. Are ABM models and local sign-in for macOS and
   Windows only (the Linux hub keeps working without them), or should Linux be dropped?
3. **The AI-validation SME:** who, and when can they start? Weeks 2–4 depend on them.
4. **Trademark search** for "ABM Attesta" (US, EU, UK) ⚖.
