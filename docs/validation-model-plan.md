# Plan: a computerised-system-validation (GAMP 5-aligned) assistant model

Status: draft for review · Decided 2026-09-28: 4B-class base model, 12 GB RAM minimum, Option C
(fine-tuned model + local knowledge pack + code-checked citations) · Not legal advice: every
item marked ⚖ needs review by counsel before release.

## 1. What we are building

An offline assistant for computerised-system validation (CSV/CSA) in GxP work. It:
- answers questions about GAMP 5-aligned practice, 21 CFR Part 11, EU GMP Annex 11 and data
  integrity;
- answers **only from passages retrieved from a local knowledge pack**, cites every claim, and
  says "the sources I have don't cover this" instead of guessing;
- runs on any 64-bit computer with **12 GB RAM, CPU only** (Windows, macOS, Linux).

The model supplies language, terminology and judgement. The facts come from the knowledge pack,
and code checks every citation before the answer is shown. Fine-tuning alone would not make a
4B model reliable on facts (see the GAMP answer in the M0 Try-it test).

**Naming ⚖.** "GAMP" is a registered trademark of ISPE. The product must not be named after it,
e.g. not "GAMP Expert". Use our own brand (e.g. "ABM Validation Assistant") and refer to GAMP only
descriptively ("supports GAMP 5-aligned validation work").

## 2. Base model

Requirements:
- open weights under Apache-2.0 or MIT (commercial use, modification, and our own branding);
- 3–4B parameters, a Q4 file under ~3 GB, so it fits a 12 GB computer with room to spare;
- supported by the pinned llama.cpp (runtime) and by MLX-LM (fine-tuning on the M5 Max).

| Candidate | Licence | Q4 size | Notes |
|---|---|---|---|
| **Qwen3.5 4B** (Feb 2026) | Apache-2.0 | 2.6 GB | Newest Qwen at this size; hybrid thinking, which must be turned off (M0 finding) |
| Qwen3 4B Instruct 2507 | Apache-2.0 | 2.3 GB | M0 baseline: extraction 1.00, NL→SQL 0.80 in the Invoice Analytics bakeoff |
| **Granite 4.2 3B** (Aug 2026, IBM) | Apache-2.0 | 2.1 GB | IBM documents its training-data governance, which matters for our provenance story |
| Gemma 4 E4B (Mar 2026) | Apache-2.0 | 4.6 GB | Multimodal and heavier than the others; a weaker fit for 12 GB |
| Phi-4 mini (3.8B) | MIT | 2.3 GB | Older (Feb 2025) |
| ✗ Llama 3.x, NVIDIA Nemotron | Custom licences | — | Excluded: the Llama licence requires "Llama" in derivative names and "Built with Llama"; NVIDIA's is a custom licence |

**Pilot decision (2026-09-28): Qwen3.5 4B.** Result from the blind pilot bake-off
(`docs/eval/pilot-2026-09-28.md`, 42 questions, grader gpt-oss-120b):
- **Overall:** Qwen3.5 4B scored 76.6 against 69.1 for Granite 4.2 3B. On a paired per-question
  comparison Qwen was 14 points ahead, 95% interval +6 to +22.
- **Made-up claims:** answers containing one: 18% against 42%.
- **Citations:** answers passing the citation check: 84% against 35%.
- **Refusals:** unanswerable questions correctly refused: 56% against 31%.
- **Speed:** Granite was faster (11.9 s against 16.7 s for a 200-token answer on CPU) and slightly
  worse on correct facts (88% against 92%). Up to 13% of its answers looped, repeating the refusal
  sentence until the token limit.

Neither model passes the release gates yet (citation check ≥ 95%, made-up answers ≤ 5%). Closing
that gap is what the fine-tune is for. Both models did clearly better with prompt wording A than B,
so the training data should teach one fixed answer format. Confirm the choice on the expert's
300-question set before training at scale.

Speed and memory on a CPU: see `docs/spike-results.md` ("4B-class candidates"). The final
choice is made by the **evaluation set (step 4.1)**, not by general benchmarks: the base model
with retrieval, before any fine-tuning, that scores best on our questions.

**Rebranding under Apache-2.0 ⚖.** Allowed: rename it, sell it and fine-tune it, with no
obligation to use the original name in the product. Required when we distribute the weights:
- include the Apache-2.0 licence text;
- keep any NOTICE file from the base model;
- state that we modified it.

These go in the pack's licence files and model card, not in the product name or marketing.
Apache-2.0 grants no rights to Alibaba's or IBM's trademarks, so we don't use them in branding. We
should still name the base model honestly in the model card. It's good practice, customers in
regulated industries will ask, and EU AI Act transparency duties for modified general-purpose
models may apply ⚖.

## 3. Data: what we may and may not use

Rule: **train only on material whose licence allows commercial reuse and adaptation**, and
record the source of every training example (§3.4). If a source's terms are unclear, it goes in
the knowledge pack only after review, or not at all.

### 3.1 Source registry

| Source | Status | Why |
|---|---|---|
| 21 CFR Part 11 (eCFR) | 🟢 | US federal government work: public domain (17 U.S.C. §105) |
| FDA guidance: Part 11 Scope and Application; General Principles of Software Validation; Data Integrity and Compliance with CGMP; Computer Software Assurance | 🟢 | US government works; cite the edition and date |
| EU GMP (EudraLex Vol. 4) Annex 11, Chapter 4, and the published Annex 11 revision drafts | 🟢 ⚖ | European Commission documents are reusable with attribution under the Commission's reuse policy; confirm the current terms |
| UK MHRA "GxP" Data Integrity Guidance and Definitions | 🟢 | GOV.UK content: Open Government Licence v3.0 (attribution) |
| ICH Q9(R1), Q10 | 🟡 ⚖ | ICH permits reuse and adaptation with acknowledgement, with conditions; confirm |
| PIC/S PI 011 (computerised systems), PI 041 (data integrity) | 🟡 ⚖ | Freely downloadable but PIC/S copyright; ask PIC/S for permission |
| WHO data-integrity guideline (TRS 1033 Annex 4) | 🔴 | WHO publications are usually CC BY-NC-SA: no commercial use |
| **ISPE GAMP 5 Guide (2nd ed.) and ISPE Good Practice Guides** | 🔴 | ISPE copyright, paid publications. Not for training or the knowledge pack without a licence from ISPE |
| ISPE *Pharmaceutical Engineering* articles, ISO/IEC standards, textbooks, vendor white papers | 🔴 | Copyright; link to them, never train on them |
| Wikipedia | 🟡 ⚖ | CC BY-SA: whether share-alike attaches to model weights is unsettled; avoid |
| Our own expert-written material | 🟢 | We own it: written by our subject-matter expert (SME) under contract assigning copyright to ABM |

The public regulatory texts cover the **requirements** (what Part 11 and Annex 11 demand). The
GAMP **method** (risk-based approach, software categories, lifecycle, V-model deliverables)
comes from our SME explaining it in their own words. Facts and ideas aren't copyrighted; ISPE's
text is. The SME must not copy or closely paraphrase the guide ⚖.

### 3.2 Synthetic data

A large model drafts training examples; the SME reviews them. Rules:
1. **Generator licence:** use an open-weights model whose licence allows training on its
   outputs, run locally. gpt-oss-120b (Apache-2.0, fits in this Mac's 128 GB) or Qwen3-235B/30B
   (Apache-2.0). Do **not** use Claude, GPT or Gemini API outputs: their terms restrict using
   outputs to train other models ⚖.
2. **Grounded generation only:** the generator gets a 🟢 source passage and writes questions
   and cited answers **from that passage**, never from its own memory. This stops copyrighted
   text the generator may have memorised (such as the GAMP guide) from leaking into our data.
3. **Leak filter:** reject any example containing 8+ consecutive words that don't appear in its
   source passage or our SME material, and any output that names a 🔴 source as its basis.
4. **Review:** the SME reviews 100% of the evaluation set and at least 20% of training data
   (stratified by topic). Automated checks cover the rest (citation validity, format).

### 3.3 Example types (the mix to train on)

| Type | Share | Example |
|---|---|---|
| Grounded Q&A with citations | ~45% | "What does Annex 11 require for audit trails?" + passages → cited answer |
| Refusal when passages don't cover it | ~15% | Passages about Part 11 signatures, question about GAMP category 5 testing → "the sources don't cover this" |
| Multi-step judgement | ~20% | "A lab LIMS is configured, not customised: which category, what validation deliverables?" (SME-written) |
| Terminology and definitions | ~10% | ALCOA+, CSA vs CSV, URS/FS/DS, IQ/OQ/PQ |
| General instructions (to avoid forgetting) | ~10% | A permissively licensed general instruction dataset whose own sources are clean ⚖ |

Target: **2,000–5,000 examples** for the first LoRA; more only if the evaluation shows gains.

### 3.4 Provenance ledger (every example)

`id, type, source_doc, source_section, source_licence, generator_model+version, prompt_template,
reviewer, review_date, status`. Stored with the dataset, so we can answer "where did this come
from?" for any example and remove a source's examples if its status changes.

## 4. Where to begin (first four weeks)

1. **Evaluation set (week 1–2, SME).** 300 questions with reference answers and required
   citations. 20% must be unanswerable from the knowledge pack, to test refusal. Split: 60%
   requirements, 25% method/judgement, 15% terminology. Kept **out of training**.
2. **Source registry and knowledge pack v0 (week 1–2).** Download the 🟢 documents, record
   their licences, and split them into passages indexed by the Qwen3 Embedding model. Ship as a
   knowledge pack.
3. **Baseline (week 2).** Each base-model candidate + knowledge pack + citation check, no
   fine-tuning, scored on the evaluation set: accuracy, citation validity, refusal rate. This
   picks the base model and shows the gap fine-tuning must close.
4. **First training set and LoRA (weeks 3–4).** Generate, filter and review per §3.2; train a
   LoRA with MLX-LM on the M5 Max; re-run the evaluation. Ship only if it beats the baseline on
   accuracy and refusal without hurting citation validity.

Exit bar for a first release (proposal): ≥ 85% correct on answerable questions, ≥ 90% correct
refusals on unanswerable ones, 100% of shown answers pass the citation check (failures are
blocked, not shown).

## 5. Packaging

- Merge the LoRA into the base model, convert to GGUF Q4, and publish as our own model pack with:
  - licence and NOTICE files;
  - a model card (base model, data sources and licences, evaluation results, limitations).
  Alternatively, ship the LoRA as a small adapter that the hub loads on top of the base model.
- The knowledge pack is a separate download, versioned independently, so regulator updates
  don't need retraining.

## 6. Open questions for counsel ⚖

1. Current EU Commission reuse terms for EudraLex; ICH and PIC/S reuse for training and for a
   commercial knowledge pack.
2. Whether we may train on facts and concepts described in the GAMP guide as re-expressed by our
   SME, and how to document the SME's independent authorship.
3. Trademark: product naming and how we may refer to GAMP.
4. Obligations as a distributor of a modified open model (EU AI Act, export rules).
5. Terms of the general instruction dataset chosen for §3.3.
