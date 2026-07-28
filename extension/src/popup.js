const tempoBrowser = globalThis.browser || globalThis.chrome;
const $ = (id) => document.getElementById(id);

function render(status) {
  const card = $("status-card");
  const detail = $("detail");
  $("enabled").checked = status.opts.enabled !== false;

  if (!status.opts.token) {
    card.textContent = "Not connected — add the token in settings.";
    card.className = "pop-status warn";
  } else if (!status.reachable) {
    card.textContent = "App not reachable. Is Tempo running?";
    card.className = "pop-status warn";
  } else {
    card.textContent = "Connected to Tempo ✓";
    card.className = "pop-status ok";
  }

  detail.textContent = "";
  if (status.config) {
    const c = status.config;
    const rows = [
      ["Page capture: ", c.capturePageContent ? "ON" : "OFF"],
      ["Store raw text: ", c.storeRawText ? "ON" : "OFF"],
      ["Domain rules: ", String((c.domainRules || []).length)],
    ];
    for (const [label, value] of rows) {
      const row = document.createElement("div");
      const strong = document.createElement("b");
      row.append(document.createTextNode(label));
      strong.textContent = value;
      row.append(strong);
      detail.append(row);
    }
  }
}

async function refresh() {
  try {
    const status = await tempoBrowser.runtime.sendMessage({ action: "getStatus" });
    if (status) render(status);
  } catch {
    $("status-card").textContent = "Background worker starting… reopen the popup.";
  }
}

$("enabled").addEventListener("change", async (e) => {
  await tempoBrowser.storage.local.set({ enabled: e.target.checked });
  try {
    await tempoBrowser.runtime.sendMessage({ action: "optionsUpdated" });
  } catch {
    /* ignore */
  }
});

$("options").addEventListener("click", () => tempoBrowser.runtime.openOptionsPage());

refresh();
