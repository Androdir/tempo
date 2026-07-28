import { useCallback, useEffect, useState } from "react";
import {
  deleteAllCapturedContent,
  deleteDomainRule,
  getAccountabilitySettings,
  getCategoryDefinitions,
  getDomainRules,
  getLlmSettings,
  getPrivacySettings,
  isTauri,
  pruneOldData,
  purgeRawContent,
  resetDatabase,
  setAccountabilitySetting,
  setDomainRule,
  setLlmSetting,
  setPrivacySetting,
  testOllamaConnection,
} from "../api";
import { previewToast } from "../components/AccountabilityLayer";
import SyncSettings from "../components/SyncSettings";
import { CAPTURE_MODE_META, captureModeMeta } from "../categories";
import type {
  AccountabilitySettings,
  CaptureMode,
  Category,
  CategoryDefinition,
  DomainRule,
  LlmSettings,
  OllamaTestResult,
  PrivacySettings as Settings,
} from "../types";

const CAPTURE_MODES: CaptureMode[] = ["text", "meta", "never"];

type SettingsSection = "tracking" | "content" | "connections" | "data";

function initialSettingsSection(): SettingsSection {
  if (typeof window === "undefined") return "tracking";
  const pending = window.sessionStorage.getItem("tempo_settings_section");
  if (pending === "content" || pending === "connections" || pending === "data") return pending;
  return "tracking";
}

export default function PrivacySettings() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [rules, setRules] = useState<DomainRule[]>([]);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [resetting, setResetting] = useState(false);
  const [section, setSection] = useState<SettingsSection>(initialSettingsSection);

  useEffect(() => {
    window.sessionStorage.removeItem("tempo_settings_section");
  }, []);

  const [maxLen, setMaxLen] = useState("8000");
  const [smartInterval, setSmartInterval] = useState("60");
  const [retention, setRetention] = useState("90");
  const [idleThreshold, setIdleThreshold] = useState("60");
  const [newDomain, setNewDomain] = useState("");
  const [newMode, setNewMode] = useState<CaptureMode>("meta");
  const [newCat, setNewCat] = useState<Category | "">("");

  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [llmUrl, setLlmUrl] = useState("http://localhost:11434");
  const [llmModel, setLlmModel] = useState("llama3.1:8b");
  const [testResult, setTestResult] = useState<OllamaTestResult | null>(null);
  const [testing, setTesting] = useState(false);

  const [acct, setAcct] = useState<AccountabilitySettings | null>(null);
  const [distractMin, setDistractMin] = useState("20");
  const [eodTime, setEodTime] = useState("21:00");

  const load = useCallback(async () => {
    try {
      const [s, r, l, a, c] = await Promise.all([
        getPrivacySettings(),
        getDomainRules(),
        getLlmSettings(),
        getAccountabilitySettings(),
        getCategoryDefinitions(),
      ]);
      setSettings(s);
      setMaxLen(String(s.maxTextLength));
      setSmartInterval(String(s.smartIntervalSeconds));
      setRetention(String(s.retentionDays));
      setIdleThreshold(String(s.idleThresholdSeconds));
      setRules(r);
      setCategories(c);
      setLlm(l);
      setLlmUrl(l.url);
      setLlmModel(l.model);
      setAcct(a);
      setDistractMin(String(a.distractionWarnMinutes));
      setEodTime(a.eodPopupTime);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const flash = (m: string) => {
    setStatus(m);
    window.setTimeout(() => setStatus(null), 2500);
  };

  async function toggle(key: string, value: boolean) {
    try {
      await setPrivacySetting(key, value ? "1" : "0");
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleAcct(key: string, value: boolean) {
    try {
      await setAccountabilitySetting(key, value ? "1" : "0");
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function commitAcct(key: string, value: string) {
    try {
      await setAccountabilitySetting(key, value);
      await load();
      flash("Saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function commitMaxLen() {
    try {
      await setPrivacySetting("max_text_length", maxLen);
      await load();
      flash("Saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function commitRetention() {
    try {
      await setPrivacySetting("retention_days", retention);
      await load();
      flash("Saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function commitIdleThreshold() {
    try {
      await setPrivacySetting("idle_threshold_seconds", idleThreshold);
      await load();
      flash("Saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function onPruneNow() {
    try {
      const n = await pruneOldData();
      await load();
      flash(`Pruned ${n} old row(s)`);
    } catch (e) {
      setError(String(e));
    }
  }

  async function commitInterval() {
    try {
      await setPrivacySetting("smart_interval_seconds", smartInterval);
      await load();
      flash("Saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleLlm(v: boolean) {
    try {
      await setLlmSetting("llm_enabled", v ? "1" : "0");
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function saveLlm() {
    try {
      await setLlmSetting("ollama_url", llmUrl.trim());
      await setLlmSetting("ollama_model", llmModel.trim());
      await load();
      flash("Saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function testLlm() {
    setTesting(true);
    setTestResult(null);
    try {
      setTestResult(await testOllamaConnection(llmUrl.trim(), llmModel.trim()));
    } catch (e) {
      setTestResult({ ok: false, message: String(e), models: [], modelAvailable: false });
    } finally {
      setTesting(false);
    }
  }

  async function saveRule(
    domain: string,
    category: Category | null,
    mode: CaptureMode,
    aiReview = false
  ) {
    try {
      await setDomainRule(domain.trim().toLowerCase(), category, mode, aiReview);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function addRule() {
    const d = newDomain.trim().toLowerCase();
    if (!d) return;
    await saveRule(d, newCat || null, newMode);
    setNewDomain("");
    setNewCat("");
    setNewMode("meta");
  }

  async function removeRule(domain: string) {
    try {
      await deleteDomainRule(domain);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function onPurge() {
    if (!confirm("Delete raw page text from all captured rows? Summaries and keywords are kept.")) return;
    const n = await purgeRawContent();
    await load();
    flash(`Removed raw text from ${n} row(s)`);
  }

  async function onDeleteAll() {
    if (!confirm("Delete ALL captured page content (raw text, summaries, keywords)? URLs/titles are kept.")) return;
    const n = await deleteAllCapturedContent();
    await load();
    flash(`Cleared content from ${n} row(s)`);
  }

  async function onResetDatabase() {
    if (!isTauri()) {
      flash("Database reset is available in the desktop app");
      return;
    }
    if (!confirm("Reset Tempo's local database? This deletes activity, projects, goals, rules, reviews, sync data and settings.")) return;
    if (!confirm("This cannot be undone. Start completely fresh?")) return;
    setResetting(true);
    try {
      const n = await resetDatabase();
      await load();
      flash(`Reset database (${n} row(s) removed)`);
    } catch (e) {
      setError(String(e));
    } finally {
      setResetting(false);
    }
  }

  function copy(text: string, label: string) {
    navigator.clipboard?.writeText(text).then(() => flash(`${label} copied`)).catch(() => {});
  }

  function selectSection(next: SettingsSection) {
    setSection(next);
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("main")?.scrollTo({ top: 0, behavior: "smooth" });
    });
  }

  if (error && !settings) {
    return (
      <>
        <Head />
        <div className="error-box">{error}</div>
      </>
    );
  }
  if (!settings) {
    return (
      <>
        <Head />
        <div className="loading">Loading settings…</div>
      </>
    );
  }

  return (
    <>
      <Head status={status} />
      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      <nav className="settings-tabs" aria-label="Settings sections">
        {([
          ["tracking", "Tracking"],
          ["content", "Browser & content"],
          ["connections", "Connections"],
          ["data", "Data & privacy"],
        ] as [SettingsSection, string][]).map(([id, label]) => (
          <button
            key={id}
            className={`settings-tab${section === id ? " active" : ""}`}
            onClick={() => selectSection(id)}
            aria-current={section === id ? "page" : undefined}
          >
            {label}
          </button>
        ))}
      </nav>

      <div hidden={section !== "connections"}>
        <SyncSettings />
      </div>

      {/* Accountability */}
      {acct && (
        <div className="card card-pad" hidden={section !== "tracking"}>
          <h2 className="card-title">Accountability</h2>
          <p className="card-hint">
            Local nudges only — distraction warnings and the end-of-day popup fire as desktop
            notifications from this device. Nothing is uploaded.
          </p>
          <SettingRow
            label="Distraction warnings"
            hint="Notify me when I spend too long, in one stretch, on a distracting app or site."
          >
            <Switch
              checked={acct.distractionWarnEnabled}
              onChange={(v) => toggleAcct("distraction_warn_enabled", v)}
            />
          </SettingRow>
          <SettingRow label="Warn after" hint="Minutes of continuous distraction before a nudge (1–240).">
            <span className="inline-edit">
              <input
                className="search"
                style={{ width: 90 }}
                type="number"
                min={1}
                max={240}
                value={distractMin}
                onChange={(e) => setDistractMin(e.target.value)}
                onBlur={() => commitAcct("distraction_warn_minutes", distractMin)}
                onKeyDown={(e) => e.key === "Enter" && commitAcct("distraction_warn_minutes", distractMin)}
              />
              <span className="muted-num">min</span>
              <button className="btn" onClick={() => commitAcct("distraction_warn_minutes", distractMin)}>
                Save
              </button>
            </span>
          </SettingRow>
          <SettingRow
            label="End-of-day review popup"
            hint="Pop up your daily review automatically at a set time."
          >
            <Switch
              checked={acct.eodPopupEnabled}
              onChange={(v) => toggleAcct("eod_popup_enabled", v)}
            />
          </SettingRow>
          <SettingRow label="Popup time" hint="24-hour HH:MM, local time.">
            <span className="inline-edit">
              <input
                className="search"
                style={{ width: 120 }}
                type="time"
                value={eodTime}
                onChange={(e) => setEodTime(e.target.value)}
                onBlur={() => commitAcct("eod_popup_time", eodTime)}
              />
              <button className="btn" onClick={() => commitAcct("eod_popup_time", eodTime)}>
                Save
              </button>
            </span>
          </SettingRow>
          <SettingRow label="Preview" hint="See what a distraction nudge looks like.">
            <button
              className="btn"
              onClick={() =>
                previewToast(
                  "distraction",
                  "Distraction check",
                  "You've been on Instagram for 20 minutes. Still intentional?",
                  "instagram.com",
                )
              }
            >
              Preview warning
            </button>
          </SettingRow>
        </div>
      )}

      {/* Smart Tracking (screen OCR) — most sensitive, shown first */}
      <div className="card card-pad smart-card section-gap" hidden={section !== "tracking"}>
        <h2 className="card-title">Personal Smart Tracking Mode</h2>
        <div className="warn-box">
          <span className="warn-ico">⚠️</span>
          <div>
            <b>This mode reads the visible text on your screen.</b> Every{" "}
            {settings.smartIntervalSeconds}s it captures the screen, runs OCR <b>locally</b> (no
            cloud), then <b>immediately deletes the screenshot</b> — the image is never written to
            disk. Only a short text summary, keywords and a classification are stored. Samples that
            look like they contain passwords, card numbers, or 2FA codes are discarded.
          </div>
        </div>
        <SettingRow
          label="Enable Smart Tracking"
          hint={
            settings.smartOcrAvailable
              ? "Off by default. Reads visible screen text via local OCR."
              : "Screen OCR is only available on Windows."
          }
        >
          <Switch
            checked={settings.smartTrackingEnabled}
            disabled={!settings.smartOcrAvailable}
            onChange={(v) => toggle("smart_tracking_enabled", v)}
          />
        </SettingRow>
        <SettingRow label="Capture interval" hint="How often the screen is OCR'd (10–3600 seconds).">
          <span className="inline-edit">
            <input
              className="search"
              style={{ width: 110 }}
              type="number"
              min={10}
              max={3600}
              value={smartInterval}
              onChange={(e) => setSmartInterval(e.target.value)}
              onBlur={commitInterval}
              onKeyDown={(e) => e.key === "Enter" && commitInterval()}
            />
            <button className="btn" onClick={commitInterval}>Save</button>
          </span>
        </SettingRow>
      </div>

      {/* Local AI classification (Ollama) */}
      {llm && (
        <div className="card card-pad section-gap" hidden={section !== "connections"}>
          <h2 className="card-title">Local AI classification (Ollama)</h2>
          <p className="card-hint">
            Optional. Uses a locally-running Ollama server to refine activity classification in the
            background. Only loopback / LAN addresses are allowed — nothing is sent to the cloud, and
            any error falls back to rule-based classification.
          </p>
          <SettingRow
            label="Enable local LLM classification"
            hint="Off by default. Requires Ollama running locally."
          >
            <Switch checked={llm.enabled} onChange={toggleLlm} />
          </SettingRow>
          <SettingRow label="Ollama URL" hint="Default http://localhost:11434">
            <input
              className="search"
              style={{ width: 240 }}
              value={llmUrl}
              onChange={(e) => setLlmUrl(e.target.value)}
              onBlur={saveLlm}
            />
          </SettingRow>
          <SettingRow label="Model" hint="e.g. llama3.1:8b or qwen2.5:7b">
            <span className="inline-edit">
              <input
                className="search"
                style={{ width: 160 }}
                value={llmModel}
                onChange={(e) => setLlmModel(e.target.value)}
                onBlur={saveLlm}
              />
              <button className="btn" onClick={saveLlm}>Save</button>
            </span>
          </SettingRow>
          <SettingRow label="Connection" hint="Check the app can reach Ollama and the model is installed.">
            <button className="btn" onClick={testLlm} disabled={testing}>
              {testing ? "Testing…" : "Test connection"}
            </button>
          </SettingRow>
          {testResult && (
            <div className={`llm-status ${testResult.ok ? (testResult.modelAvailable ? "ok" : "warn") : "bad"}`}>
              {testResult.message}
            </div>
          )}
          {!testResult && llm.lastError && (
            <div className="llm-status warn">Last error: {llm.lastError}</div>
          )}
        </div>
      )}

      {/* Capture master toggle */}
      <div className="card card-pad section-gap" hidden={section !== "content"}>
        <h2 className="card-title">Page content capture</h2>
        <p className="card-hint">
          Off by default. When off, only domain, URL and title are recorded — never page text.
        </p>
        <SettingRow
          label="Capture page content"
          hint="Allow the extension to read visible text on allowed domains."
        >
          <Switch checked={settings.capturePageContent} onChange={(v) => toggle("capture_page_content", v)} />
        </SettingRow>
      </div>

      {/* Storage */}
      <div className="card card-pad section-gap" hidden={section !== "content"}>
        <h2 className="card-title">What gets stored</h2>
        <p className="card-hint">Summaries &amp; keywords are kept; raw text is optional.</p>
        <SettingRow label="Store raw text excerpt" hint="Keep the raw extracted text (off = summary + keywords only).">
          <Switch checked={settings.storeRawText} onChange={(v) => toggle("store_raw_text", v)} />
        </SettingRow>
        <SettingRow
          label="Delete raw text after classification"
          hint="Discard raw text once keywords/summary are derived."
        >
          <Switch
            checked={settings.deleteRawAfterClassification}
            onChange={(v) => toggle("delete_raw_after_classification", v)}
          />
        </SettingRow>
        <SettingRow label="Max captured text length" hint="Upper bound on stored raw text (characters).">
          <span className="inline-edit">
            <input
              className="search"
              style={{ width: 120 }}
              type="number"
              min={500}
              max={100000}
              value={maxLen}
              onChange={(e) => setMaxLen(e.target.value)}
              onBlur={commitMaxLen}
              onKeyDown={(e) => e.key === "Enter" && commitMaxLen()}
            />
            <button className="btn" onClick={commitMaxLen}>Save</button>
          </span>
        </SettingRow>
      </div>

      {/* Domain rules */}
      <div className="card section-gap" hidden={section !== "content"}>
        <div className="card-pad" style={{ paddingBottom: 8 }}>
          <h2 className="card-title">Domain rules — allowlist &amp; blocklist</h2>
          <p className="card-hint">
            <b>Readable text</b> = allowlist (content can be captured). <b>Never capture</b> = blocklist.
            Banking, payment, email, password-manager &amp; gov/health domains are blocked automatically.
          </p>
        </div>

        <div className="rule-add card-pad">
          <input
            className="search"
            placeholder="example.com"
            value={newDomain}
            onChange={(e) => setNewDomain(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addRule()}
          />
          <select className="select" value={newCat} onChange={(e) => setNewCat(e.target.value as Category | "")}>
            <option value="">No category</option>
            {categories.map((c) => (
              <option key={c.id} value={c.id}>{c.label}</option>
            ))}
          </select>
          <select className="select" value={newMode} onChange={(e) => setNewMode(e.target.value as CaptureMode)}>
            {CAPTURE_MODES.map((m) => (
              <option key={m} value={m}>{CAPTURE_MODE_META[m].label}</option>
            ))}
          </select>
          <button className="btn btn-primary" onClick={addRule}>Add</button>
        </div>

        <table className="app-table">
          <thead>
            <tr>
              <th>Domain</th>
              <th style={{ width: 160 }}>Category</th>
              <th style={{ width: 170 }}>Capture</th>
              <th style={{ width: 40 }}></th>
            </tr>
          </thead>
          <tbody>
            {rules.length === 0 && (
              <tr><td colSpan={4} className="muted-num" style={{ padding: 16 }}>No domain rules yet.</td></tr>
            )}
            {rules.map((r) => (
              <tr key={r.domain}>
                <td>
                  <span className="dot" style={{ background: captureModeMeta(r.captureMode).color, marginRight: 8 }} />
                  {r.domain}
                </td>
                <td>
                  <select
                    className="select"
                    style={{ width: "100%" }}
                    value={r.category ?? ""}
                    onChange={(e) => saveRule(r.domain, (e.target.value || null) as Category | null, r.captureMode, r.aiReview)}
                  >
                    <option value="">No category</option>
                    {categories.map((c) => (
                      <option key={c.id} value={c.id}>{c.label}</option>
                    ))}
                  </select>
                </td>
                <td>
                  <select
                    className="select"
                    style={{ width: "100%" }}
                    value={r.captureMode}
                    onChange={(e) => saveRule(r.domain, r.category, e.target.value as CaptureMode, r.aiReview)}
                  >
                    {CAPTURE_MODES.map((m) => (
                      <option key={m} value={m}>{CAPTURE_MODE_META[m].label}</option>
                    ))}
                  </select>
                </td>
                <td>
                  <button className="icon-btn" onClick={() => removeRule(r.domain)} aria-label="Delete">✕</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* Extension endpoint */}
      <div className="card card-pad section-gap" hidden={section !== "connections"}>
        <h2 className="card-title">Browser extension</h2>
        <p className="card-hint">
          Paste these into the extension's options page (see <code>extension/README.md</code>). The
          endpoint is loopback-only and the token gates every request.
        </p>
        <SettingRow label="Local endpoint" hint="">
          <span className="inline-edit">
            <code className="token-box">{settings.endpoint}</code>
            <button className="btn" onClick={() => copy(settings.endpoint, "Endpoint")}>Copy</button>
          </span>
        </SettingRow>
        <SettingRow label="Ingest token" hint="">
          <span className="inline-edit">
            <code className="token-box">{settings.ingestToken}</code>
            <button className="btn" onClick={() => copy(settings.ingestToken, "Token")}>Copy</button>
          </span>
        </SettingRow>
      </div>

      {/* Idle detection */}
      <div className="card card-pad section-gap" hidden={section !== "tracking"}>
        <h2 className="card-title">Idle detection</h2>
        <p className="card-hint">
          After this many seconds with no keyboard or mouse input, the desktop counts as idle and
          that time is excluded from every stat — so leaving it on while you eat, hit the gym, or
          scroll your phone doesn't inflate your numbers. Lower = stricter.
        </p>
        <SettingRow label="Mark idle after" hint="20–1800 seconds of no input (default 60).">
          <span className="inline-edit">
            <input
              className="search"
              style={{ width: 90 }}
              type="number"
              min={20}
              max={1800}
              value={idleThreshold}
              onChange={(e) => setIdleThreshold(e.target.value)}
              onBlur={commitIdleThreshold}
              onKeyDown={(e) => e.key === "Enter" && commitIdleThreshold()}
            />
            <span className="muted-num">seconds</span>
            <button className="btn" onClick={commitIdleThreshold}>Save</button>
          </span>
        </SettingRow>
        <SettingRow
          label="Count media as active"
          hint="When a video, music, or a lecture is playing, keep that time active even with no input (Windows)."
        >
          <Switch
            checked={settings.countMediaAsActive}
            onChange={(v) => toggle("count_media_as_active", v)}
          />
        </SettingRow>
      </div>

      {/* Data retention */}
      <div className="card card-pad section-gap" hidden={section !== "data"}>
        <h2 className="card-title">Data retention</h2>
        <p className="card-hint">
          How long to keep raw activity samples on this device. Older rows are pruned automatically at
          startup. Set 0 to keep everything forever. Goals, scores, reviews and rules are always kept.
        </p>
        <SettingRow label="Keep raw activity for" hint="Days of detailed samples to retain (0 = forever).">
          <span className="inline-edit">
            <input
              className="search"
              style={{ width: 90 }}
              type="number"
              min={0}
              max={3650}
              value={retention}
              onChange={(e) => setRetention(e.target.value)}
              onBlur={commitRetention}
              onKeyDown={(e) => e.key === "Enter" && commitRetention()}
            />
            <span className="muted-num">days</span>
            <button className="btn" onClick={commitRetention}>Save</button>
          </span>
        </SettingRow>
        <SettingRow label="Prune now" hint="Apply the retention policy immediately.">
          <button className="btn" onClick={onPruneNow}>Prune old data</button>
        </SettingRow>
      </div>

      {/* Danger zone */}
      <div className="card card-pad section-gap danger-zone" hidden={section !== "data"}>
        <h2 className="card-title">Delete captured content</h2>
        <p className="card-hint">Activity rows (domain, URL, title, duration) are preserved.</p>
        <div className="danger-actions">
          <button className="btn" onClick={onPurge}>Delete raw page text only</button>
          <button className="btn btn-danger" onClick={onDeleteAll}>Delete all captured page content</button>
        </div>
      </div>

      <div className="card card-pad section-gap danger-zone" hidden={section !== "data"}>
        <h2 className="card-title">Reset app data</h2>
        <p className="card-hint">
          Wipe the local SQLite data so you can rebuild projects, goals and rules from scratch.
          Default privacy settings and editable streak definitions are recreated.
        </p>
        <div className="danger-actions">
          <button
            className="btn btn-danger"
            onClick={onResetDatabase}
            disabled={!isTauri() || resetting}
          >
            {resetting ? "Resetting…" : "Reset local database"}
          </button>
        </div>
      </div>
    </>
  );
}

function Head({ status }: { status?: string | null }) {
  return (
    <div className="page-head">
      <div>
        <h1 className="page-title">Settings</h1>
        <div className="page-subtitle">Local-only by default. Optional connections are always under your control.</div>
      </div>
      {status && <span className="live-dot"><span className="pulse" />{status}</span>}
    </div>
  );
}

function SettingRow({
  label,
  hint,
  children,
}: {
  label: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <div className="setting-row">
      <div>
        <div className="setting-label">{label}</div>
        {hint && <div className="setting-hint">{hint}</div>}
      </div>
      <div>{children}</div>
    </div>
  );
}

function Switch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      className={`switch${checked ? " on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="switch-thumb" />
    </button>
  );
}
