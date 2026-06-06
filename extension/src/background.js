// Tempo background service worker.
//
// Receives 10s "ticks" from the focused tab's content script, pulls capture
// policy from the local app (GET /config), optionally requests readable text,
// and POSTs one activity record to the local app (POST /ingest). Loopback only.

import { detectContentType, summarize } from "./summarize.js";

const DEFAULTS = { endpoint: "http://127.0.0.1:48710", token: "", enabled: true };
let configCache = { at: 0, value: null };

async function getOptions() {
  const o = await chrome.storage.local.get(DEFAULTS);
  return { ...DEFAULTS, ...o };
}

async function setBadge(state) {
  const map = { ok: ["", "#16a34a"], warn: ["!", "#dc2626"], off: ["॥", "#94a3b8"] };
  const [text, color] = map[state] || map.warn;
  try {
    await chrome.action.setBadgeText({ text });
    await chrome.action.setBadgeBackgroundColor({ color });
  } catch {
    /* no action surface */
  }
}

function originOf(url) {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.host}`;
  } catch {
    return url;
  }
}

function registrableDomain(url) {
  try {
    return new URL(url).hostname.replace(/^www\./, "").toLowerCase();
  } catch {
    return "";
  }
}

// Mirror of the app's built-in "never capture" policy (defence in depth).
function builtinBlocked(domain) {
  const d = domain.toLowerCase();
  if (d.endsWith(".gov") || d.endsWith(".bank") || d.endsWith(".gov.uk")) return true;
  const needles = [
    "bank", "paypal", "stripe", "venmo", "wallet", "chase", "wellsfargo",
    "citibank", "barclays", "hsbc", "santander", "coinbase",
    "1password", "lastpass", "bitwarden", "dashlane", "keeper",
    "mychart", "patient", "medicare", "medicaid", "nhs", "healthcare",
  ];
  if (needles.some((n) => d.includes(n))) return true;
  const mail = [
    "mail.google.com", "outlook.live.com", "outlook.office.com",
    "mail.proton.me", "mail.yahoo.com", "mail.aol.com",
  ];
  return mail.includes(d);
}

async function getConfig(opts) {
  const now = Date.now();
  if (configCache.value && now - configCache.at < 60000) return configCache.value;
  if (!opts.token) return null;
  try {
    const res = await fetch(`${opts.endpoint}/config`, {
      headers: { "X-Tempo-Token": opts.token },
    });
    if (!res.ok) throw new Error(`config ${res.status}`);
    const cfg = await res.json();
    configCache = { at: now, value: cfg };
    return cfg;
  } catch {
    return configCache.value; // last-known-good, if any
  }
}

function effectiveMode(cfg, domain) {
  if (builtinBlocked(domain)) return "never";
  const rule = (cfg.domainRules || []).find((r) => r.domain === domain);
  return rule ? rule.captureMode : "meta";
}

async function handleTick(tab) {
  const opts = await getOptions();
  if (!opts.enabled) return setBadge("off");
  if (!opts.token) return setBadge("warn");
  if (!tab || !tab.url || !/^https?:/.test(tab.url)) return;

  const cfg = await getConfig(opts);
  if (!cfg) return setBadge("warn");

  const domain = registrableDomain(tab.url);
  if (!domain) return;

  let idle = false;
  try {
    const state = await chrome.idle.queryState(Math.max(15, cfg.idleSeconds || 60));
    idle = state !== "active";
  } catch {
    /* idle API unavailable */
  }

  const mode = effectiveMode(cfg, domain);
  const record = {
    domain,
    url: mode === "never" ? originOf(tab.url) : tab.url,
    pageTitle: tab.title || "",
    timestamp: new Date().toISOString(),
    durationSeconds: cfg.sampleSeconds || 10,
    isIdle: idle,
    contentType: detectContentType(tab.url, null),
  };

  const allowText = cfg.capturePageContent && mode === "text" && !idle;
  if (allowText) {
    try {
      const extracted = await chrome.tabs.sendMessage(tab.id, {
        action: "extract",
        maxLength: cfg.maxTextLength || 8000,
      });
      if (extracted && !extracted.error && extracted.text) {
        const { summary, keywords } = summarize(extracted);
        record.contentType = detectContentType(tab.url, extracted.flags);
        record.contentSummary = summary;
        record.detectedKeywords = keywords;
        if (cfg.storeRawText) record.rawTextExcerpt = extracted.text;
      }
    } catch {
      // content script not reachable (loading / restricted page) — meta only
    }
  }

  await postIngest(opts, record);
}

async function postIngest(opts, record) {
  try {
    const res = await fetch(`${opts.endpoint}/ingest`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Tempo-Token": opts.token },
      body: JSON.stringify(record),
    });
    await setBadge(res.ok ? "ok" : "warn");
  } catch {
    await setBadge("warn"); // desktop app not running
  }
}

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (!msg) return;
  if (msg.action === "tick") {
    handleTick(sender.tab);
    return;
  }
  if (msg.action === "optionsUpdated") {
    configCache = { at: 0, value: null };
    return;
  }
  if (msg.action === "getStatus") {
    (async () => {
      const opts = await getOptions();
      let reachable = false;
      try {
        const r = await fetch(`${opts.endpoint}/health`);
        reachable = r.ok;
      } catch {
        /* app down */
      }
      const config = await getConfig(opts);
      sendResponse({ opts, reachable, config });
    })();
    return true; // async response
  }
});

chrome.runtime.onInstalled.addListener(async () => {
  const o = await chrome.storage.local.get(DEFAULTS);
  await chrome.storage.local.set({ ...DEFAULTS, ...o });
  setBadge(o.token ? "ok" : "warn");
});
