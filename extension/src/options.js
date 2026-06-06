const DEFAULTS = { endpoint: "http://127.0.0.1:48710", token: "", enabled: true };

const $ = (id) => document.getElementById(id);
const statusEl = $("status");

function setStatus(msg, kind) {
  statusEl.textContent = msg;
  statusEl.className = `status ${kind || ""}`;
}

async function load() {
  const o = await chrome.storage.local.get(DEFAULTS);
  $("endpoint").value = o.endpoint || DEFAULTS.endpoint;
  $("token").value = o.token || "";
  $("enabled").checked = o.enabled !== false;
}

async function save() {
  const endpoint = $("endpoint").value.trim().replace(/\/+$/, "") || DEFAULTS.endpoint;
  const token = $("token").value.trim();
  const enabled = $("enabled").checked;
  await chrome.storage.local.set({ endpoint, token, enabled });
  try {
    await chrome.runtime.sendMessage({ action: "optionsUpdated" });
  } catch {
    /* worker asleep */
  }
  setStatus("Saved.", "ok");
}

async function test() {
  const endpoint = $("endpoint").value.trim().replace(/\/+$/, "") || DEFAULTS.endpoint;
  const token = $("token").value.trim();
  setStatus("Testing…", "");
  try {
    const health = await fetch(`${endpoint}/health`);
    if (!health.ok) throw new Error(`health ${health.status}`);
    const cfg = await fetch(`${endpoint}/config`, { headers: { "X-Tempo-Token": token } });
    if (cfg.status === 401) {
      setStatus("Reached the app, but the token was rejected. Re-copy it from the app.", "warn");
      return;
    }
    if (!cfg.ok) throw new Error(`config ${cfg.status}`);
    const json = await cfg.json();
    setStatus(
      `Connected ✓  Capture is ${json.capturePageContent ? "ON" : "OFF"}, ` +
        `${(json.domainRules || []).length} domain rules.`,
      "ok"
    );
  } catch (e) {
    setStatus(`Could not reach the app at ${endpoint}. Is Tempo running? (${e})`, "warn");
  }
}

$("save").addEventListener("click", save);
$("test").addEventListener("click", test);
load();
