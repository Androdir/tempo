import { useCallback, useEffect, useMemo, useState } from "react";
import {
  correctActivity,
  excludeActivityFromProject,
  getActivityDetails,
  getCategoryDefinitions,
  getCorrectionHistory,
  getClassificationPolicies,
  setClassificationPolicies,
  undoCorrection,
} from "../api";
import { categoryMeta, contentTypeMeta } from "../categories";
import { formatDuration } from "../format";
import type { ActivityDetail, ActivityLogEntry, CategoryDefinition, CorrectionHistoryEntry } from "../types";
import { CategoryBadge, ProjectTag } from "./ui";

interface ActivityDetailsDrawerProps {
  id?: number | null;
  activity?: ActivityLogEntry | null;
  onClose: () => void;
  onCorrected?: () => void;
}

function sourceName(source: ActivityLogEntry["source"]): string {
  if (source === "web") return "Website";
  if (source === "screen") return "Screen OCR";
  return "Desktop app";
}


function explainCategory(
  reason: string,
  source: ActivityLogEntry["source"],
  category: string,
  classifier: ActivityLogEntry["classifier"],
): string {
  const label = categoryMeta(category).label;
  const lower = reason.toLocaleLowerCase();

  if (classifier === "manual" || lower.includes("manual correction")) {
    return `You previously corrected this activity to ${label}. Your correction takes priority over automatic matching.`;
  }
  if (lower.startsWith("policy:")) {
    return `Tempo identified the activity type first, then your visible policy mapped that type to ${label}.`;
  }
  if (lower.startsWith("project:")) {
    return `A confident project match supplied the ${label} category. See the separate project evidence below.`;
  }
  if (lower.startsWith("app rule")) {
    return `A saved rule for this desktop app classifies it as ${label}.`;
  }
  if (lower.startsWith("domain rule")) {
    return `A saved rule for this website classifies it as ${label}.`;
  }
  if (lower.includes("telegram default")) {
    return `Tempo's built-in Telegram default classifies it as ${label} unless you add an app rule or a strong project match.`;
  }
  if (lower.includes("tempo app") || lower.includes("tempo settings")) {
    return `Tempo treats time spent administering Tempo as ${label}, rather than counting it as goal progress.`;
  }
  if (lower.includes("windows system surface")) {
    return `Tempo recognises this as a temporary Windows surface and classifies it as ${label}.`;
  }
  if (lower.startsWith("content signals")) {
    return `Several title or captured-content signals pointed to ${label}.`;
  }
  if (lower.startsWith("content hint")) {
    return `One title or captured-content signal suggested ${label}; treat this as a lower-confidence guess.`;
  }
  if (lower.startsWith("default for")) {
    return `No stronger saved rule or project match applied, so Tempo used the default for this kind of website content: ${label}.`;
  }
  if (lower === "no app rule") {
    return `There is no saved rule for this app and no project reached the assignment threshold, so Tempo left it ${label}.`;
  }
  if (classifier === "llm") {
    return `The local AI reviewed the ${source === "screen" ? "visible screen text" : "activity context"} and chose ${label}.`;
  }
  return `Tempo's ${sourceName(source).toLocaleLowerCase()} classifier chose ${label}.`;
}

function explainProjectSignal(signal: string): string {
  const [kind, ...rest] = signal.split(":");
  const value = rest.join(":").trim();
  if (!value) return signal;
  if (kind === "app/domain") return `App or domain matched: ${value}`;
  if (kind === "title keyword") return `Window or page title contained: “${value}”`;
  if (kind === "content keyword") return `Captured content contained: “${value}”`;
  if (kind === "keyword") return `Title or captured content contained: “${value}”`;
  return signal;
}

function ClassificationSource({ classifier }: { classifier: ActivityLogEntry["classifier"] }) {
  if (classifier === "llm") return <span className="src-chip llm">🤖 Local AI</span>;
  if (classifier === "manual") return <span className="src-chip manual">✎ Manual correction</span>;
  return <span className="src-chip rule">Rule</span>;
}

export default function ActivityDetailsDrawer({
  id = null,
  activity = null,
  onClose,
  onCorrected,
}: ActivityDetailsDrawerProps) {
  const [detail, setDetail] = useState<ActivityDetail | null>(null);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [projectAction, setProjectAction] = useState<string | null>(null);
  const [history, setHistory] = useState<CorrectionHistoryEntry[]>([]);
  const [typeAction, setTypeAction] = useState<string | null>(null);

  const open = id != null || activity != null;

  const loadDetail = useCallback(() => {
    if (id == null) {
      setDetail(null);
      return;
    }
    getActivityDetails(id)
      .then(setDetail)
      .catch((e) => setError(String(e)));
  }, [id]);

  useEffect(() => {
    if (!open) return;
    setDetail(null);
    setError(null);
    loadDetail();
    getCategoryDefinitions().then(setCategories).catch(() => {});
  }, [open, activity?.blockKey, loadDetail]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    const key = activity?.blockKey ?? detail?.blockKey;
    if (!open || !key) {
      setHistory([]);
      return;
    }
    getCorrectionHistory(key).then(setHistory).catch(() => setHistory([]));
  }, [open, activity?.blockKey, detail?.blockKey]);
  const view = useMemo(() => {
    if (!activity && !detail) return null;
    const source = activity?.source ?? "web";
    const category = detail?.category ?? activity?.category ?? "uncategorized";
    const classifier = detail?.classifier ?? activity?.classifier ?? "rule";
    const reason = detail?.classificationReason ?? activity?.reason ?? "No explanation recorded";
    const activityKind = activity?.activityKind ?? "unknown";
    const confidence = classifier === "llm"
      ? (detail?.llmConfidence ?? activity?.llmConfidence ?? null)
      : (detail?.confidence ?? activity?.confidence ?? null);
    const projectName = detail ? detail.projectName : (activity?.projectName ?? null);
    const projectConfidence = detail ? detail.projectConfidence : (activity?.projectConfidence ?? 0);
    const projectSignals = detail ? detail.projectSignals : (activity?.projectSignals ?? []);

    return {
      source,
      label: activity?.label ?? detail?.domain ?? "",
      title: activity?.title ?? detail?.pageTitle ?? "",
      seconds: activity?.seconds ?? detail?.durationSeconds ?? 0,
      lastSeen: activity?.lastSeen ?? detail?.timestamp ?? "",
      category,
      classifier,
      reason,
      activityKind,
      confidence,
      projectName,
      projectConfidence,
      projectSignals,
      contentType: detail?.contentType ?? activity?.contentType ?? null,
      summary: detail?.contentSummary ?? activity?.summary ?? null,
      blockKey: activity?.blockKey ?? detail?.blockKey ?? "",
    };
  }, [activity, detail]);

  async function doCorrect(category: string) {
    if (!view) return;
    try {
      await correctActivity(view.blockKey, view.source, view.label, view.title, category);
      loadDetail();
      setHistory(await getCorrectionHistory(view.blockKey));
      await onCorrected?.();
    } catch (e) {
      setError(String(e));
    }
  }

  async function setTypeDefault(category: string) {
    if (!view || view.activityKind === "unknown" || view.activityKind === "system") return;
    const label = categoryMeta(category).label;
    if (!confirm(`Classify all recognised ${view.activityKind.replace(/_/g, " ")} activity as ${label}? Saved app/site rules and strong project matches still take priority.`)) return;
    try {
      const policies = await getClassificationPolicies();
      const index = policies.findIndex((policy) => policy.kinds.includes(view.activityKind));
      if (index >= 0) {
        policies[index] = { ...policies[index], category, enabled: true };
      } else {
        policies.push({
          id: `type-${view.activityKind}-${Date.now().toString(36)}`,
          name: `${view.activityKind.replace(/_/g, " ")} default`,
          category,
          kinds: [view.activityKind],
          terms: [],
          enabled: true,
          builtIn: false,
          priority: 60,
        });
      }
      await setClassificationPolicies(policies);
      setTypeAction(`All recognised ${view.activityKind.replace(/_/g, " ")} activity now defaults to ${label}.`);
      await onCorrected?.();
    } catch (e) {
      setError(String(e));
    }
  }
  async function undoLatestCorrection() {
    const latest = history.find((entry) => !entry.undoneAt);
    if (!latest || !view) return;
    try {
      await undoCorrection(latest.id);
      loadDetail();
      setHistory(await getCorrectionHistory(view.blockKey));
      await onCorrected?.();
    } catch (e) {
      setError(String(e));
    }
  }
  async function excludeFromProject() {
    if (!view?.projectName || view.source === "screen") return;
    const kind = view.source === "web" ? "website" : "app";
    if (!confirm(`Never match this ${kind} to “${view.projectName}”? Other projects are not affected.`)) return;
    try {
      setProjectAction("Saving exclusion…");
      await excludeActivityFromProject(view.projectName, view.source, view.label);
      setProjectAction(`${view.label} will no longer match ${view.projectName}.`);
      await onCorrected?.();
    } catch (e) {
      setProjectAction(null);
      setError(String(e));
    }
  }

  if (!open) return null;

  const heading = view?.source === "web"
    ? "Website details"
    : view?.source === "screen"
      ? "Screen activity details"
      : "App details";

  return (
    <div className="drawer-overlay" onClick={onClose}>
      <aside className="drawer" onClick={(e) => e.stopPropagation()} aria-label={heading}>
        <div className="drawer-head">
          <div>
            <h2 className="card-title" style={{ fontSize: 15 }}>{heading}</h2>
            {view && <p className="drawer-kicker">{sourceName(view.source)}</p>}
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="Close details">✕</button>
        </div>

        {error && <div className="error-box">{error}</div>}
        {!view && !error && <div className="loading">Loading…</div>}

        {view && (
          <div className="drawer-body">
            <Row label={view.source === "web" ? "Domain" : "App"}>{view.label}</Row>
            <Row label={view.source === "web" ? "Page title" : "Window title"}>{view.title || "—"}</Row>
            {detail?.url && (
              <Row label="URL">
                <span className="detail-url" title={detail.url}>{detail.url}</span>
              </Row>
            )}
            <Row label="Duration">
              {formatDuration(view.seconds)}
              {detail?.isIdle && <span className="muted-num"> · idle</span>}
            </Row>
            {view.source === "screen" && view.lastSeen && (
              <Row label="Observed">{new Date(view.lastSeen).toLocaleString()}</Row>
            )}
            {view.activityKind !== "unknown" && (
              <Row label="Activity type">{view.activityKind.replace(/_/g, " ")}</Row>
            )}
            {view.contentType && (
              <Row label="Type">
                {contentTypeMeta(view.contentType).icon} {contentTypeMeta(view.contentType).label}
              </Row>
            )}
            <Row label="Category"><CategoryBadge category={view.category} /></Row>
            <Row label="Project">
              {view.projectName ? (
                <ProjectTag name={view.projectName} confidence={view.projectConfidence} />
              ) : (
                <span className="muted-num">No project assigned</span>
              )}
            </Row>

            <div className="detail-section match-explanation">
              <div className="detail-section-title">Why was this matched?</div>

              <div className="match-evidence-block">
                <div className="match-evidence-head">
                  <span>Category match</span>
                  <span className="match-evidence-meta">
                    <ClassificationSource classifier={view.classifier} />
                    {view.confidence != null && (
                      <span className="confidence-value">{Math.round(view.confidence * 100)}%</span>
                    )}
                  </span>
                </div>
                <p>{explainCategory(view.reason, view.source, view.category, view.classifier)}</p>
                <div className="technical-reason">Recorded reason: {view.reason}</div>
              </div>

              <div className="match-evidence-block">
                <div className="match-evidence-head">
                  <span>Project match</span>
                  {view.projectName && <span className="confidence-value">{view.projectConfidence}%</span>}
                </div>
                {view.projectName ? (
                  <>
                    <p>
                      Assigned to <strong>{view.projectName}</strong> because the evidence reached Tempo's 60% project threshold.
                    </p>
                    {view.projectSignals.length > 0 && (
                      <ul className="evidence-list">
                        {view.projectSignals.map((signal) => (
                          <li key={signal}>{explainProjectSignal(signal)}</li>
                        ))}
                      </ul>
                    )}
                    {view.source !== "screen" && (
                      <div className="project-match-action">
                        <button className="btn btn-small" onClick={excludeFromProject}>
                          Never match this {view.source === "web" ? "website" : "app"} to {view.projectName}
                        </button>
                        <small>Adds one explicit exclusion to this project. Other projects are unaffected.</small>
                      </div>
                    )}
                    {projectAction && <div className="success-inline" role="status">{projectAction}</div>}
                  </>
                ) : (
                  <p>No project reached the 60% evidence threshold, so Tempo did not guess a project.</p>
                )}
              </div>
            </div>

            <div className="detail-section">
              <div className="detail-section-title">Correct this activity</div>
              <p className="detail-help">Your correction is saved as a reusable {view.source === "web" ? "website" : "app"} rule.</p>
              <div className="correct-bar wrap">
                {categories.map((c) => (
                  <button key={c.id} className="correct-btn" onClick={() => doCorrect(c.id)}>
                    <span className="dot" style={{ background: c.color }} />
                    {c.label}
                  </button>
                ))}
                <button className="correct-btn ignore" onClick={() => doCorrect("ignore")}>Ignore</button>
              </div>
            </div>

            {view.activityKind !== "unknown" && view.activityKind !== "system" && (
              <div className="detail-section">
                <div className="detail-section-title">Default for this activity type</div>
                <p className="detail-help">
                  Apply one policy to every recognised {view.activityKind.replace(/_/g, " ")} activity. Specific app, website and strong project rules still win.
                </p>
                <select className="select" defaultValue="" onChange={(e) => e.target.value && setTypeDefault(e.target.value)}>
                  <option value="" disabled>Choose a default…</option>
                  {categories.map((category) => <option key={category.id} value={category.id}>{category.label}</option>)}
                </select>
                {typeAction && <div className="success-inline" role="status">{typeAction}</div>}
              </div>
            )}

            {history.length > 0 && (
              <div className="detail-section correction-history">
                <div className="detail-section-title">Correction history</div>
                {history.slice(0, 3).map((entry, index) => (
                  <div className={`correction-history-row${entry.undoneAt ? " undone" : ""}`} key={entry.id}>
                    <div>
                      <strong>{entry.newCategory === "ignore" ? "Ignored" : categoryMeta(entry.newCategory).label}</strong>
                      <span>{new Date(entry.createdAt).toLocaleString()}{entry.undoneAt ? " · undone" : ""}</span>
                    </div>
                    {index === 0 && !entry.undoneAt && <button className="btn btn-small" onClick={undoLatestCorrection}>Undo</button>}
                  </div>
                ))}
              </div>
            )}
            {view.summary && (
              <div className="detail-section">
                <div className="detail-section-title">Content summary</div>
                <p className="summary-text">{view.summary}</p>
              </div>
            )}

            {detail && detail.detectedKeywords.length > 0 && (
              <div className="detail-section">
                <div className="detail-section-title">Detected content signals</div>
                <div className="kw-chips">
                  {detail.detectedKeywords.map((keyword) => (
                    <span className="kw-chip" key={keyword}>{keyword}</span>
                  ))}
                </div>
              </div>
            )}

            {detail?.rawTextExcerpt && (
              <div className="detail-section">
                <div className="detail-section-title">Raw text excerpt</div>
                <pre className="raw-box">{detail.rawTextExcerpt}</pre>
              </div>
            )}
          </div>
        )}
      </aside>
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="detail-row">
      <div className="detail-label">{label}</div>
      <div className="detail-value">{children}</div>
    </div>
  );
}
