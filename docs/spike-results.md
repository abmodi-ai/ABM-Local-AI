# M0 spike results

Status: **packaging and process checks pass on all three OSes; speed and RAM on the Windows and Linux reference PCs are still to run.** Pinned llama.cpp: v0.5.0 (build b11146).
Candidates: Small = Qwen3 1.7B Q4_K_M (1.03 GB), Standard = Qwen3 4B Instruct 2507 Q4_K_M (2.33 GB).
Both are Apache-2.0.

## What M0 had to prove, and where each stands

| Question | Windows | macOS | Linux |
|---|---|---|---|
| Pinned llama.cpp downloads, verifies and stages | ✅ staged (40 MB, 24 files) | ✅ staged (24 MB, 12 files) | ✅ staged (37 MB, 23 files) |
| Core smoke test: start, schema JSON, key, loopback only, orphan kill, clean stop | ✅ CI | ✅ local + CI | ✅ CI (Ubuntu 22.04) |
| Installer bundles llama.cpp and the installed app launches it | ✅ CI: NSIS `/S` per-user install, app runs `AppData\Local\ABM Local AI\llama\llama-server.exe` | ✅ `.app` + 12 MB `.dmg` | ✅ CI: `.deb` installed, app runs `/usr/lib/ABM Local AI/llama/llama-server`; AppImage contains it; `.rpm` built |
| llama-server dies when the hub is force-killed | ✅ CI (Job Object; core and installed app) | ✅ watchdog | ✅ CI (`PR_SET_PDEATHSIG`; core and installed app) |
| Speed and RAM on an 8 GB, CPU-only reference PC | **to run** | proxy only (see below) | **to run** |
| Tray icon on stock GNOME and KDE | n/a | n/a | **to check by hand** |

"CI" means `.github/workflows/ci.yml` runs the check on that OS. The first run passed
everywhere on 2026-09-27 (run 36339578608). The installed app also serves only on loopback on
all three OSes. Installers from each run are kept as build artifacts for 14 days.

## Findings that changed the plan

1. **Package llama.cpp as a resource folder, not `externalBin`.** `llama-server` loads 10–20
   shared libraries from its own directory, and `externalBin` handles only single files.
   `scripts/fetch_llama.py` stages the server, its libraries and the licence. It swaps library
   symlinks for plain files, so installers and code signing handle them reliably.
2. **No separate AVX2 and fallback builds.** The official Windows and Linux CPU builds ship every
   CPU variant (SSE4.2 up to Zen4), and ggml picks one at runtime. One package covers old and new
   PCs.
3. **Linux baseline confirmed.** The official Linux build needs glibc 2.34+ and libstdc++ 3.4.30
   (GCC 12). That matches Ubuntu 22.04, Debian 12 and Fedora 39 exactly. It also needs `libgomp`
   and OpenSSL 3, which are now declared as `.deb`/`.rpm` dependencies. The AppImage relies on the
   host having them (to check in the Linux run).
4. **Keep the llama-server key out of argv.** A command-line argument is visible to every user in
   `ps`. The key now goes in via `LLAMA_API_KEY` in the child's environment, which only its owner
   can read.
5. **Schema-constrained output needs bounded strings.** Grammar-constrained JSON is always
   well-formed *until `max_tokens` cuts it off*. Unbounded string fields produced truncated
   (invalid) JSON. Adding `maxLength`/`maxItems` fixed it. The M3 harness should require bounds,
   or derive `max_tokens` from the schema.
6. **Measure private memory, not RSS. Keep weight repacking on.** On CPU, llama.cpp repacks the
   weights into a faster layout. RSS then counts both the repacked copy and the original model
   pages, so Standard looks like 6 GB. But the original pages are clean and file-backed: the OS
   can drop them once repacking is done. Private (dirty) memory shows the real cost. Mac, CPU
   only, 4 threads, 8k context, medians of 3 runs:

   | Pack | Setting | RSS | Private | Real working set | Gen tok/s | 200-token answer |
   |---|---|---|---|---|---|---|
   | Small | repack (default) | 3.13 GB | 2.05 GB | ≈ 2.1 GB | 43 | 11.9 s |
   | Small | `--no-repack` | 2.18 GB | 1.09 GB | ≈ 2.2 GB (weights stay mapped and in use) | 28 | 14.5 s |
   | Standard | repack (default) | 6.04 GB | 3.67 GB | ≈ 3.7 GB | 15 | 36.4 s |
   | Standard | `--no-repack` | 3.87 GB | 1.49 GB | ≈ 3.9 GB | 13 | 32.0 s |

   Repacking doesn't cost extra memory in practice, and it generates faster, so keep the
   default. What fits in 8 GB: Small needs about 2.1 GB and Standard about 3.7 GB, plus the OS
   and the user's apps. `bench.py` now records private memory:
   - Windows and Linux: USS via psutil;
   - macOS: `footprint`.

   An earlier draft of this document read RSS and wrongly concluded that repacking doubles RAM.
   Speeds on this machine vary by up to ±40% between sessions. For example, Standard's
   short-prompt first token took 3.0 s in one session and 5.3 s in another. The reference-PC
   runs should use `--runs 5`.
7. **First-token time depends on prompt length, not model load.** The brief's "≤ 3 s first token"
   has to name a prompt size. On CPU, a full evidence pack (~1,700 tokens) takes several times
   longer than a single flagged pair (~360 tokens). Levers for M1/M3:
   - keep evidence tight;
   - reuse the prompt cache for the fixed system prompt (per app only; never across `sensitive` apps);
   - stream tokens.

8. **Invoice Analytics' RAM tiers conflict with the 8 GB goal.** Its `ai/hardware.py`
   requires **≥ 15 GB RAM** for its lowest AI tier, which is the 4B model. The brief promises
   AI on an 8 GB PC. M4 must replace those tiers with the hub's pack requirements, measured on
   the Windows reference PC.
9. **macOS notarization must cover the bundled llama.cpp (M5 risk).** Tauri signed only the
   main executable. `llama-server` and its 10 dylibs under `Contents/Resources/llama/` are
   unsigned. Notarization requires every nested Mach-O file to be signed with a Developer ID
   and the hardened runtime. The release pipeline must sign them before Tauri seals the bundle.

## Mac results (development machine, not the reference PC)

The Mac is an Apple M5 Max with 128 GB of RAM, running macOS 26.6. It's far faster than the
target PC, so read the Metal row as "Apple Silicon works". The CPU-only, 4-thread row is only a
rough proxy for an ordinary PC; the real numbers must come from Windows.

**Metal (default on Apple Silicon)**

| Pack | Load | First token, 358-token prompt (≤ 3 s) | First token, 1,666-token prompt | 200-token answer (≤ 20 s) | Prompt tok/s | Gen tok/s | Schema valid | Peak RSS |
|---|---|---|---|---|---|---|---|---|
| Small | 0.41 s | 0.04 s ✅ | 0.19 s ✅ | 1.01 s ✅ | 9,098 | 253 | 3/3 | 2.08 GB |
| Standard | 0.41 s | 0.09 s ✅ | 0.41 s ✅ | 1.78 s ✅ | 4,091 | 145 | 3/3 | 3.67 GB |

**CPU only, 4 threads (rough PC proxy)**

| Pack | Load | First token, 358-token prompt (≤ 3 s) | First token, 1,666-token prompt | 200-token answer (≤ 20 s) | Prompt tok/s | Gen tok/s | Schema valid | Peak RSS |
|---|---|---|---|---|---|---|---|---|
| Small | 1.03 s | 1.39 s ✅ | 7.05 s ❌ | 11.5 s ✅ | 237 | 45 | 3/3 | 3.13 GB |
| Standard | 1.67 s | 3.04 s ❌ | 16.41 s ❌ | 24.42 s ❌ | 101 | 25 | 3/3 | 6.04 GB |

Medians of 3 runs after one warm-up, prompt cache off, 8,192-token context, repacking on. The
200-token answer uses the long prompt.

**Early read:**
- **Small** is the realistic CPU default. It meets the short-prompt and 200-token targets even on
  this proxy.
- **Standard** needs a GPU (Metal, or Vulkan/CUDA later) or a fast CPU.
- On a slower 8 GB Windows PC, expect both to be slower than this proxy.

## Invoice Analytics bakeoff baseline

Run on the Mac with Metal, using the pinned llama-server (the M5 Max is fast, so read these
as quality scores, not speed). Reduced set: 40 extraction documents,
80 triage pairs and 30 NL→SQL questions. Reports are in `bench/results/bakeoff-*/`, which is
not committed.

| Task | Metric | Small, thinking on (app default) | Small, thinking off | Standard (4B 2507, no thinking) | Bar (Invoice Analytics spec) |
|---|---|---|---|---|---|
| Extraction | field accuracy | aborted (see below) | **1.00** (p95 3.1 s) | **1.00** (p95 2.8 s) | ≥ 0.97 digital |
| Triage (tool loop) | Cohen's kappa | aborted | 0.00, **100% invalid** (p95 15 s) | −0.01, **97.5% invalid** (p95 48 s) | ≥ 0.6 |
| Triage | uncited verdicts shown as valid | — | 0 ✅ | 0 ✅ | 0 |
| NL→SQL | result accuracy | — | **0.80** (20% invalid) | **0.80** (20% invalid) | (none set) |

**Thinking mode matters for Small.** Qwen3 1.7B is a hybrid model that thinks by default, and
Invoice Analytics never turns thinking off. With thinking on, the model wrote malformed JSON
inside a tool call's arguments. llama-server returned **HTTP 500** ("Failed to parse tool call
arguments as JSON"), the app's client raised it, and the run aborted. With thinking off
(`--chat-template-kwargs '{"enable_thinking":false}'`), Small finished with no parse errors and
matched Standard on extraction and NL→SQL.

Consequences:
- **Pack manifests:** each pack must declare its thinking behaviour, and the hub must set the
  per-app default.
- **Hub (M1):** map upstream tool-call parse failures to a typed, retryable error, never a bare
  500.
- **Harness (M3):** treat such a failure as one failed step, not a crash.

**Triage fails for both models, which points at the triage harness, not model size.**
`INVALID` means either citations that fail validation or final JSON that doesn't parse. Both
models land there almost every time, including the 4B model that aces extraction. The verdict
schema allows a 1,500-character rationale plus up to 12 citations with unbounded strings, all
inside a 700-token budget. That's the same truncation risk as finding 5, and it's the first
thing to check. Strict citation matching is the second. **Not yet diagnosed.** This is
Invoice Analytics work for M4, and it has to be fixed before the triage bakeoff can rank models.

Two notes on these numbers:
- **Safeguard held.** The "0 uncited verdicts shown as valid" result shows the code-level
  guardrail works even when the model fails.
- **RAM figure.** The bakeoff reported 14.7 GB peak RAM, but it starts llama-server with its own
  settings (default parallel slots). The hub's settings (`--parallel 1`) gave 3.67 GB for the
  same model, so the bakeoff figure doesn't reflect hub memory use.

## Still to do to close M0

1. **Windows reference PC** (8 GB, CPU only, Windows 10 or 11):
   - `python scripts/fetch_llama.py`
   - `python scripts/fetch_model.py small standard`
   - `pip install psutil`
   - `python bench/bench.py small standard --runs 5`
   - compare the **Peak private** column, not RSS
2. **Linux reference machine** (Ubuntu 22.04, 8 GB): the same commands.
3. **Check the Linux tray** by hand on stock GNOME (no AppIndicator extension) and on KDE.
4. **Confirm or change** the Small/Standard defaults and the RAM tiers from
   steps 1–2. Then update the pack list in the brief (section 6.3).
