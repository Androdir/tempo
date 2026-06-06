import { useCallback, useEffect, useState } from "react";
import { correctActivity, getActivityDetails } from "../api";
import { CATEGORY_LIST, CATEGORY_META, contentTypeMeta } from "../categories";
import { CategoryBadge, ProjectTag } from "./ui";
import { formatDuration } from "../format";
import type { ActivityDetail } from "../types";

export default function ActivityDetailsDrawer({
  id,
  onClose,
  onCorrected,
}: {
  id: number | null;
  onClose: () => void;
  onCorrected?: () => void;
}) {
  const [detail, setDetail] = useState<ActivityDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadDetail = useCallback(() => {
    if (id == null) return;
    getActivityDetails(id)
      .then(setDetail)
      .catch((e) => setError(String(e)));
  }, [id]);

  useEffect(() => {
    if (id == null) return;
    setDetail(null);
    setError(null);
    loadDetail();
  }, [id, loadDetail]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function doCorrect(category: string) {
    if (!detail) return;
    try {
      await correctActivity(detail.blockKey, "web", detail.domain, detail.pageTitle, category);
      loadDetail();
      onCorrected?.();
    } catch (e) {
      setError(String(e));
    }
  }

  if (id == null) return null;

  return (
    <div className="drawer-overlay" onClick={onClose}>
      <aside className="drawer" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-head">
          <h2 className="card-title" style={{ fontSize: 15 }}>Activity details</h2>
          <button className="icon-btn" onClick={onClose} aria-label="Close">✕</button>
        </div>

        {error && <div className="error-box">{error}</div>}
        {!detail && !error && <div className="loading">Loading…</div>}

        {detail && (
          <div className="drawer-body">
            <Row label="Domain">{detail.domain}</Row>
            <Row label="Title">{detail.pageTitle || "—"}</Row>
            <Row label="URL">
              <span className="detail-url" title={detail.url}>{detail.url}</span>
            </Row>
            <Row label="Duration">
              {formatDuration(detail.durationSeconds)}
              {detail.isIdle && <span className="muted-num"> · idle</span>}
            </Row>
            <Row label="Type">
              {contentTypeMeta(detail.contentType).icon}{" "}
              {contentTypeMeta(detail.contentType).label}
            </Row>
            <Row label="Category">
              <CategoryBadge category={detail.category} />
            </Row>
            <Row label="Project">
              {detail.projectName ? (
                <ProjectTag name={detail.projectName} confidence={detail.projectConfidence} />
              ) : (
                <span className="muted-num">No project match</span>
              )}
            </Row>
            <Row label="Classified by">
              {detail.classifier === "llm" ? (
                <span className="ai-badge">
                  🤖 Local LLM
                  {detail.llmConfidence != null ? ` ${Math.round(detail.llmConfidence * 100)}%` : ""}
                </span>
              ) : detail.classifier === "manual" ? (
                <span className="src-chip manual">✎ Manual correction</span>
              ) : (
                <span className="src-chip rule">
                  Rule-based · {Math.round(detail.confidence * 100)}% conf
                </span>
              )}
            </Row>
            <Row label="Why">
              <span className="muted-num">{detail.classificationReason}</span>
            </Row>

            <div className="detail-section">
              <div className="detail-section-title">Correct this activity</div>
              <div className="correct-bar wrap">
                {CATEGORY_LIST.map((c) => (
                  <button key={c} className="correct-btn" onClick={() => doCorrect(c)}>
                    <span className="dot" style={{ background: CATEGORY_META[c].color }} />
                    {CATEGORY_META[c].label}
                  </button>
                ))}
                <button className="correct-btn ignore" onClick={() => doCorrect("ignore")}>Ignore</button>
              </div>
            </div>

            <div className="detail-section">
              <div className="detail-section-title">Content summary</div>
              {detail.contentSummary ? (
                <p className="summary-text">{detail.contentSummary}</p>
              ) : (
                <p className="muted-num">No content captured for this page.</p>
              )}
            </div>

            {detail.detectedKeywords.length > 0 && (
              <div className="detail-section">
                <div className="detail-section-title">Detected keywords</div>
                <div className="kw-chips">
                  {detail.detectedKeywords.map((k) => (
                    <span className="kw-chip" key={k}>{k}</span>
                  ))}
                </div>
              </div>
            )}

            {detail.projectSignals.length > 0 && (
              <div className="detail-section">
                <div className="detail-section-title">Project match signals</div>
                <div className="kw-chips">
                  {detail.projectSignals.map((s) => (
                    <span className="kw-chip" key={s}>{s}</span>
                  ))}
                </div>
              </div>
            )}

            {detail.rawTextExcerpt && (
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
