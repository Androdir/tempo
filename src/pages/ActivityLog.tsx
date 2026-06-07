import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { correctActivity, getCategoryDefinitions, getRecentActivity } from "../api";
import { contentTypeMeta } from "../categories";
import ActivityDetailsDrawer from "../components/ActivityDetailsDrawer";
import { AppGlyph, CategoryBadge, ProjectTag } from "../components/ui";
import { formatDuration } from "../format";
import type { ActivityLogEntry, CategoryDefinition } from "../types";

type Filter = "all" | "app" | "web" | "screen";

function timeOf(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

function SrcChip({ classifier }: { classifier: ActivityLogEntry["classifier"] }) {
  if (classifier === "llm")
    return <span className="src-chip llm" title="Classified by the local LLM">🤖 LLM</span>;
  if (classifier === "manual")
    return <span className="src-chip manual" title="Manual correction">✎ Manual</span>;
  return <span className="src-chip rule" title="Rule-based classification">Rule</span>;
}

export default function ActivityLog() {
  const [entries, setEntries] = useState<ActivityLogEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openId, setOpenId] = useState<number | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [correctingKey, setCorrectingKey] = useState<string | null>(null);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);

  const load = useCallback(async () => {
    try {
      const [items, cats] = await Promise.all([getRecentActivity(), getCategoryDefinitions()]);
      setEntries(items);
      setCategories(cats);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
    const t = window.setInterval(load, 15000);
    return () => window.clearInterval(t);
  }, [load]);

  async function doCorrect(e: ActivityLogEntry, category: string) {
    try {
      await correctActivity(e.blockKey, e.source, e.label, e.title, category);
      setCorrectingKey(null);
      await load();
    } catch (err) {
      setError(String(err));
    }
  }

  const visible = useMemo(
    () => (entries ?? []).filter((e) => filter === "all" || e.source === filter),
    [entries, filter]
  );

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Activity Log</h1>
          <div className="page-subtitle">
            Hybrid classification: rules first, local LLM only when unsure — and you can correct any block.
          </div>
        </div>
        <div className="head-actions">
          <div className="segmented">
            <button className={filter === "all" ? "on" : ""} onClick={() => setFilter("all")}>All</button>
            <button className={filter === "app" ? "on" : ""} onClick={() => setFilter("app")}>Apps</button>
            <button className={filter === "web" ? "on" : ""} onClick={() => setFilter("web")}>Web</button>
            <button className={filter === "screen" ? "on" : ""} onClick={() => setFilter("screen")}>Screen</button>
          </div>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      <div className="card">
        {!entries ? (
          <div className="loading">Loading activity…</div>
        ) : visible.length === 0 ? (
          <div className="empty">
            <div className="empty-glyph">🗂️</div>
            <h3>No activity yet</h3>
            <p>Use your computer for a bit and activity will appear here.</p>
          </div>
        ) : (
          <table className="app-table">
            <thead>
              <tr>
                <th>Activity</th>
                <th style={{ width: 200 }}>Project</th>
                <th style={{ width: 190 }}>Category &amp; source</th>
                <th className="right" style={{ width: 70 }}>Time</th>
                <th style={{ width: 44 }}></th>
              </tr>
            </thead>
            <tbody>
              {visible.map((e) => {
                const clickable = e.detailId != null;
                const projConf =
                  e.classifier === "llm" && e.llmConfidence != null
                    ? Math.round(e.llmConfidence * 100)
                    : e.projectConfidence;
                return (
                  <Fragment key={e.blockKey}>
                    <tr
                      className={clickable ? "clickable" : ""}
                      onClick={clickable ? () => setOpenId(e.detailId) : undefined}
                    >
                      <td>
                        <div className="app-cell">
                          <AppGlyph name={e.label} />
                          <div style={{ minWidth: 0 }}>
                            <div className="ellip">{e.title || e.label}</div>
                            <div className="muted-num ellip" style={{ fontSize: 11.5 }}>
                              <span className="src-tag">{e.source}</span>
                              {e.label}
                              {e.contentType ? ` · ${contentTypeMeta(e.contentType).label}` : ""}
                            </div>
                            {e.summary && <div className="muted-num ellip log-summary">{e.summary}</div>}
                          </div>
                        </div>
                      </td>
                      <td>
                        {e.projectName ? (
                          <ProjectTag name={e.projectName} confidence={projConf} />
                        ) : (
                          <span className="muted-num">—</span>
                        )}
                      </td>
                      <td>
                        <div className="cat-src">
                          <CategoryBadge category={e.category} />
                          <SrcChip classifier={e.classifier} />
                        </div>
                      </td>
                      <td className="right muted-num">
                        {e.source === "screen" ? timeOf(e.lastSeen) : formatDuration(e.seconds)}
                      </td>
                      <td className="right">
                        <button
                          className="icon-btn"
                          title="Correct classification"
                          onClick={(ev) => {
                            ev.stopPropagation();
                            setCorrectingKey(correctingKey === e.blockKey ? null : e.blockKey);
                          }}
                        >
                          ✎
                        </button>
                      </td>
                    </tr>
                    {correctingKey === e.blockKey && (
                      <tr className="correct-row">
                        <td colSpan={5}>
                          <div className="correct-bar">
                            <span className="correct-label">Mark as</span>
                            {categories.map((c) => (
                              <button key={c.id} className="correct-btn" onClick={() => doCorrect(e, c.id)}>
                                <span className="dot" style={{ background: c.color }} />
                                {c.label}
                              </button>
                            ))}
                            <button className="correct-btn ignore" onClick={() => doCorrect(e, "ignore")}>
                              Ignore
                            </button>
                          </div>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      <ActivityDetailsDrawer id={openId} onClose={() => setOpenId(null)} onCorrected={load} />
    </>
  );
}
