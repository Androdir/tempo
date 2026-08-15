import { useCallback, useEffect, useState } from "react";
import {
  createDatabaseBackup,
  generateAccountabilityExport,
  deleteAllCapturedContent,
  deleteDomainRule,
  getAccountabilitySettings,
  getCategoryDefinitions,
  getTrackingHealth,
  listDatabaseBackups,
  getDomainRules,
  getLlmSettings,
  getPrivacySettings,
  getLaunchAtLogin,
  isTauri,
  pruneOldData,
  purgeRawContent,
  resetDatabase,
  restoreDatabaseBackup,
  saveAccountabilityExport,
  setAccountabilitySetting,
  setDomainRule,
  setLlmSetting,
  setPrivacySetting,
  setTrackingPause,
  setLaunchAtLogin,
  setOpenAiApiKey,
  clearOpenAiApiKey,
  testOpenAiConnection,
  testOllamaConnection,
} from "../api";
import { previewToast } from "../components/AccountabilityLayer";
import SyncSettings from "../components/SyncSettings";
import { CAPTURE_MODE_META, captureModeMeta } from "../categories";
import type {
  AccountabilityExportOptions,
  AccountabilitySettings,
  CaptureMode,
  Category,
  CategoryDefinition,
  DatabaseBackup,
  DomainRule,
  LlmSettings,
  OllamaTestResult,
  PrivacySettings as Settings,
  TrackingHealth,
} from "../types";

const CAPTURE_MODES: CaptureMode[] = ["text", "meta", "never"];
function localIsoDate(date = new Date()): string {
  const shifted = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return shifted.toISOString().slice(0, 10);
}

function rangeStart(days: number): string {
  const date = new Date();
  date.setDate(date.getDate() - Math.max(0, days - 1));
  return localIsoDate(date);
}

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
  const [backups, setBackups] = useState<DatabaseBackup[]>([]);
  const [health, setHealth] = useState<TrackingHealth | null>(null);
  const [backupBusy, setBackupBusy] = useState(false);
  const [launchAtLogin, setLaunchAtLoginState] = useState<boolean | null>(null);
  const [section, setSection] = useState<SettingsSection>(initialSettingsSection);
  const [exportStart, setExportStart] = useState(() => rangeStart(7));
  const [exportEnd, setExportEnd] = useState(() => localIsoDate());
  const [exportNames, setExportNames] = useState(true);
  const [exportTitles, setExportTitles] = useState(false);
  const [exportNotes, setExportNotes] = useState(false);
  const [exportRaw, setExportRaw] = useState(false);
  const [exportBusy, setExportBusy] = useState<"copy" | "save" | null>(null);
  const [exportResult, setExportResult] = useState<string | null>(null);

  useEffect(() => {
    window.sessionStorage.removeItem("tempo_settings_section");
  }, []);

  const [maxLen, setMaxLen] = useState("8000");
  const [smartInterval, setSmartInterval] = useState("60");
  const [retention, setRetention] = useState("90");
  const [idleThreshold, setIdleThreshold] = useState("60");
  const [privateApps, setPrivateApps] = useState("");
  const [newDomain, setNewDomain] = useState("");
  const [newMode, setNewMode] = useState<CaptureMode>("meta");
  const [newCat, setNewCat] = useState<Category | "">("");

  const [llm, setLlm] = useState<LlmSettings | null>(null);
  const [llmUrl, setLlmUrl] = useState("http://localhost:11434");
  const [llmModel, setLlmModel] = useState("llama3.1:8b");
  const [openAiKey, setOpenAiKey] = useState("");
  const [openAiClassificationModel, setOpenAiClassificationModel] = useState("gpt-5.4-nano");
  const [openAiReviewModel, setOpenAiReviewModel] = useState("gpt-5.4-mini");
  const [testResult, setTestResult] = useState<OllamaTestResult | null>(null);
  const [testing, setTesting] = useState(false);

  const [acct, setAcct] = useState<AccountabilitySettings | null>(null);
  const [distractMin, setDistractMin] = useState("20");
  const [eodTime, setEodTime] = useState("21:00");

  const load = useCallback(async () => {
    try {
      const [s, r, l, a, c, startup, backupRows, healthStatus] = await Promise.all([
        getPrivacySettings(),
        getDomainRules(),
        getLlmSettings(),
        getAccountabilitySettings(),
        getCategoryDefinitions(),
        getLaunchAtLogin(),
        listDatabaseBackups(),
        getTrackingHealth(),
      ]);
      setSettings(s);
      setMaxLen(String(s.maxTextLength));
      setSmartInterval(String(s.smartIntervalSeconds));
      setRetention(String(s.retentionDays));
      setIdleThreshold(String(s.idleThresholdSeconds));
      setPrivateApps(s.titleExcludedApps.join(", "));
      setRules(r);
      setCategories(c);
      setLaunchAtLoginState(startup);
      setBackups(backupRows);
      setHealth(healthStatus);
      setLlm(l);
      setLlmUrl(l.url);
      setLlmModel(l.model);
      setOpenAiClassificationModel(l.openaiClassificationModel);
      setOpenAiReviewModel(l.openaiReviewModel);
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

  async function pauseTracking(minutes: number) {
    try {
      await setTrackingPause(minutes);
      await load();
      flash(minutes > 0 ? "Tracking paused" : "Tracking resumed");
    } catch (e) {
      setError(String(e));
    }
  }
  async function savePrivateApps() {
    const apps = privateApps
      .split(/[,\n]+/)
      .map((app) => app.trim())
      .filter(Boolean);
    try {
      await setPrivacySetting("title_excluded_apps", JSON.stringify(apps));
      await load();
      flash(apps.length ? "Sensitive-app privacy saved" : "Sensitive-app exclusions cleared");
    } catch (e) {
      setError(String(e));
    }
  }
  async function toggleLaunchAtLogin(value: boolean) {
    try {
      await setLaunchAtLogin(value);
      setLaunchAtLoginState(await getLaunchAtLogin());
      flash(value ? "Tempo will launch at login" : "Launch at login disabled");
    } catch (e) {
      setError(String(e));
    }
  }

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

  async function setProvider(provider: "ollama" | "openai") {
    try {
      await setLlmSetting("llm_provider", provider);
      setTestResult(null);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function saveOpenAiModels() {
    try {
      await setLlmSetting("openai_classification_model", openAiClassificationModel.trim());
      await setLlmSetting("openai_review_model", openAiReviewModel.trim());
      await load();
      flash("OpenAI models saved");
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleOpenAiContent(value: boolean) {
    try {
      await setLlmSetting("openai_include_content", value ? "1" : "0");
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function saveOpenAiKey() {
    if (!openAiKey.trim()) return;
    try {
      await setOpenAiApiKey(openAiKey.trim());
      setOpenAiKey("");
      await load();
      flash("API key saved securely");
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeOpenAiKey() {
    if (!confirm("Remove Tempo's OpenAI API key from Windows Credential Manager?")) return;
    try {
      await clearOpenAiApiKey();
      setOpenAiKey("");
      setTestResult(null);
      await load();
      flash("API key removed");
    } catch (e) {
      setError(String(e));
    }
  }

  async function testLlm() {
    setTesting(true);
    setTestResult(null);
    try {
      setTestResult(
        llm?.provider === "openai"
          ? await testOpenAiConnection(openAiClassificationModel.trim())
          : await testOllamaConnection(llmUrl.trim(), llmModel.trim())
      );
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

  async function copyDiagnostics() {
    const diagnostics = {
      app: "Tempo",
      version: "0.1.0",
      generatedAt: new Date().toISOString(),
      desktopApp: isTauri(),
      platform: navigator.platform,
      database: health ? {
        status: health.status,
        ok: health.databaseOk,
        browserConnected: health.browserConnected,
        smartEnabled: health.smartEnabled,
        pendingSyncEvents: health.pendingSyncEvents,
        lastDesktopAt: health.lastDesktopAt,
        lastBrowserAt: health.lastBrowserAt,
        lastScreenAt: health.lastScreenAt,
        lastBackupAt: health.lastBackupAt,
        issues: health.issues,
      } : null,
      privacy: settings ? {
        trackingPaused: settings.trackingPausedUntil != null,
        smartTrackingEnabled: settings.smartTrackingEnabled,
        capturePageContent: settings.capturePageContent,
        storeRawText: settings.storeRawText,
        retentionDays: settings.retentionDays,
        sensitiveAppCount: settings.titleExcludedApps.length,
      } : null,
    };
    try {
      await navigator.clipboard.writeText(JSON.stringify(diagnostics, null, 2));
      flash("Privacy-filtered diagnostics copied");
    } catch (e) {
      setError(`Could not copy diagnostics: ${String(e)}`);
    }
  }
  async function makeBackup() {
    setBackupBusy(true);
    try {
      const backup = await createDatabaseBackup();
      await load();
      flash(`Backup created: ${backup.name}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBackupBusy(false);
    }
  }

  async function restoreBackup(backup: DatabaseBackup) {
    if (!confirm(`Restore “${backup.name}”? Tempo will first preserve the current database as a new manual backup, then replace current data.`)) return;
    setBackupBusy(true);
    try {
      await restoreDatabaseBackup(backup.name);
      await load();
      flash("Backup restored. Reloading current data…");
      window.setTimeout(() => window.location.reload(), 700);
    } catch (e) {
      setError(String(e));
      setBackupBusy(false);
    }
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

  function exportOptions(): AccountabilityExportOptions {
    return { startDate: exportStart, endDate: exportEnd, includeActivityNames: exportNames,
      includeTitles: exportTitles, includeNotes: exportNotes, includeRawText: exportRaw };
  }

  function setExportPreset(days: number) {
    setExportStart(rangeStart(days));
    setExportEnd(localIsoDate());
    setExportResult(null);
  }

  async function copyAccountabilityReport() {
    setExportBusy("copy"); setExportResult(null);
    try {
      const report = await generateAccountabilityExport(exportOptions());
      await navigator.clipboard.writeText(report.markdown);
      setExportResult(`Copied ${report.dayCount}-day report. Paste it into ChatGPT.`);
      flash("Accountability report copied");
    } catch (e) { setError(String(e)); } finally { setExportBusy(null); }
  }

  async function saveAccountabilityReport() {
    setExportBusy("save"); setExportResult(null);
    try {
      const options = exportOptions();
      const nativePath = await saveAccountabilityExport(options);
      if (nativePath) {
        setExportResult(`Saved and verified: ${nativePath}`);
        flash("Accountability report saved");
      } else {
        const report = await generateAccountabilityExport(options);
        const url = URL.createObjectURL(new Blob([report.markdown], { type: "text/markdown;charset=utf-8" }));
        const link = document.createElement("a");
        link.href = url; link.download = report.filename; document.body.appendChild(link); link.click(); link.remove();
        window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
        setExportResult(`Downloaded ${report.filename}`); flash("Accountability report downloaded");
      }
    } catch (e) { setError(String(e)); } finally { setExportBusy(null); }
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

      {launchAtLogin !== null && (
        <div className="card card-pad" hidden={section !== "tracking"}>
          <h2 className="card-title">Startup & background</h2>
          <p className="card-hint">
            Keep tracking automatic without putting a window in your way. Closing Tempo still sends
            it to the system tray; use the tray menu when you want to quit completely.
          </p>
          <SettingRow
            label="Launch at Windows login"
            hint="Starts hidden in the system tray so tracking begins automatically after you sign in."
          >
            <Switch checked={launchAtLogin} onChange={toggleLaunchAtLogin} />
          </SettingRow>
        </div>
      )}

      <div className="card card-pad section-gap" hidden={section !== "tracking"}>
        <h2 className="card-title">Pause &amp; private time</h2>
        <p className="card-hint">
          Pausing stops desktop, browser-extension and screen activity rows. Tempo automatically resumes at the selected time.
        </p>
        <SettingRow
          label={settings.trackingPausedUntil ? "Tracking is paused" : "Tracking is active"}
          hint={settings.trackingPausedUntil ? `Resumes ${new Date(settings.trackingPausedUntil).toLocaleString()}` : "You can also pause or resume from the Tempo tray icon."}
        >
          <span className="pause-actions">
            {settings.trackingPausedUntil ? (
              <button className="btn btn-primary" onClick={() => pauseTracking(0)}>Resume now</button>
            ) : (
              <>
                <button className="btn" onClick={() => pauseTracking(15)}>15 min</button>
                <button className="btn" onClick={() => pauseTracking(60)}>1 hour</button>
                <button className="btn" onClick={() => pauseTracking(8 * 60)}>8 hours</button>
              </>
            )}
          </span>
        </SettingRow>
        <SettingRow
          label="Sensitive apps"
          hint="Tempo still records the app and duration, but stores no window title and skips screen OCR while one of these apps is active."
        >
          <span className="inline-edit private-apps-control">
            <input
              className="search"
              placeholder="1Password, Bitwarden, Signal"
              value={privateApps}
              onChange={(e) => setPrivateApps(e.target.value)}
              onBlur={savePrivateApps}
              onKeyDown={(e) => e.key === "Enter" && savePrivateApps()}
            />
            <button className="btn" onClick={savePrivateApps}>Save</button>
          </span>
        </SettingRow>
      </div>

      {/* Accountability */}
      {acct && (
        <div className="card card-pad" hidden={section !== "tracking"}>
          <h2 className="card-title">Accountability</h2>
          <p className="card-hint">
            Local nudges only — distraction warnings and the end-of-day review appear as Windows
            notifications even while Tempo is in the tray. The in-app banner keeps quick actions
            available when Tempo is open. Nothing is uploaded.
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
          <SettingRow label="Preview" hint="Send a Windows notification and show its in-app actions.">
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

      {/* Optional AI classification */}
      {llm && (
        <div className="card card-pad section-gap" hidden={section !== "connections"}>
          <h2 className="card-title">AI classification</h2>
          <p className="card-hint">
            Optional. Rules and your manual corrections always take priority. AI only reviews uncertain
            activity, and any error falls back to rule-based classification.
          </p>
          <SettingRow label="Provider" hint="OpenAI is faster and avoids using your gaming GPU. Ollama stays entirely local.">
            <select
              className="select"
              value={llm.provider}
              onChange={(event) => setProvider(event.target.value as "ollama" | "openai")}
            >
              <option value="openai">OpenAI API</option>
              <option value="ollama">Local Ollama</option>
            </select>
          </SettingRow>
          <SettingRow
            label="Enable AI"
            hint={llm.provider === "openai" ? "Uses your OpenAI API credit." : "Requires Ollama running locally."}
          >
            <Switch checked={llm.enabled} onChange={toggleLlm} />
          </SettingRow>

          {llm.provider === "openai" ? (
            <>
              <div className="llm-cloud-notice">
                <b>What OpenAI receives:</b> app/site name, window or page title, duration, rule result,
                project-match evidence and nearby activity labels. Raw text is never sent. Locally derived
                page/OCR summaries are excluded unless you enable the option below.
              </div>
              <SettingRow
                label="OpenAI API key"
                hint={llm.openaiKeyConfigured ? "Saved in Windows Credential Manager." : "Paste the key you just created. Tempo never stores it in SQLite."}
              >
                <span className="inline-edit llm-key-edit">
                  <input
                    className="search"
                    type="password"
                    autoComplete="new-password"
                    placeholder={llm.openaiKeyConfigured ? "Key configured" : "sk-…"}
                    value={openAiKey}
                    onChange={(event) => setOpenAiKey(event.target.value)}
                  />
                  <button className="btn btn-primary" onClick={saveOpenAiKey} disabled={!openAiKey.trim()}>
                    {llm.openaiKeyConfigured ? "Replace" : "Save key"}
                  </button>
                  {llm.openaiKeyConfigured && <button className="btn" onClick={removeOpenAiKey}>Remove</button>}
                </span>
              </SettingRow>
              <SettingRow label="Classification model" hint="Low-cost model used only for uncertain activity.">
                <input
                  className="search llm-model-input"
                  value={openAiClassificationModel}
                  onChange={(event) => setOpenAiClassificationModel(event.target.value)}
                />
              </SettingRow>
              <SettingRow label="Review model" hint="Used for Daily Review and tomorrow's lock-in plan.">
                <span className="inline-edit">
                  <input
                    className="search llm-model-input"
                    value={openAiReviewModel}
                    onChange={(event) => setOpenAiReviewModel(event.target.value)}
                  />
                  <button className="btn" onClick={saveOpenAiModels}>Save models</button>
                </span>
              </SettingRow>
              <SettingRow
                label="Include locally derived content summaries"
                hint="Off by default. Enable only if titles alone are not enough; raw captured text is still never sent."
              >
                <Switch checked={llm.openaiIncludeContent} onChange={toggleOpenAiContent} />
              </SettingRow>
            </>
          ) : (
            <>
              <p className="card-hint">
                Only loopback, LAN and Tailscale addresses are allowed. Tempo pauses local AI during
                detected gameplay and releases the model after each batch.
              </p>
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
                    className="search llm-model-input"
                    value={llmModel}
                    onChange={(e) => setLlmModel(e.target.value)}
                    onBlur={saveLlm}
                  />
                  <button className="btn" onClick={saveLlm}>Save</button>
                </span>
              </SettingRow>
            </>
          )}
          <SettingRow label="Connection" hint={`Check Tempo can reach ${llm.provider === "openai" ? "OpenAI" : "Ollama"}.`}>
            <button className="btn" onClick={testLlm} disabled={testing || (llm.provider === "openai" && !llm.openaiKeyConfigured)}>
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
        <p className="card-hint">
          Normal tracking only needs the page title, summary and keywords. Leave raw text off unless you are debugging why a page was classified a certain way.
        </p>
        <SettingRow
          label="Keep page-text excerpt (debugging only)"
          hint={
            settings.deleteRawAfterClassification
              ? "Raw text is currently discarded after classification, so no excerpt will remain."
              : "Shows the exact captured visible-text excerpt in Activity details so you can audit a bad classification."
          }
        >
          <Switch checked={settings.storeRawText} onChange={(v) => toggle("store_raw_text", v)} />
        </SettingRow>
        <SettingRow
          label="Delete raw text after classification"
          hint="Recommended. Use page text temporarily to derive a summary and keywords, then discard the original excerpt."
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

      <div className="card card-pad section-gap accountability-export" hidden={section !== "data"}>
        <div className="settings-card-head">
          <div>
            <h2 className="card-title">Accountability export</h2>
            <p className="card-hint">Create a compact, ChatGPT-ready report that exposes time leaks, fragmented work, missed goals and output patterns. Nothing is uploaded by Tempo.</p>
          </div>
          <span className="health-pill">Local report</span>
        </div>
        <div className="export-presets" aria-label="Quick date ranges">
          {[7, 14, 30, 90].map((days) => <button className="btn" key={days} onClick={() => setExportPreset(days)}>Last {days} days</button>)}
        </div>
        <div className="export-date-grid">
          <label><span>From</span><input className="search" type="date" value={exportStart} max={exportEnd} onChange={(e) => { setExportStart(e.target.value); setExportResult(null); }} /></label>
          <label><span>To</span><input className="search" type="date" value={exportEnd} min={exportStart} max={localIsoDate()} onChange={(e) => { setExportEnd(e.target.value); setExportResult(null); }} /></label>
        </div>
        <div className="export-privacy-grid">
          <label><input type="checkbox" checked={exportNames} onChange={(e) => setExportNames(e.target.checked)} /> Include app and website names</label>
          <label><input type="checkbox" checked={exportTitles} onChange={(e) => setExportTitles(e.target.checked)} /> Include window/page title samples</label>
          <label><input type="checkbox" checked={exportNotes} onChange={(e) => setExportNotes(e.target.checked)} /> Include private daily notes</label>
          <label className={exportRaw ? "sensitive-option selected" : "sensitive-option"}><input type="checkbox" checked={exportRaw} onChange={(e) => setExportRaw(e.target.checked)} /> Include raw captured text samples</label>
        </div>
        <p className="export-privacy-note">Titles, notes and raw text are excluded by default because they can contain private chats or client information. URLs, file paths, tokens and secrets are never included. Review the file before sharing it.</p>
        {exportRaw && <div className="health-issue">Raw text is high-sensitivity. Only enable it when you genuinely need classification context.</div>}
        <div className="export-actions">
          <button className="btn btn-primary" onClick={saveAccountabilityReport} disabled={exportBusy !== null || !exportStart || !exportEnd}>{exportBusy === "save" ? "Building report…" : isTauri() ? "Save .md to Downloads" : "Download .md report"}</button>
          <button className="btn" onClick={copyAccountabilityReport} disabled={exportBusy !== null || !exportStart || !exportEnd}>{exportBusy === "copy" ? "Copying…" : "Copy for ChatGPT"}</button>
        </div>
        {exportResult && <div className="export-result" role="status">{exportResult}</div>}
        <p className="card-hint export-hint">The report includes a suggested “brutally honest accountability coach” prompt, but tells the AI not to invent conclusions from missing or uncertain tracking.</p>
      </div>
      <div className="card card-pad section-gap" hidden={section !== "data"}>
        <div className="settings-card-head">
          <div>
            <h2 className="card-title">Database safety</h2>
            <p className="card-hint">Tempo verifies SQLite integrity and keeps up to seven automatic daily backups before startup migrations.</p>
          </div>
          <span className={`health-pill ${health?.status ?? "warning"}`}>
            {!isTauri() ? "Desktop app only" : health?.databaseOk ? "Database healthy" : "Check required"}
          </span>
        </div>
        {health?.issues.map((issue) => <div className="health-issue" key={issue}>{issue}</div>)}
        <div className="backup-actions">
          <button className="btn btn-primary" onClick={makeBackup} disabled={!isTauri() || backupBusy}>
            {backupBusy ? "Working…" : "Back up now"}
          </button>
          <button className="btn" onClick={copyDiagnostics}>Copy diagnostics</button>
          {health?.lastBackupAt && <span className="muted-num">Latest: {new Date(health.lastBackupAt).toLocaleString()}</span>}
          {!isTauri() && <span className="muted-num">Open the desktop app to create or restore a manual backup. Automatic startup backups also run on the Hub.</span>}
        </div>
        {backups.length > 0 && (
          <div className="backup-list">
            {backups.map((backup) => (
              <div className="backup-row" key={backup.name}>
                <div>
                  <strong>{backup.automatic ? "Automatic" : "Manual"}</strong>
                  <span>{new Date(backup.createdAt).toLocaleString()} · {(backup.bytes / 1024 / 1024).toFixed(1)} MB</span>
                </div>
                <button className="btn" onClick={() => restoreBackup(backup)} disabled={backupBusy}>Restore</button>
              </div>
            ))}
          </div>
        )}
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
