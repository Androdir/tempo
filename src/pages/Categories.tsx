import { useCallback, useEffect, useMemo, useState } from "react";
import {
  deleteCategoryRule,
  getTrackedApps,
  getTrackedDomains,
  setCategoryRule,
  setDomainRule,
} from "../api";
import {
  BUCKET_LIST,
  BUCKET_META,
  captureModeMeta,
  CATEGORY_LIST,
  CATEGORY_META,
} from "../categories";
import { AppGlyph } from "../components/ui";
import { formatDuration } from "../format";
import type { Category, TrackedApp, TrackedDomain } from "../types";

type Tab = "apps" | "websites";

export default function Categories() {
  const [tab, setTab] = useState<Tab>("apps");
  const [apps, setApps] = useState<TrackedApp[]>([]);
  const [domains, setDomains] = useState<TrackedDomain[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  const load = useCallback(async () => {
    try {
      const [a, d] = await Promise.all([getTrackedApps(), getTrackedDomains()]);
      setApps(a);
      setDomains(d);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function changeApp(a: TrackedApp, value: string) {
    const category = value === "" ? null : (value as Category);
    setApps((prev) => prev.map((x) => (x.appName === a.appName ? { ...x, category } : x)));
    try {
      if (category) await setCategoryRule(a.appName, category, a.aiReview);
      else await deleteCategoryRule(a.appName);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  async function toggleAppAi(a: TrackedApp, aiReview: boolean) {
    if (!a.category) return; // need a category rule before flagging an app
    setApps((prev) => prev.map((x) => (x.appName === a.appName ? { ...x, aiReview } : x)));
    try {
      await setCategoryRule(a.appName, a.category, aiReview);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  async function changeDomain(d: TrackedDomain, value: string) {
    const category = value === "" ? null : (value as Category);
    setDomains((prev) => prev.map((x) => (x.domain === d.domain ? { ...x, category } : x)));
    try {
      // Preserve the existing capture mode + AI-review flag.
      await setDomainRule(d.domain, category, d.captureMode, d.aiReview);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  async function toggleDomainAi(d: TrackedDomain, aiReview: boolean) {
    setDomains((prev) => prev.map((x) => (x.domain === d.domain ? { ...x, aiReview } : x)));
    try {
      await setDomainRule(d.domain, d.category, d.captureMode, aiReview);
    } catch (e) {
      setError(String(e));
      load();
    }
  }

  const q = filter.trim().toLowerCase();
  const visibleApps = useMemo(
    () => (q ? apps.filter((a) => a.appName.toLowerCase().includes(q)) : apps),
    [apps, q]
  );
  const visibleDomains = useMemo(
    () => (q ? domains.filter((d) => d.domain.toLowerCase().includes(q)) : domains),
    [domains, q]
  );

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Categories</h1>
          <div className="page-subtitle">Tag apps and websites — rules are saved locally and reused.</div>
        </div>
        <div className="head-actions">
          <div className="segmented">
            <button className={tab === "apps" ? "on" : ""} onClick={() => setTab("apps")}>Apps</button>
            <button className={tab === "websites" ? "on" : ""} onClick={() => setTab("websites")}>Websites</button>
          </div>
          <input
            className="search"
            placeholder={`Filter ${tab}…`}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
      </div>

      <BucketLegend />

      {error && <div className="error-box section-gap">{error}</div>}

      <div className="card section-gap">
        {loading ? (
          <div className="loading">Loading…</div>
        ) : tab === "apps" ? (
          <CategoryTable
            empty={apps.length === 0}
            emptyTitle="No apps tracked yet"
            emptyHint="Once the tracker has seen a few apps, they'll appear here."
            rows={visibleApps.map((a) => ({
              key: a.appName,
              name: a.appName,
              seconds: a.totalSeconds,
              category: a.category,
              extra: null,
              onChange: (v: string) => changeApp(a, v),
              aiReview: a.aiReview,
              aiDisabled: !a.category,
              onToggleAi: (v: boolean) => toggleAppAi(a, v),
            }))}
          />
        ) : (
          <CategoryTable
            empty={domains.length === 0}
            emptyTitle="No websites tracked yet"
            emptyHint="Install the browser extension (extension/README.md) to see websites here."
            rows={visibleDomains.map((d) => ({
              key: d.domain,
              name: d.domain,
              seconds: d.totalSeconds,
              category: d.category,
              extra: (
                <span className="badge" style={{ color: captureModeMeta(d.captureMode).color }}>
                  {captureModeMeta(d.captureMode).label}
                </span>
              ),
              onChange: (v: string) => changeDomain(d, v),
              aiReview: d.aiReview,
              onToggleAi: (v: boolean) => toggleDomainAi(d, v),
            }))}
          />
        )}
      </div>
    </>
  );
}

interface Row {
  key: string;
  name: string;
  seconds: number;
  category: Category | null;
  extra: React.ReactNode;
  onChange: (value: string) => void;
  aiReview: boolean;
  aiDisabled?: boolean;
  onToggleAi: (value: boolean) => void;
}

function CategoryTable({
  rows,
  empty,
  emptyTitle,
  emptyHint,
}: {
  rows: Row[];
  empty: boolean;
  emptyTitle: string;
  emptyHint: string;
}) {
  if (empty) {
    return (
      <div className="empty">
        <div className="empty-glyph">🏷️</div>
        <h3>{emptyTitle}</h3>
        <p>{emptyHint}</p>
      </div>
    );
  }
  if (rows.length === 0) {
    return (
      <div className="empty">
        <div className="empty-glyph">🔍</div>
        <h3>No matches</h3>
        <p>Try a different search term.</p>
      </div>
    );
  }
  return (
    <table className="app-table">
      <thead>
        <tr>
          <th>Name</th>
          <th className="right">Tracked</th>
          <th style={{ width: 190 }}>Category</th>
          <th style={{ width: 120 }}>AI review</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <td>
              <div className="app-cell">
                <AppGlyph name={r.name} />
                {r.name}
                {r.extra}
              </div>
            </td>
            <td className="right muted-num">{formatDuration(r.seconds)}</td>
            <td>
              <select
                className="select"
                value={r.category ?? ""}
                onChange={(e) => r.onChange(e.target.value)}
                style={{ width: "100%" }}
              >
                <option value="">Uncategorized</option>
                {CATEGORY_LIST.map((c) => (
                  <option key={c} value={c}>{CATEGORY_META[c].label}</option>
                ))}
              </select>
            </td>
            <td>
              <label
                className="ai-toggle"
                title={
                  r.aiDisabled
                    ? "Set a category first"
                    : "Always send this to the local LLM for review"
                }
              >
                <input
                  type="checkbox"
                  checked={r.aiReview}
                  disabled={r.aiDisabled}
                  onChange={(e) => r.onToggleAi(e.target.checked)}
                />
                <span>review</span>
              </label>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function BucketLegend() {
  return (
    <div className="card card-pad">
      <h2 className="card-title">How categories roll up</h2>
      <p className="card-hint">The six categories group into three buckets on your dashboard.</p>
      <div className="bucket-legend">
        {BUCKET_LIST.map((bucket) => (
          <div className="bucket-col" key={bucket}>
            <div className="bucket-head">
              <span className="legend-dot" style={{ background: BUCKET_META[bucket].color }} />
              {BUCKET_META[bucket].label}
            </div>
            <div className="bucket-cats">
              {CATEGORY_LIST.filter((c) => CATEGORY_META[c].bucket === bucket).map((c) => (
                <div className="bucket-cat" key={c}>
                  <span className="dot" style={{ background: CATEGORY_META[c].color }} />
                  <span>{CATEGORY_META[c].label}</span>
                  <span className="bucket-cat-blurb">{CATEGORY_META[c].blurb}</span>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
