import { useEffect, useMemo, useState } from "react";
import { getTimeBreakdown } from "../api";
import { BUCKET_META, categoryMeta } from "../categories";
import { AppGlyph, CategoryBadge, StackedBar, StatCard } from "../components/ui";
import { formatDuration } from "../format";
import type { Bucket, TimeBreakdown as Breakdown } from "../types";

type Preset = "today" | "7d" | "30d" | "custom";
type KindFilter = "all" | "app" | "site";
type BucketFilter = "all" | Bucket;

function localIso(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function presetRange(preset: Exclude<Preset, "custom">): { start: string; end: string } {
  const end = new Date();
  const start = new Date(end);
  const days = preset === "today" ? 1 : preset === "7d" ? 7 : 30;
  start.setDate(start.getDate() - days + 1);
  return { start: localIso(start), end: localIso(end) };
}

export default function TimeBreakdown() {
  const initial = presetRange("7d");
  const [preset, setPreset] = useState<Preset>("7d");
  const [startDate, setStartDate] = useState(initial.start);
  const [endDate, setEndDate] = useState(initial.end);
  const [kind, setKind] = useState<KindFilter>("all");
  const [bucket, setBucket] = useState<BucketFilter>("all");
  const [search, setSearch] = useState("");
  const [data, setData] = useState<Breakdown | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!startDate || !endDate || startDate > endDate) return;
    let cancelled = false;
    setLoading(true);
    getTimeBreakdown(startDate, endDate)
      .then((report) => {
        if (!cancelled) { setData(report); setError(null); }
      })
      .catch((reason) => { if (!cancelled) setError(String(reason)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [startDate, endDate]);

  const choosePreset = (next: Preset) => {
    setPreset(next);
    if (next !== "custom") {
      const range = presetRange(next);
      setStartDate(range.start);
      setEndDate(range.end);
    }
  };

  const rows = useMemo(() => {
    if (!data) return [];
    const combined = [
      ...data.perApp.map((item) => ({
        key: `app:${item.appName}`, label: item.appName, seconds: item.seconds,
        category: item.category, kind: "app" as const,
      })),
      ...data.perWebsite.map((item) => ({
        key: `site:${item.domain}`, label: item.domain, seconds: item.seconds,
        category: item.category, kind: "site" as const,
      })),
    ].sort((a, b) => b.seconds - a.seconds || a.label.localeCompare(b.label));
    const needle = search.trim().toLowerCase();
    return combined.filter((item) => {
      if (kind !== "all" && item.kind !== kind) return false;
      if (bucket !== "all" && categoryMeta(item.category).bucket !== bucket) return false;
      return !needle || item.label.toLowerCase().includes(needle);
    });
  }, [data, kind, bucket, search]);

  const bucketSeconds = (target: Bucket) =>
    data?.perBucket.find((item) => item.bucket === target)?.seconds ?? 0;
  const visibleTotal = rows.reduce((sum, row) => sum + row.seconds, 0);
  const maxSeconds = rows[0]?.seconds ?? 1;

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Time breakdown</h1>
          <div className="page-subtitle">See exactly which apps and websites consume your time</div>
        </div>
      </div>

      <div className="card card-pad breakdown-controls">
        <div className="segmented" aria-label="Date range">
          {([["today", "Today"], ["7d", "7 days"], ["30d", "30 days"], ["custom", "Custom"]] as const).map(([value, label]) => (
            <button key={value} className={preset === value ? "on" : ""} onClick={() => choosePreset(value)}>{label}</button>
          ))}
        </div>
        {preset === "custom" && (
          <div className="breakdown-dates">
            <label>From<input className="search" type="date" value={startDate} max={endDate} onChange={(e) => setStartDate(e.target.value)} /></label>
            <label>To<input className="search" type="date" value={endDate} min={startDate} onChange={(e) => setEndDate(e.target.value)} /></label>
          </div>
        )}
        <p className="card-hint breakdown-truth">Totals use the same reconciled blocks as Activity, including Smart Tracking and browser overlap removal.</p>
      </div>

      {error && <div className="error-box section-gap">{error}</div>}
      {loading && !data ? <div className="loading">Calculating the breakdown…</div> : data && (
        <>
          <div className="stat-grid section-gap">
            <StatCard label="Active time" value={formatDuration(data.totalActiveSeconds)} foot={`${data.dayCount} day${data.dayCount === 1 ? "" : "s"}`} />
            <StatCard chip={BUCKET_META.productive.color} label="Productive" value={formatDuration(bucketSeconds("productive"))} />
            <StatCard chip={BUCKET_META.distracting.color} label="Distracting" value={formatDuration(bucketSeconds("distracting"))} />
            <StatCard label="Browser" value={formatDuration(data.totalBrowserSeconds)} foot="identified websites" />
          </div>

          <div className="card card-pad section-gap">
            <div className="breakdown-filter-row">
              <div className="segmented" aria-label="Activity type">
                {([["all", "All"], ["app", "Apps"], ["site", "Websites"]] as const).map(([value, label]) => (
                  <button key={value} className={kind === value ? "on" : ""} onClick={() => setKind(value)}>{label}</button>
                ))}
              </div>
              <select className="select" value={bucket} onChange={(e) => setBucket(e.target.value as BucketFilter)} aria-label="Focus bucket">
                <option value="all">All categories</option>
                <option value="productive">Productive</option>
                <option value="neutral">Neutral</option>
                <option value="distracting">Distracting</option>
              </select>
              <input className="search" placeholder="Find an app or website…" value={search} onChange={(e) => setSearch(e.target.value)} />
            </div>
            <div className="breakdown-heading">
              <div>
                <h2 className="card-title">Most time spent</h2>
                <p className="card-hint">{rows.length} result{rows.length === 1 ? "" : "s"} · {formatDuration(visibleTotal)} shown</p>
              </div>
            </div>
            {rows.length === 0 ? (
              <div className="empty"><div className="empty-glyph">⏱️</div><h3>No matching activity</h3><p>Try a wider date range or remove a filter.</p></div>
            ) : (
              <div className="breakdown-list">
                {rows.map((row, index) => {
                  const meta = categoryMeta(row.category);
                  return (
                    <div className="breakdown-row" key={row.key}>
                      <span className="breakdown-rank">{index + 1}</span>
                      <AppGlyph name={row.label} size={30} />
                      <div className="breakdown-name">
                        <strong>{row.label}</strong>
                        <span>{row.kind === "app" ? "App" : "Website"} <CategoryBadge category={row.category} /></span>
                      </div>
                      <div className="breakdown-bar" aria-hidden="true"><span style={{ width: `${Math.max(2, (row.seconds / maxSeconds) * 100)}%`, background: meta.color }} /></div>
                      <strong className="breakdown-duration">{formatDuration(row.seconds)}</strong>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          <div className="card card-pad section-gap">
            <h2 className="card-title">Category split</h2>
            <StackedBar total={data.totalActiveSeconds} segments={data.perCategory.map((item) => ({ color: categoryMeta(item.category).color, value: item.seconds }))} />
            <div className="legend breakdown-legend">
              {data.perCategory.map((item) => (
                <div className="legend-item" key={item.category}><span className="legend-dot" style={{ background: categoryMeta(item.category).color }} /><span>{categoryMeta(item.category).label}</span><b>{formatDuration(item.seconds)}</b></div>
              ))}
            </div>
          </div>
        </>
      )}
    </>
  );
}
