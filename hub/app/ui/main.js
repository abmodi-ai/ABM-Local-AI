const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const LABELS = {
  stopped: "Stopped", starting: "Loading model…", ready: "Ready",
  restarting: "Restarting…", stopping: "Stopping…", failed: "Failed",
};

function showError(msg) {
  $("error").hidden = !msg;
  $("error").textContent = msg || "";
}

async function refresh() {
  const s = await invoke("hub_status");
  const st = s.server.state;
  $("badge").textContent = LABELS[st] + (st === "restarting" ? ` (attempt ${s.server.attempt})` : "");
  $("badge").className = "badge " + st;
  $("model").textContent = s.model || "No model selected";
  $("send").disabled = st !== "ready";
  if (st === "failed") showError(s.server.error);
  const facts = {
    Platform: `${s.platform} ${s.arch}`,
    "Hub version": s.version,
    "llama.cpp": s.llama_build || "not found",
    "Model server": st === "ready" ? `127.0.0.1:${s.server.port} (pid ${s.server.pid})` : "not running",
    "Packs folder": s.data_dir + (s.platform === "windows" ? "\\packs" : "/packs"),
  };
  $("facts").replaceChildren(...Object.entries(facts).flatMap(([k, v]) => {
    const dt = document.createElement("dt"); dt.textContent = k;
    const dd = document.createElement("dd"); dd.textContent = v;
    return [dt, dd];
  }));
}

$("start").onclick = async () => {
  showError("");
  try { await invoke("start_model", { path: $("model-path").value || null }); }
  catch (e) { showError(String(e)); }
  refresh();
};

$("stop").onclick = async () => { await invoke("stop_model"); refresh(); };

$("send").onclick = async () => {
  $("send").disabled = true;
  $("timing").textContent = "Generating…";
  $("answer").hidden = true;
  try {
    const r = await invoke("test_prompt", { prompt: $("prompt").value });
    $("answer").textContent = r.text;
    $("answer").hidden = false;
    const tps = r.tokens_per_s ? `, ${r.tokens_per_s.toFixed(1)} tokens/s` : "";
    $("timing").textContent = `${r.tokens} tokens in ${(r.total_ms / 1000).toFixed(1)} s${tps}`;
  } catch (e) {
    $("timing").textContent = "";
    showError(String(e));
  }
  refresh();
};

refresh();
setInterval(refresh, 1000);
