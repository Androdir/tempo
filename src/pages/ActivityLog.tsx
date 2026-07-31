import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { correctActivity, getCategoryDefinitions, getRecentActivity } from "../api";
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

interface ActivityGroup {
  key: string;
  activity: ActivityLogEntry;
  entries: ActivityLogEntry[];
  sources: ActivityLogEntry["source"][];
  titles: string[];
  mixedCategory: boolean;
}

function groupActivities(entries: ActivityLogEntry[]): ActivityGroup[] {
  const grouped = new Map<string, ActivityLogEntry[]>();
  for (const entry of entries) {
    const family = entry.source === "web" ? "web" : "app";
    const key = `${family}:${entry.label.trim().toLocaleLowerCase()}`;
    grouped.set(key, [...(grouped.get(key) ?? []), entry]);
  }
  const rank = (entry: ActivityLogEntry) =>
    (entry.classifier === "manual" ? 30 : entry.classifier === "rule" ? 20 : 10)
    + (entry.source === "screen" ? 0 : 2);
  return Array.from(grouped, ([key, items]) => {
    const sorted = [...items].sort((a, b) =>
      rank(b) - rank(a) || new Date(b.lastSeen).getTime() - new Date(a.lastSeen).getTime(),
    );
    const timed = items.filter((entry) => entry.source !== "screen");
    const durationEntries = timed.length > 0 ? timed : items;
    const seconds = durationEntries.reduce((sum, entry) => sum + entry.seconds, 0);
    const lastSeen = items.reduce((latest, entry) =>
      new Date(entry.lastSeen).getTime() > new Date(latest).getTime() ? entry.lastSeen : latest,
    items[0].lastSeen);
    const titles = Array.from(new Set(items.map((entry) => entry.title.trim()).filter(Boolean)));
    const sources = Array.from(new Set(items.map((entry) => entry.source)));
    return {
      key,
      activity: { ...sorted[0], seconds, lastSeen },
      entries: items,
      sources,
      titles,
      mixedCategory: new Set(items.map((entry) => entry.category)).size > 1,
    };
  }).sort((a, b) => new Date(b.activity.lastSeen).getTime() - new Date(a.activity.lastSeen).getTime());
}

export default function ActivityLog() {
  const [entries, setEntries] = useState<ActivityLogEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openKey, setOpenKey] = useState<string | null>(null);
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
    () => (entries ?? []).filter((entry) => filter === "all" || entry.source === filter),
    [entries, filter],
  );
  const groups = useMemo(() => groupActivities(visible), [visible]);
  const openGroup = useMemo(
    () => groups.find((group) => group.key === openKey) ?? null,
    [groups, openKey],
  );
  const openActivity = openGroup?.activity ?? null;

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Classifications</h1>
          <div className="page-subtitle">
            One row per app or website. Window titles and screen observations stay available as context instead of appearing as duplicate apps.
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
        ) : groups.length === 0 ? (
          <div className="empty">
            <div className="empty-glyph">🗂️</div>
            <h3>No activity yet</h3>
            <p>Use your computer for a bit and activity will appear here.</p>
          </div>
        ) : (
          <table className="app-table activity-table">
            <thead>
              <tr>
                <th>Activity</th>
                <th className="activity-project-col">Project</th>
                <th className="activity-category-col">Category &amp; source</th>
                <th className="right activity-time-col">Time</th>
                <th className="activity-action-col"></th>
              </tr>
            </thead>
            <tbody>
              {groups.map((group) => {
                const e = group.activity;
                const projConf =
                  e.classifier === "llm" && e.llmConfidence != null
                    ? Math.round(e.llmConfidence * 100)
                    : e.projectConfidence;
                return (
                  <Fragment key={group.key}>
                    <tr
                      className="clickable"
                      tabIndex={0}
                      onClick={() => setOpenKey(group.key)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          setOpenKey(group.key);
                        }
                      }}
                    >
                      <td>
                        <div className="app-cell">
                          <AppGlyph name={e.label} />
                          <div style={{ minWidth: 0 }}>
                            <div className="ellip">{e.label}</div>
                            <div className="muted-num activity-contexts" style={{ fontSize: 11.5 }}>
                              <span className="activity-source-list">
                                {group.sources.map((source) => <span className="src-tag" key={source}>{source}</span>)}
                              </span>
                              <span className="ellip">
                                {group.entries.length === 1
                                  ? e.title || "One observed context"
                                  : `${group.entries.length} contexts${group.titles.length ? ` · ${group.titles.slice(0, 2).join(" · ")}${group.titles.length > 2 ? "…" : ""}` : ""}`}
                              </span>
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
                          {group.mixedCategory
                            ? <span className="src-chip conflict" title="Observed contexts currently have different categories">Mixed categories</span>
                            : <CategoryBadge category={e.category} />}
                          <SrcChip classifier={e.classifier} />
                        </div>
                      </td>
                      <td className="right muted-num">
                        {group.sources.every((source) => source === "screen") ? timeOf(e.lastSeen) : formatDuration(e.seconds)}
                      </td>
                      <td className="right">
                        <button
                          className="icon-btn"
                          title="Correct classification"
                          onClick={(ev) => {
                            ev.stopPropagation();
                            setCorrectingKey(correctingKey === group.key ? null : group.key);
                          }}
                        >
                          ✎
                        </button>
                      </td>
                    </tr>
                    {correctingKey === group.key && (
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

      <ActivityDetailsDrawer
        id={openActivity?.detailId ?? null}
        activity={openActivity}
        relatedActivities={openGroup?.entries ?? []}
        onClose={() => setOpenKey(null)}
        onCorrected={load}
      />
    </>
  );
}
