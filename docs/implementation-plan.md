# ABM Local AI — Implementation Plan

## Context
The folder has only `README.md` and `docs/project-brief.md`. It is not a git repo yet. The brief defines the goal: a desktop hub (Tauri 2 + Rust) that supervises llama.cpp `llama-server` and serves local, OpenAI-compatible AI to every ABM app, with privacy enforced by policy. Model packs are installed separately. The first client is ABM Invoice Analytics. This plan turns the brief's milestones M0–M5 into concrete work. It also answers the user's question: do we need one download or two (Mac vs Windows)?

**Platform scope (decided):** Windows, macOS and Linux are all supported from v1. **Windows is the primary platform**, because most early users are on it. The reference machine, the performance targets and release sign-off are all Windows-first, but no release ships unless all three platforms pass.

## Answer: one codebase, separate installers per OS, one download button
- **One codebase and one CI pipeline** build every platform. Tauri 2, Rust, axum and the SDKs are all cross-platform, so about 95% of the code is shared.
- **Separate installer files per OS are unavoidable.** The hub and `llama-server` are native machine code, and no OS can run another's binaries.
  - Windows x64: `.exe` (NSIS, per-user) and `.msi` (for IT).
  - macOS: `.dmg`.
  - Linux x64: `.AppImage` (runs on any distro) plus `.deb` (Ubuntu/Debian) and `.rpm` (Fedora/RHEL).
- **The user still sees one download.** A single download page detects the OS and offers the matching installer, with an "other platforms" link.
- **Model packs are the same file on every OS.** GGUF plus the manifest is platform-independent, so the large downloads (1–20 GB) are never duplicated per OS.
- **Why not "one file for every OS":**
  - Electron, Python and Docker all still need a native llama.cpp build per OS.
  - Docker also breaks the brief's success criterion of "< 5 min, no terminal" for end users.
  - Per-OS installers built from one pipeline is the standard approach, and it costs almost nothing extra.

### Thin platform layer (the only OS-specific code)
Isolated in `hub/src/platform/{windows,macos,linux}.rs` behind one trait:

| Concern | Windows (primary) | macOS | Linux |
|---|---|---|---|
| `endpoint.json` location | `%LOCALAPPDATA%\ABM\LocalAI\` | `~/Library/Application Support/ABM/LocalAI/` | `$XDG_DATA_HOME/abm/localai/` (default `~/.local/share/abm/localai/`) |
| SDK key storage | DPAPI / Credential Manager | Keychain | Secret Service (libsecret: GNOME Keyring / KWallet). Falls back to a 0600 key file, with a warning in the UI, on headless systems |
| llama-server dies with the hub | Job Object (kill-on-close) | process group + parent-death watchdog | `prctl(PR_SET_PDEATHSIG)` + process group |
| Bundled llama-server build | official `win-cpu-x64` build: one package with every CPU variant (SSE4.2 up to Zen4), picked by ggml at runtime *(M0 finding)* | arm64 with Metal (GPU acceleration comes free on Apple Silicon) | official `ubuntu-x64` build, same runtime CPU variants; needs glibc 2.34+, `libgomp`, OpenSSL 3 *(M0 finding)*; Vulkan build later |
| Sidecar packaging | Tauri `resources` folder `llama/` (llama-server needs ~10–20 shared libraries, so `externalBin` doesn't fit) *(M0 finding)* | same | same |
| Installer | NSIS `.exe` + MSI | `.dmg` | `.AppImage` + `.deb` + `.rpm` |
| Code signing | Azure Trusted Signing (avoids SmartScreen) | Apple Developer ID + **notarization** (without it, Gatekeeper blocks the app) | GPG-signed `.deb`/`.rpm` and AppImage, with checksums published |
| Tray / autostart | Tauri tray + Run key | Tauri tray + LaunchAgent / login item | Tauri tray (AppIndicator; stock GNOME needs the AppIndicator extension, so the main window must work without a tray) + XDG autostart `.desktop` |

## Repository layout (new)
```
ABM-Local-AI/
  hub/                 Tauri 2 app: Rust core (axum API, supervisor, policy, pack manager) + small web UI
    src/platform/      the OS-specific trait implementations above
    app/resources/llama/  pinned llama.cpp for the build's OS (fetched and verified by scripts/fetch_llama.py, not committed)
  runtime/llama.lock.json  pinned llama.cpp build + sha256 per target
  protocol/            wire spec (OpenAPI for /v1 + /hub/v1, endpoint.json + manifest JSON schemas)
    conformance/       language-neutral test cases that both SDKs and the hub must pass
  sdk-python/          abm-localai: discovery, pairing, key storage, client, harness, eval runner (CLI `abm-ai`)
  sdk-ts/              @abm/localai: same surface (after Python)
  packs/               pack manifests, catalog.json, packaging scripts
  docs/                existing brief + architecture decision records
  .github/workflows/   CI matrix: windows-latest, macos-latest (arm64), ubuntu-22.04
```

## Milestones

### M0 — Spike (1–1.5 weeks), on all three OSes
- Run a pinned llama-server build with Qwen3 1.7B Q4 and 4B Q4:
  - on an 8 GB, CPU-only Windows PC;
  - on an Apple Silicon Mac;
  - on an Ubuntu 22.04 x64 machine (8 GB, CPU-only).
- Record first-token latency, tokens/s and peak RAM against the section 10 targets (≤ 3 s first token, ≤ 20 s for a 200-token answer).
- Run Invoice Analytics' `tests/ai_eval/bakeoff.py` against both models to get baseline scores.
- Minimal Tauri 2 skeleton that launches the sidecar on all three OSes, to prove packaging of the llama.cpp folder (M0 found `externalBin` unsuitable: llama-server needs its shared libraries alongside it). On Linux, also check the tray on stock GNOME and KDE, and check that the AppImage runs on the oldest supported distro (glibc baseline).
- **Exit:** a results table in `docs/spike-results.md`, and the Small/Standard defaults confirmed or changed.

**M0 status (2026-09-27):** repo scaffolded; the Mac spike is done; CI is written for all three OSes. Windows and Linux benchmark numbers are still needed on real 8 GB machines. See [spike-results.md](spike-results.md).

### M1 — Hub core
1. Supervisor: start, health-check, restart on crash, idle unload, kill-with-parent (platform layer). llama-server runs on a random port with a random key.
2. axum API on 127.0.0.1 only, with Host/Origin checks. Proxies `/v1/chat/completions` (streaming, `response_format` json_schema, tools), `/v1/embeddings` and `/v1/models`.
3. `endpoint.json` writer. Pairing flow `/hub/v1/pair` sends an Allow/Deny prompt to the tray UI, then issues a per-app key (stored hashed) that can be revoked.
4. Policy engine that enforces each app's declared manifest:
   - allowed models and tools;
   - max tokens and timeouts;
   - concurrency;
   - data class.
5. Fair queue across apps. KV/prompt cache cleared between apps, and no shared cache for `sensitive` apps.
6. Metadata-only audit log (app, model, tokens, duration, result code).
7. `/hub/v1/status`.
- **Tests, run in CI on all three OSes:**
  - no-prompts-on-disk (scan the data dir and temp files after a run);
  - no non-local sockets;
  - policy rejection cases;
  - pairing and revocation.

### M2 — Model packs
- Manifest JSON schema covering sha256, licence (Apache-2.0/MIT only), minimum RAM, context length, capabilities and scores.
- `catalog.json`:
  - packs under 2 GB hosted on GitHub releases;
  - larger packs linked to Hugging Face;
  - downloads only from hosts the catalog lists.
- Resumable download, with sha256 checked at install and again at load. Unverified files are refused.
- Pack manager UI: install, remove, set default, RAM warnings.
- Publish the "Small" pack.
- **Exit:** a fresh machine installs the hub plus the Small pack in under 5 minutes, with no terminal.

### M3 — SDK + harness (Python first, then TypeScript)
- **SDK:** discovery via `endpoint.json`, pairing, OS-protected key storage, an OpenAI-compatible client, retries, and an "AI unavailable" state.
- **Harness:**
  - versioned prompt registry;
  - JSON-schema output helpers;
  - tool loop with step and time budgets;
  - output checks (grounding, citations, SQL safety via sqlglot);
  - traces.
- Port the existing Invoice Analytics pieces as the reference implementation: the prompt files, grounding check, citation validator, agent loop, and `bakeoff.py`, which becomes the eval runner.
- `abm-ai eval --pack <pack> --suite <app>` writes scores into the catalog and the app's allow-list.
- Both SDKs pass `protocol/conformance/`.
- **Exit:** a sample app gets paired and makes a schema-valid call in ≤ 20 lines.

### M4 — First client: Invoice Analytics
- Replace `engine/invoice_analytics/ai/runtime.py` with hub discovery and pairing through the SDK. Keep `IA_LLM_BASE_URL` as an Ollama / LM Studio fallback.
- Declare `data class: sensitive`. Publish its task suite, which gates which packs it may use.
- Settings shows "Connected to ABM Local AI · model …" or "Install ABM Local AI".
- **Exit:** grounding check passes on ≥ 95% of explanations on the chosen default pack.

### M5 — Release
- Signed and notarized installers built from a single tagged CI release:
  - Windows `.exe` + `.msi`;
  - macOS `.dmg`;
  - Linux `.AppImage`, `.deb`, `.rpm`, plus an optional APT/DNF repository so updates arrive through the package manager.
- Tauri updater with signed update manifests.
- IT policy file for silent install, pre-approved ABM apps and a fixed model list.
- Download page that detects the OS. User docs and SDK docs.

## Decisions needed (recommended default in bold)
1. **Intel Macs:** **Apple Silicon only in v1**, or a universal build (adds an x64 CPU-only llama-server and slower performance).
2. **Signing accounts:** an Apple Developer Program membership ($99/yr) and Azure Trusted Signing are both required before a public release. **Start both at M1**, because approval takes time. Linux needs only a project GPG key.
3. **GPU:** **Metal on Mac from the start (free); Windows and Linux CPU-only in v1**, with CUDA/Vulkan later.
4. **Linux distro baseline:** **Ubuntu 22.04+, Debian 12+, Fedora 39+** officially; others via AppImage on a best-effort basis.
5. The brief's open questions (§11), such as tray app vs service and third-party apps, can wait until M1/M5. None of them blocks M0.

## Verification
- **CI matrix (windows-latest, macos-latest, ubuntu-22.04), on every PR:**
  - Rust unit and integration tests;
  - socket test and no-disk-prompts test;
  - pack verification test (a tampered file is refused);
  - SDK conformance suite;
  - eval smoke test with a tiny model.
- **End-to-end manual check per OS, on clean VMs or machines** (Windows 10 and 11; macOS; Ubuntu with GNOME and Fedora with KDE):
  1. Download from the page and install without a terminal.
  2. Install the Small pack.
  3. Run the sample SDK app, approve it in the tray, get schema-valid JSON.
  4. Revoke the app and confirm the next call is rejected.
  5. Confirm no prompt text anywhere under the data directory.
- Performance: re-run the M0 benchmark on the release build. It must meet the section 10 latency and RAM targets on the reference 8 GB Windows PC (the primary gate), and must stay within 25% of those targets on the Linux reference machine.
