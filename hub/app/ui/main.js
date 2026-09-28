const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const gb = (bytes) => bytes / 2 ** 30;
const fmtGb = (v) => (v >= 10 ? v.toFixed(0) : v.toFixed(1)) + " GB";
const fmtRate = (bps) => (bps >= 2 ** 20 ? (bps / 2 ** 20).toFixed(1) + " MB/s" : Math.round(bps / 1024) + " KB/s");
function fmtEta(sec) {
  if (!isFinite(sec) || sec <= 0) return "";
  if (sec < 60) return "under a minute left";
  if (sec < 3600) return `about ${Math.round(sec / 60)} min left`;
  return `about ${(sec / 3600).toFixed(1)} h left`;
}

let lib = null;
const armed = new Map(); // id -> timeout for the two-step "Remove" confirmation

function showError(msg) {
  $("error").hidden = !msg;
  $("error").textContent = msg || "";
}

async function act(cmd, args) {
  showError("");
  try {
    await invoke(cmd, args);
  } catch (e) {
    showError(String(e));
  }
  refresh();
}

// ---- Your computer ----
function renderPc(hw) {
  const tiles = [
    ["Memory", `${Math.round(hw.total_ram_gb)} GB`, "RAM"],
    ["Processor", `${hw.cpu_cores} cores`, hw.cpu_name],
    ["Graphics", hw.gpu ? "Supported GPU" : "Processor only", hw.gpu || "Models run on the processor"],
    ["Free disk space", fmtGb(hw.free_disk_gb), hw.os],
  ];
  const html = tiles
    .map(([k, v, s]) => `<div class="tile"><div class="k">${esc(k)}</div><div class="v">${esc(v)}</div><div class="s" title="${esc(s)}">${esc(s)}</div></div>`)
    .join("");
  if ($("pc").dataset.html !== html) {
    $("pc").innerHTML = html;
    $("pc").dataset.html = html;
  }
}

// ---- Status pill and Try it ----
function renderStatus(l) {
  const st = l.server.state;
  const name = l.running ? l.running.name : null;
  let cls = "", text = "No model running";
  if (st === "ready" && name) [cls, text] = ["ready", `${name} is ready`];
  else if (st === "starting" || st === "restarting") [cls, text] = ["busy", name ? `Loading ${name}…` : "Loading…"];
  else if (st === "stopping") [cls, text] = ["busy", "Stopping…"];
  else if (st === "failed") [cls, text] = ["failed", "The model couldn't start"];
  $("status").className = "status " + cls;
  $("status-text").textContent = text;
  if (st === "failed" && l.server.error) showError(l.server.error);

  const chatReady = st === "ready" && l.running && l.running.kind === "chat";
  $("try").hidden = !chatReady;
  if (chatReady) $("try-title").textContent = `Try ${name}`;
  $("about").textContent = `Hub ${l.version} · llama.cpp ${l.llama_build || "not found"}`;
}

// ---- Library cards ----
function tipHtml(m) {
  const rows = m.compatibility.checks
    .map((c) => `<li class="${c.ok ? "yes" : "no"}"><span class="mark">${c.ok ? "✓" : "✗"}</span><span><b>${esc(c.label)}</b>: needs ${esc(c.required)}, this computer has ${esc(c.actual)}</span></li>`)
    .join("");
  return `<div class="tip" role="tooltip" id="tip-${esc(m.id)}"><div class="tip-title">Needs a more powerful computer</div><ul>${rows}</ul><div class="min">Minimum to run it comfortably: ${esc(m.compatibility.minimum)}</div></div>`;
}

function actionsHtml(m, l) {
  const d = m.download;
  const id = esc(m.id);
  const rm = armed.has(m.id)
    ? `<button class="danger" data-act="remove" data-id="${id}">Confirm remove</button>`
    : `<button class="secondary" data-act="arm" data-id="${id}">Remove</button>`;
  if (m.installed) {
    const running = l.running && l.running.id === m.id && l.server.state !== "stopped";
    if (m.active && running) return `<button class="secondary" data-act="stop">Stop</button>${rm}`;
    return `<button data-act="use" data-id="${id}">Use this model</button>${rm}`;
  }
  if (!m.compatibility.ok) return `<button disabled aria-describedby="tip-${id}">Not available</button>`;
  if (!d) return `<button data-act="download" data-id="${id}">Download</button>`;
  if (d.state === "downloading") return `<button class="secondary" data-act="pause" data-id="${id}">Pause</button>`;
  if (d.state === "verifying") return `<button disabled>Checking…</button>`;
  if (d.state === "paused") return `<button data-act="download" data-id="${id}">Resume</button>${rm}`;
  if (d.state === "failed") return `<button data-act="download" data-id="${id}">Try again</button>${rm}`;
  return "";
}

function progressHtml(m) {
  const d = m.download;
  if (!d) return "";
  if (d.state === "verifying") return `<div class="progress"><progress aria-label="Checking the file"></progress><span class="small">Checking the file is intact…</span></div>`;
  const pct = d.total ? Math.min(100, (100 * d.downloaded) / d.total) : 0;
  let line = `${pct.toFixed(0)}% · ${fmtGb(gb(d.downloaded))} of ${fmtGb(gb(d.total))}`;
  if (d.state === "downloading" && d.bytes_per_s > 0) line += ` · ${fmtRate(d.bytes_per_s)} · ${fmtEta((d.total - d.downloaded) / d.bytes_per_s)}`;
  if (d.state === "downloading" && !d.bytes_per_s) line += " · connecting…";
  if (d.state === "paused") line += " · paused";
  const err = d.state === "failed" ? `<span class="err">${esc(d.error)}</span>` : "";
  return `<div class="progress"><progress max="100" value="${pct.toFixed(1)}" aria-label="Download progress"></progress><span class="small">${line}</span>${err}</div>`;
}

// "12 GB RAM (this computer has 8 GB)"
function shortNeed(c) {
  const what = { "Memory (RAM)": "RAM", "Processor": "", "Graphics": "", "Free disk space": "free disk space" }[c.label] ?? c.label;
  if (c.label === "Graphics") return "an Apple Silicon GPU";
  const need = c.required.replace(" cores", " processor cores");
  const have = c.actual.replace(" cores", "");
  return `${need}${what ? " " + what : ""} (this computer has ${have})`;
}

function cardHtml(m, l) {
  const off = !m.compatibility.ok && !m.installed;
  const badges = [
    m.active && l.running && l.running.id === m.id ? `<span class="badge use">In use</span>` : "",
    m.recommended && !m.installed ? `<span class="badge rec">Recommended for you</span>` : "",
    m.installed && !m.active ? `<span class="badge inst">Downloaded</span>` : "",
  ].join("");
  const kind = m.kind === "embeddings" ? "For search" : "Chat";
  const fit = off
    ? `<div class="why">Needs ${m.compatibility.checks.filter((c) => !c.ok).map((c) => `${esc(shortNeed(c))}`).join(" and ")}</div>`
    : m.installed
      ? ""
      : `<div class="fit">✓ Runs well on this computer</div>`;
  return `
    <div class="card-head">
      <div><div class="name">${esc(m.name)}</div><div class="by">${esc(m.publisher)} · ${esc(m.license)} · ${kind}</div></div>
      <div class="badges">${badges}</div>
    </div>
    <p class="summary">${esc(m.summary)}</p>
    <div class="chips">${m.tags.map((t) => `<span class="chip">${esc(t)}</span>`).join("")}</div>
    ${fit}
    ${progressHtml(m)}
    <div class="card-foot"><span class="size">${fmtGb(m.download_gb)} download</span><div class="actions">${actionsHtml(m, l)}</div></div>
    ${off ? tipHtml(m) : ""}`;
}

function renderLibrary(l) {
  const root = $("library");
  const seen = new Set();
  l.models.forEach((m, i) => {
    seen.add(m.id);
    let el = root.querySelector(`[data-model="${CSS.escape(m.id)}"]`);
    if (!el) {
      el = document.createElement("article");
      el.dataset.model = m.id;
      root.appendChild(el);
    }
    if (root.children[i] !== el) root.insertBefore(el, root.children[i]);
    const off = !m.compatibility.ok && !m.installed;
    el.className = "card" + (off ? " off" : "") + (m.active ? " active" : "");
    el.tabIndex = off ? 0 : -1; // greyed cards are focusable so keyboard users can read why
    el.setAttribute("aria-label", off ? `${m.name}, not available on this computer` : m.name);
    const html = cardHtml(m, l);
    if (el.dataset.html !== html) {
      el.innerHTML = html; // only when something changed, so hover and focus survive polling
      el.dataset.html = html;
    }
  });
  [...root.children].forEach((el) => seen.has(el.dataset.model) || el.remove());
}

async function refresh() {
  try {
    lib = await invoke("library");
  } catch (e) {
    showError(String(e));
    return;
  }
  renderPc(lib.hardware);
  renderStatus(lib);
  renderLibrary(lib);
}

$("library").addEventListener("click", (ev) => {
  const b = ev.target.closest("button[data-act]");
  if (!b || b.disabled) return;
  const id = b.dataset.id;
  switch (b.dataset.act) {
    case "download": return act("download_model", { id });
    case "pause": return act("pause_download", { id });
    case "use": return act("use_model", { id });
    case "stop": return act("stop_model");
    case "arm":
      clearTimeout(armed.get(id));
      armed.set(id, setTimeout(() => { armed.delete(id); refresh(); }, 4000));
      return refresh();
    case "remove":
      clearTimeout(armed.get(id));
      armed.delete(id);
      return act("remove_model", { id });
  }
});

$("try-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  $("send").disabled = true;
  $("timing").textContent = "Thinking…";
  $("answer").hidden = true;
  try {
    const r = await invoke("test_prompt", { prompt: $("prompt").value });
    $("answer").textContent = r.text;
    $("answer").hidden = false;
    const tps = r.tokens_per_s ? ` · about ${Math.round(r.tokens_per_s * 0.75)} words per second` : "";
    $("timing").textContent = `Answered in ${(r.total_ms / 1000).toFixed(1)} s${tps}`;
  } catch (e) {
    $("timing").textContent = "";
    showError(String(e));
  }
  $("send").disabled = false;
});

refresh();
setInterval(refresh, 1000);
