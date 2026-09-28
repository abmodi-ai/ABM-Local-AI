# ABM Local AI: project brief

Status: draft for review · Owner: ABM · First client app: ABM Invoice Analytics

## 1. Summary

ABM Local AI is a desktop hub that runs open-source language models locally and serves them to
every ABM app on the same computer. It replaces the per-app model runtime. Each app gets:

- a shared, already-running model (no per-app 2–5 GB download);
- a local, OpenAI-compatible API;
- a baseline of privacy and safety rules the hub enforces for every app;
- a shared harness library for domain context, output checks and evaluation.

The hub ships without a model. Models are separate **model packs** the user installs.

## 2. Problem

- Local AI needs a runtime (llama.cpp) and a model file of 0.5–20 GB. If every app bundles its
  own, users download the same model many times and it is loaded into memory several times.
- Bundling a model inside an app installer is capped at about 2 GB (GitHub release assets and
  the NSIS installer format), which excludes the better models.
- ABM apps handle sensitive data (patient and billing records). Each app currently reinvents the
  same guardrails: local-only access, structured output, no prompt logging, and output checks.
- General-purpose models are not trustworthy on their own. Small local models in particular
  follow instructions loosely, so safety has to be enforced by code.

## 3. Goals and non-goals

**Goals**
1. Any ABM desktop app can use local AI with no model setup of its own.
2. Users can add, remove and switch model packs without reinstalling apps.
3. Privacy guarantees hold for every app, including badly written ones. The hub listens only on
   the local machine, stores no prompts or outputs, isolates apps from each other and sends no
   telemetry.
4. A reusable harness (context, tools, output checks, evaluation) so each new app gets
   trustworthy AI features quickly.
5. Model packs are scored against each app's task suite, so an app only uses models that pass
   its bar.
6. Runs on an ordinary 64-bit Windows 10/11 PC with **at least 12 GB of RAM** and no GPU (minimum
   raised from 8 GB on 2026-09-28; 4B-class models are the smallest offered). macOS and
   Linux (Ubuntu 22.04+, Debian 12+, Fedora 39+) are also supported from v1. Windows is the
   primary platform.

**Non-goals (for v1)**
- Cloud or remote inference, or a hub shared across machines.
- Training models from scratch. Adapters (LoRA) are optional and come later (section 6.3).
- A general chatbot UI. The hub is infrastructure; apps provide the user experience.
- A shared knowledge store across apps (section 11, question 2).

## 4. Users

| User | Needs |
|---|---|
| End user of an ABM app | Turn AI on with one install; see which apps use it; revoke access |
| IT administrator | Silent install, a fixed approved model list, policy by machine, no internet needed after setup |
| ABM app developer | A few lines of code to find the hub, get approved, and call a model with structured output, tools and checks |

## 5. Architecture

```
┌──────────────────────── App (e.g. ABM Invoice Analytics) ──────────────────────┐
│  Domain harness: context from the app's own data, versioned prompts, tools,    │
│  output checks, the user's permissions, human decisions, the app's test sets   │
│                ▲ uses the shared harness library (Python / TypeScript)         │
└────────────────┼───────────────────────────────────────────────────────────────┘
                 │ local HTTP · per-app key
┌────────────────┴──────────── ABM Local AI hub ─────────────────────────────────┐
│  App approval and keys · per-app policy · schema-enforced output · limits and  │
│  quotas · scheduling · no prompt storage · isolation between apps · model      │
│  checksums and licences · pack manager · metadata-only audit                   │
│                ▼ supervises                                                    │
│  llama.cpp llama-server (child process, local only, random key)                │
└────────────────┬───────────────────────────────────────────────────────────────┘
┌────────────────┴──────────── Model packs ──────────────────────────────────────┐
│  GGUF base model + manifest (sha256, licence, RAM, capabilities, scores)       │
│  + optional LoRA adapters per domain                                           │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Components

1. **Hub app.** Tray app with a small window: installed packs, connected apps, activity (metadata
   only) and settings. It supervises llama-server, restarting it on crashes and unloading the
   model when idle. v1 uses one loaded model at a time with a fair queue across apps.
2. **Model packs and catalog.** A pack is a `.zip` (or `.gguf` plus a manifest) with a sha256,
   licence, minimum RAM, context length, capability tags and evaluation scores. Packs under
   2 GB attach to GitHub releases; larger ones are listed in a catalog file that links to
   Hugging Face. The hub verifies every file before use.
3. **Client SDK** (Python and TypeScript). It covers discovery, the approval flow, secure key
   storage (DPAPI on Windows, Keychain on macOS, Secret Service on Linux), OpenAI-compatible calls, retries, and an
   "AI unavailable" state apps can show to users.
4. **Harness library** (ships inside the SDK). It covers:
   - a prompt registry with versions;
   - structured output via JSON schema;
   - a tool-calling loop with step and time budgets;
   - output checks (grounding, citations, SQL safety);
   - traces for review;
   - an evaluation runner.
5. **Evaluation runner.** Scores a model pack on an app's task suite, and writes the scores into
   the catalog and the app's allow-list.

### 5.2 Where each guardrail lives

| Concern | Layer | Reason |
|---|---|---|
| Facts, customer and patient data, business rules | **App**, supplied at request time | Data changes and is private. Training it into weights is a compliance risk (models memorise), goes stale and can't be deleted. |
| Checking outputs against the truth (every amount/date appears in the evidence; SQL is a single read-only SELECT) | **App**, via the harness library | Only the app knows the ground truth. |
| Tools and permissions | **App** defines them; **hub** enforces the tool list the app declared | Two locks: app logic plus hub policy. |
| Valid JSON or grammar-constrained output | **Hub** (llama.cpp `json_schema` / GBNF) | Generic, and the most reliable guardrail for small models. |
| Allowed models, token and time limits, concurrency and quotas | **Hub**, from the app's declared policy | Uniform, and stops one app starving the others. |
| No prompt storage, isolation between apps, local-only access, no telemetry | **Hub** | Must hold even for a badly written app. |
| Domain vocabulary, format adherence | **Model**, as optional LoRA adapters loaded per app | The one thing weights do well; only when evaluation shows a clear gain. |
| Safety policy and refusals | **Never the model alone** | Weights can be talked around; they back up code, never replace it. |

Design principle: **the hub is inference plus policy; the harness lives in the app process.**
Putting task logic in the hub would couple every app to one hub version and put every app's data
semantics in one place.

## 6. Key designs

### 6.1 Discovery and app approval
- The hub writes `endpoint.json` (port, protocol version, public key fingerprint) to a well-known
  per-user location: `%LOCALAPPDATA%\ABM\LocalAI\` on Windows and
  `~/Library/Application Support/ABM/LocalAI/` on macOS, and `$XDG_DATA_HOME/abm/localai/`
  (default `~/.local/share/abm/localai/`) on Linux.
- The first time an app connects, it sends its name, publisher and a **policy manifest**:
  - models it wants and allowed tools;
  - maximum tokens per request;
  - data class (`general` or `sensitive`), where `sensitive` means PHI or PII.
- The hub shows the user "ABM Invoice Analytics wants to use local AI" with **Allow / Deny**.
  Approval issues a per-app key, which is revocable in the hub window.
- Apps signed by ABM can be pre-approved by an IT policy file for managed installs.

### 6.2 API
- OpenAI-compatible: `/v1/chat/completions` (streaming, `response_format` with JSON schema, tools),
  `/v1/embeddings`, `/v1/models`. Each call is filtered by that app's policy.
- Hub endpoints:
  - `/hub/v1/pair` and `/hub/v1/status` (loaded model, queue, readiness, missing setup steps);
  - `/hub/v1/packs` (installed packs and their scores).
- Bound to 127.0.0.1 only, with the Host and Origin headers checked. There is no CORS for
  browsers unless a specific app requests it.

### 6.3 Models
- **Licences:** Apache-2.0 or MIT only in v1 (for example the Qwen3 family). Other licences can
  come later, behind an explicit warning.
- **Suggested starting packs** (to be confirmed by evaluation):
  - **Minimum: 12 GB RAM, 4B-class models.** Nothing below 12 GB is offered (decided
    2026-09-28, after M0 showed 1.7B models state wrong facts confidently).
  - Default: a 4B instruct model, Q4, about 2.5 GB (Qwen3 4B Instruct 2507 today; the next
    candidates, Qwen3.5 4B and Granite 4.2 3B, are being benchmarked).
  - A small embeddings model for retrieval.
  - Larger packs for 16 GB+ and Apple Silicon machines.
- **Adapters (later):** LoRA adapters per domain, loaded per request (llama.cpp supports LoRA). Build
  one only if base model plus retrieval misses an app's bar on its task suite.

### 6.4 Privacy and security
- The hub never writes prompts or outputs to disk. The audit log records only app, model,
  tokens, duration and result code.
- **Isolation between apps:** no shared prompt cache for `sensitive` apps and no cross-app logs.
  KV caches are cleared between apps.
- **Model integrity:** sha256 verified at install and at load. Unverified files are refused.
- **Network:** no outbound traffic except pack downloads the user starts, and those go to hosts
  listed in the catalog. A test fails CI if the hub opens a non-local socket.
- The llama-server child process listens on a random local port with a random key; only the hub
  talks to it.

### 6.5 Evaluation
- Each app ships a task suite: inputs, expected properties and pass thresholds. Examples from
  ABM Invoice Analytics: flag explanations that pass the grounding check, triage verdict
  accuracy, NL→SQL safety and correctness.
- `abm-ai eval --pack <pack> --suite <app>` produces scores. The catalog records them, and the hub
  only offers an app the packs that pass its thresholds.

## 7. First client: ABM Invoice Analytics

It already has most of the app-side harness, which becomes the reference for the harness library:

| Existing piece (Invoice Analytics repo) | Moves to / maps onto |
|---|---|
| `engine/invoice_analytics/ai/runtime.py`: runs llama-server, checks sha256 and licence | Hub supervisor and pack verification |
| `IA_LLM_BASE_URL`: an external OpenAI-compatible server | The connection path to the hub (via the SDK) |
| `ai/prompts/*_v1.md`: versioned prompts | Harness prompt registry |
| Explanation grounding check, citation validator, `sqlglot` NL→SQL checks | Harness output checks |
| `ai/guard.py`: AI code may write only to `ai_suggestions`/`jobs` | Stays in the app (an app-data guardrail) |
| `ai/agent/`: tool loop with ≤8 calls and a 120 s budget | Harness tool loop |
| `tests/ai_eval/bakeoff.py` | Evaluation runner and the app's task suite |
| Settings → Local AI readiness checklist | "Connected to ABM Local AI · model …", or "Install ABM Local AI" |

Integration steps:
1. Add the SDK.
2. Replace the built-in runtime with hub discovery and approval, keeping `IA_LLM_BASE_URL` for
   Ollama or LM Studio users.
3. Publish its task suite.
4. Declare `data class: sensitive`.

## 8. Technology (proposed)

| Part | Choice | Notes |
|---|---|---|
| Hub shell | Tauri 2 + Rust | Same stack as Invoice Analytics: small installer, tray icon, process supervision |
| Inference | llama.cpp `llama-server` (MIT) | Official CPU builds for Windows x64 and Linux x64 (every CPU variant in one package, chosen at runtime); macOS arm64 with Metal; other GPU builds later |
| Hub API layer | Rust (axum), in-process | Policy, auth, queueing and proxying to llama-server |
| SDK and harness | Python package + TypeScript package | Shared wire protocol and conformance tests |
| Installers | Windows: NSIS per-user `.exe` + MSI · macOS: `.dmg` · Linux: `.AppImage`, `.deb`, `.rpm` | Code signing: Azure Trusted Signing (to decide), Apple Developer ID with notarization, GPG for Linux packages |
| CI | GitHub Actions on Windows, macOS and Linux | Includes the network-socket test, a pack-verification test and an eval smoke test |

## 9. Milestones

1. **M0 – Spike (1 week).** Run llama-server with Qwen3 1.7B and 4B on a typical 8 GB Windows PC.
   Measure speed and RAM. Run Invoice Analytics' `bakeoff.py` to get baseline scores.
2. **M1 – Hub core.** Supervisor, local API proxy, `endpoint.json`, approval, per-app keys, policy
   enforcement, JSON-schema output, and a no-disk-prompts test.
3. **M2 – Packs.** Manifest format, verification, pack manager UI and catalog. Publish the
   default 4B pack.
4. **M3 – SDK and harness.** Python SDK (TypeScript after), prompt registry, output checks, tool
   loop, traces and the eval runner.
5. **M4 – First client.** Invoice Analytics runs on the hub; its task suite gates which packs it
   may use.
6. **M5 – Release.** Signed installers, IT policy file, docs, and a public release page for the
   hub and packs.

## 10. Success criteria

- A new ABM app gets approved and makes a schema-valid call in **≤ 20 lines** of SDK code.
- Installing the hub plus the default 4B pack takes **< 5 minutes** on a typical PC, with no terminal.
- On a 12 GB PC with no GPU: the first token arrives in **≤ 3 s** for a short (~400-token)
  prompt, and a 200-token explanation finishes in **≤ 20 s** on the default 4B pack.
- **Zero** prompts or outputs on disk, and **zero** non-local sockets (tested in CI).
- Invoice Analytics explanations pass the grounding check **≥ 95%** of the time on the chosen
  default pack.

## 11. Open questions

1. **Data classes:** should `sensitive` apps get stricter defaults (no audit detail at all, a
   restricted model list, never a shared cache)?
2. **Shared knowledge:** will apps ever share a retrieval index, or does each keep its own? The
   recommendation is per-app only, at least for sensitive data.
3. **Adapters:** fine-tune per-domain LoRA adapters, or stay with base models plus retrieval
   until evaluation shows a gap?
4. **Tray app or Windows service:** a tray app is simpler; a service suits IT-managed machines.
5. **GPUs:** CPU-only in v1, or NVIDIA (CUDA/Vulkan) and Apple Metal from the start?
6. **Updates:** who approves a new model version for an app: the user, IT, or ABM by publishing
   passing scores?
7. **Licences:** stay Apache-2.0/MIT only, or allow others with a warning?
8. **Third-party apps:** allow non-ABM apps to request approval, or keep the hub ABM-only in v1?
9. **Coexisting with Ollama or LM Studio:** can the hub use an existing one as a back end, or
   should apps connect to those directly?

## 12. Risks

| Risk | Mitigation |
|---|---|
| Small models are too weak for some tasks (multi-step triage, NL→SQL) | Gate by task-suite scores; route those tasks to the Standard pack; keep deterministic fallbacks |
| Memory pressure when several apps want different models | One model at a time with a queue in v1; apps state a minimum pack; clear "busy" states |
| A local process impersonates an app | Per-app keys in OS-protected storage, user approval, and revocation; data classes limit exposure |
| Model files are large to host and download | Packs under 2 GB on GitHub; larger ones on Hugging Face; resumable downloads with verification |
| An unsigned installer triggers SmartScreen warnings | Code signing before public release |
| llama.cpp moves fast | Pin a tested build per hub release; run conformance tests before upgrading |
