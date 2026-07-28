import { useCallback, useEffect, useState } from "react";
import {
  getBrowserActivity,
  getGoals,
  getPrivacySettings,
  getTodaySummary,
  getTrackedApps,
  getWatchedFolders,
  isRemote,
  isTauri,
} from "../api";
import type { Page } from "../components/Sidebar";
import type { PrivacySettings } from "../types";

type SettingsSection = "tracking" | "content" | "connections" | "data";

type SetupSnapshot = {
  goals: number;
  appsSeen: number;
  appsCategorized: number;
  browserSamples: number;
  watchedFolders: number;
  settings: PrivacySettings | null;
};

const EMPTY_SNAPSHOT: SetupSnapshot = {
  goals: 0,
  appsSeen: 0,
  appsCategorized: 0,
  browserSamples: 0,
  watchedFolders: 0,
  settings: null,
};

export default function SetupGuide({ onNavigate }: { onNavigate: (page: Page) => void }) {
  const desktopApp = isTauri();
  const hubDashboard = isRemote();
  const previewMode = !desktopApp && !hubDashboard;
  const [snapshot, setSnapshot] = useState<SetupSnapshot>(EMPTY_SNAPSHOT);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    const [goals, apps, summary, browser, folders, settings] = await Promise.all([
      getGoals().catch(() => []),
      getTrackedApps().catch(() => []),
      getTodaySummary().catch(() => null),
      getBrowserActivity().catch(() => null),
      getWatchedFolders().catch(() => []),
      getPrivacySettings().catch(() => null),
    ]);
    const observedApps = hubDashboard && summary ? summary.perApp : apps;

    setSnapshot({
      goals: goals.length,
      appsSeen: observedApps.length,
      appsCategorized: observedApps.filter((app) => app.category !== null).length,
      browserSamples: hubDashboard && summary
        ? summary.perWebsite.length
        : browser
          ? browser.perDomain.length + browser.recentPages.length
          : 0,
      watchedFolders: folders.filter((folder) => folder.enabled).length,
      settings,
    });
    setLoading(false);
  }, [hubDashboard]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  function openSettings(section: SettingsSection) {
    window.sessionStorage.setItem("tempo_settings_section", section);
    onNavigate("privacy");
  }

  async function copy(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(label);
      window.setTimeout(() => setCopied(null), 1800);
    } catch {
      setCopied("Copy failed");
    }
  }

  const essentials = [
    {
      title: desktopApp
        ? "Desktop tracking is running"
        : hubDashboard
          ? "Hub is ready for a tracker"
          : "Run the desktop tracker",
      detail: desktopApp
        ? "Tempo records the active app and window title every 10 seconds."
        : hubDashboard
          ? "Connect a Tempo desktop or Android tracker in Settings."
          : "This browser preview does not record activity. Start the real app with npm run app.",
      done: desktopApp || (hubDashboard && snapshot.appsSeen > 0),
      action: hubDashboard ? "Connect a device" : undefined,
      onAction: hubDashboard ? () => openSettings("connections") : undefined,
    },
    {
      title: "Choose today's main mission",
      detail: snapshot.goals > 0
        ? `${snapshot.goals} goal${snapshot.goals === 1 ? "" : "s"} ready for today.`
        : "Add one clear outcome so the daily review has useful context.",
      done: snapshot.goals > 0,
      action: "Set a goal",
      onAction: () => onNavigate("goals"),
    },
    {
      title: "Let Tempo observe one minute of work",
      detail: snapshot.appsSeen > 0
        ? `${snapshot.appsSeen} app${snapshot.appsSeen === 1 ? "" : "s"} detected.`
        : "Use your computer normally, then refresh this check.",
      done: snapshot.appsSeen > 0,
      action: "View activity",
      onAction: () => onNavigate("timeline"),
    },
    {
      title: "Classify the apps that matter",
      detail: snapshot.appsCategorized > 0
        ? `${snapshot.appsCategorized} app${snapshot.appsCategorized === 1 ? "" : "s"} classified. Tempo will reuse these rules.`
        : "Mark your main apps as productive, neutral, distracting, or excluded.",
      done: snapshot.appsCategorized > 0,
      action: "Classify apps",
      onAction: () => onNavigate("categories"),
    },
  ];

  const completeCount = essentials.filter((step) => step.done).length;
  const nextStep = essentials.find((step) => !step.done && step.onAction);
  const progress = Math.round((completeCount / essentials.length) * 100);
  const browserConnected = snapshot.browserSamples > 0;

  return (
    <div className="setup-guide">
      <section className="guide-hero">
        <div className="guide-hero-copy">
          <div className="eyebrow">SETUP &amp; GUIDE</div>
          <h1>Get useful data in a few minutes</h1>
          <p>
            Start with the four essentials. Browser history, output folders, screen context,
            and local AI are optional upgrades you can add later.
          </p>
          <div className="guide-hero-actions">
            {nextStep ? (
              <button className="btn btn-primary" onClick={nextStep.onAction}>
                Next: {nextStep.title}
              </button>
            ) : (
              <button className="btn btn-primary" onClick={() => onNavigate("dashboard")}>
                Go to Today
              </button>
            )}
            <button className="btn" onClick={refresh} disabled={loading}>
              {loading ? "Checking…" : "Refresh checks"}
            </button>
          </div>
        </div>
        <div className="guide-progress-card" aria-label={`${completeCount} of 4 essentials complete`}>
          <div className="guide-progress-number">{completeCount}/4</div>
          <div className="guide-progress-label">essentials ready</div>
          <div className="guide-progress-track" aria-hidden="true">
            <span style={{ width: `${progress}%` }} />
          </div>
          <span className={`guide-mode ${desktopApp ? "ready" : ""}`}>
            <span className="pulse" />
            {desktopApp ? "Desktop app" : hubDashboard ? "Hub dashboard" : "Preview mode"}
          </span>
        </div>
      </section>

      {previewMode && (
        <div className="guide-callout">
          <b>You are viewing the browser preview.</b>
          <span>
            It uses sample data and cannot track your computer. Run <code>npm run app</code> for
            real tracking.
          </span>
        </div>
      )}

      <section className="guide-section">
        <div className="guide-section-head">
          <div>
            <div className="eyebrow">START HERE</div>
            <h2>Essential setup</h2>
          </div>
          <span className="guide-muted">Checks update automatically when this page opens.</span>
        </div>
        <div className="guide-checklist">
          {essentials.map((step, index) => (
            <div className={`guide-check${step.done ? " done" : ""}`} key={step.title}>
              <span className="guide-check-icon">{step.done ? "✓" : index + 1}</span>
              <div>
                <b>{step.title}</b>
                <span>{step.detail}</span>
              </div>
              {step.onAction && step.action && (
                <button className="btn" onClick={step.onAction}>{step.action}</button>
              )}
            </div>
          ))}
        </div>
      </section>

      <details className="guide-details">
        <summary>
          <span>
            <b>Add browser tracking</b>
            <small>Optional · Firefox, Chrome and Edge</small>
          </span>
          <span className={`guide-status-pill${browserConnected ? " ready" : ""}`}>
            {browserConnected ? "Receiving activity" : "Not connected"}
          </span>
        </summary>
        <div className="guide-details-body">
          <p>
            The extension adds domain and page-title context. Tempo still works without it, so
            skip this until the basic app tracking feels useful.
          </p>
          <ol className="guide-numbered">
            <li>
              <b>Chrome or Edge:</b> open <code>chrome://extensions</code> or
              <code> edge://extensions</code>, enable Developer mode, choose <b>Load unpacked</b>,
              and select this project&apos;s <code>extension</code> folder.
            </li>
            <li>
              <b>Firefox 142+:</b> open <code>about:debugging#/runtime/this-firefox</code>, choose
              <b> Load Temporary Add-on</b>, and select <code>extension/manifest.json</code>.
            </li>
            <li>Open Tempo&apos;s extension Options page and paste the endpoint and token below.</li>
            <li>Choose <b>Save</b>, then <b>Test connection</b>.</li>
          </ol>
          {snapshot.settings ? (
            <div className="guide-credentials">
              <div>
                <span>Endpoint</span>
                <code>{snapshot.settings.endpoint}</code>
                <button className="btn" onClick={() => copy(snapshot.settings!.endpoint, "Endpoint")}>
                  {copied === "Endpoint" ? "Copied" : "Copy"}
                </button>
              </div>
              <div>
                <span>Token</span>
                <code>{snapshot.settings.ingestToken}</code>
                <button className="btn" onClick={() => copy(snapshot.settings!.ingestToken, "Token")}>
                  {copied === "Token" ? "Copied" : "Copy"}
                </button>
              </div>
            </div>
          ) : (
            <div className="guide-inline-note">Open the desktop app to generate extension credentials.</div>
          )}
          <div className="guide-actions">
            <button className="btn" onClick={() => openSettings("connections")}>Connection settings</button>
            <button className="btn" onClick={() => onNavigate("browser")}>View browser activity</button>
          </div>
        </div>
      </details>

      <section className="guide-section">
        <div className="guide-section-head">
          <div>
            <div className="eyebrow">OPTIONAL</div>
            <h2>Add only what helps</h2>
          </div>
        </div>
        <div className="guide-option-grid">
          <article className="guide-option">
            <div className="guide-option-top">
              <span className="guide-option-icon">◫</span>
              <span className={`guide-status-pill${snapshot.watchedFolders > 0 ? " ready" : ""}`}>
                {snapshot.watchedFolders > 0 ? `${snapshot.watchedFolders} active` : "Off"}
              </span>
            </div>
            <h3>Output folders</h3>
            <p>Count saved files as evidence of completed work without reading their contents.</p>
            <button className="btn" onClick={() => onNavigate("outputs")}>Choose folders</button>
          </article>
          <article className="guide-option">
            <div className="guide-option-top">
              <span className="guide-option-icon">⌗</span>
              <span className={`guide-status-pill${snapshot.settings?.smartTrackingEnabled ? " ready" : ""}`}>
                {snapshot.settings?.smartTrackingEnabled ? "On" : "Off"}
              </span>
            </div>
            <h3>Screen context</h3>
            <p>Use local OCR for better classification when app titles are not enough.</p>
            <button className="btn" onClick={() => openSettings("tracking")}>Review tracking</button>
          </article>
          <article className="guide-option">
            <div className="guide-option-top">
              <span className="guide-option-icon">✦</span>
              <span className="guide-status-pill">Optional</span>
            </div>
            <h3>Local AI review</h3>
            <p>Connect Ollama for richer summaries. Core tracking and scoring do not require AI.</p>
            <button className="btn" onClick={() => openSettings("connections")}>AI settings</button>
          </article>
        </div>
      </section>

      <section className="guide-section">
        <div className="guide-section-head">
          <div>
            <div className="eyebrow">EVERYDAY USE</div>
            <h2>A lightweight routine</h2>
          </div>
        </div>
        <div className="guide-routine">
          <article>
            <span>1</span>
            <div><b>Plan</b><small>Set one to three goals for today.</small></div>
            <button className="link-btn" onClick={() => onNavigate("goals")}>Open goals</button>
          </article>
          <article>
            <span>2</span>
            <div><b>Work</b><small>Leave Tempo running. Use Focus Mode only when you need it.</small></div>
            <button className="link-btn" onClick={() => onNavigate("focus")}>Start focus</button>
          </article>
          <article>
            <span>3</span>
            <div><b>Correct</b><small>Fix misclassified apps once; future activity reuses the rule.</small></div>
            <button className="link-btn" onClick={() => onNavigate("activity")}>Activity log</button>
          </article>
          <article>
            <span>4</span>
            <div><b>Review</b><small>Check what helped, what distracted you, and tomorrow&apos;s adjustment.</small></div>
            <button className="link-btn" onClick={() => onNavigate("review")}>Daily review</button>
          </article>
        </div>
      </section>

      <section className="guide-expectations">
        <div>
          <h3>What Tempo tracks automatically</h3>
          <p>Foreground app time, window titles, idle time, and any optional sources you enable.</p>
        </div>
        <div>
          <h3>What still needs your input</h3>
          <p>Offline work, your intentions, and whether an activity was genuinely useful.</p>
        </div>
        <button className="btn" onClick={() => openSettings("data")}>Privacy &amp; data controls</button>
      </section>
    </div>
  );
}
