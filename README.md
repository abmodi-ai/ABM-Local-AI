# ABM Local AI

A private AI hub that runs on your own computer. One download provides local AI models to
every app that needs them, so apps don't each bundle their own. Nothing is sent to the cloud.

```
  ABM Invoice Analytics ─┐                          ┌─ Model packs (separate downloads)
  Future app #2 ─────────┼──► ABM Local AI (hub) ───┤   small · standard · embeddings
  Future app #3 ─────────┘    this computer only    └─ added and removed in the hub
```

- **One hub, many apps.** Apps find the hub automatically and ask to connect. You approve each
  app once and can revoke it at any time.
- **Models are packs.** Install the models you want as separate downloads. Each pack is
  checksum-verified and openly licensed.
- **Private by design.** The hub listens only on this computer, stores no prompts, keeps apps
  isolated from each other and sends no telemetry. The only internet use is a model download
  you start yourself.
- **Guardrails outside the model.** The hub enforces a baseline policy for every app, and a
  shared harness library gives each app its domain checks. The model proposes; code decides.
- **Standard interface.** It exposes an OpenAI-compatible API on the local machine, so tools you
  didn't write can use it too.

**Platforms:** Windows 10/11 (primary), macOS on Apple Silicon, and Linux (Ubuntu 22.04+,
Debian 12+, Fedora 39+). One codebase builds a native installer for each: `.exe`/`.msi`, `.dmg`,
and `.AppImage`/`.deb`/`.rpm`. Model packs are the same file on every OS.

Status: **M0 spike**. See the [project brief](docs/project-brief.md), the
[implementation plan](docs/implementation-plan.md) and the [spike results](docs/spike-results.md).

## Repository layout

```
hub/core/        Rust: llama-server supervisor + per-OS platform layer, abm-hub-smoke test binary
hub/app/         Tauri 2 shell: tray, status window; bundles llama.cpp from resources/llama/
runtime/         llama.lock.json: the pinned llama.cpp build and its sha256 per target
scripts/         fetch_llama.py, fetch_model.py (pinned, verified downloads); app_launch_check.py
bench/           models.json (candidate packs) and bench.py (M0 speed/RAM benchmark)
```

## Developer quick start (any OS)

Requires Rust (stable), Python 3.9+, Node (only for `npx @tauri-apps/cli`) and, on Linux, the
[Tauri system packages](https://v2.tauri.app/start/prerequisites/).

```sh
python scripts/fetch_llama.py            # pinned llama.cpp for this OS -> hub/app/resources/llama/
python scripts/fetch_model.py ci-tiny    # 105 MB test model (add: small standard)

# Headless smoke test: start, schema-valid JSON, API key, loopback-only, orphan kill, clean stop
cargo run --manifest-path hub/Cargo.toml -p abm-hub-core --bin abm-hub-smoke -- \
  --llama-dir hub/app/resources/llama --model .cache/models/SmolLM2-135M-Instruct-Q4_K_M.gguf

# Run the hub app with a model
ABM_MODEL=.cache/models/SmolLM2-135M-Instruct-Q4_K_M.gguf \
  ABM_LLAMA_DIR=hub/app/resources/llama cargo run --manifest-path hub/Cargo.toml -p abm-local-ai

# Build this OS's installers
cd hub/app && npx @tauri-apps/cli@2 build

# Benchmark candidate packs (psutil gives peak RAM on Windows)
python bench/bench.py small standard [--cpu-only --threads 4]
```

On Windows PowerShell, set the variables with `$env:ABM_MODEL="..."` instead.

## Licence

Apache License 2.0; see [LICENSE](LICENSE) and [NOTICE](NOTICE). The installers bundle llama.cpp
(MIT). Model packs are separate downloads under their own licences (Apache-2.0 or MIT only in v1).
