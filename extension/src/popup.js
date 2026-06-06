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

  if (status.config) {
    const c = status.config;
    detail.innerHTML =
      `<div>Page capture: <b>${c.capturePageContent ? "ON" : "OFF"}</b></div>` +
      `<div>Store raw text: <b>${c.storeRawText ? "ON" : "OFF"}</b></div>` +
      `<div>Domain rules: <b>${(c.domainRules || []).length}</b></div>`;
  } else {
    detail.textContent = "";
  }
}

async function refresh() {
  try {
    const status = await chrome.runtime.sendMessage({ action: "getStatus" });
    if (status) render(status);
  } catch {
    $("status-card").textContent = "Background worker starting… reopen the popup.";
  }
}

$("enabled").addEventListener("change", async (e) => {
  await chrome.storage.local.set({ enabled: e.target.checked });
  try {
    await chrome.runtime.sendMessage({ action: "optionsUpdated" });
  } catch {
    /* ignore */
  }
});

$("options").addEventListener("click", () => chrome.runtime.openOptionsPage());

refresh();
