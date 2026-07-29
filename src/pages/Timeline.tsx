import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { correctActivity, getCategoryDefinitions, getOutputEvents, getTimelineForDay } from "../api";
import { categoryMeta } from "../categories";
import { AppGlyph, CategoryBadge, ProjectTag, StatCard } from "../components/ui";
import { formatDuration } from "../format";
import type { CategoryDefinition, OutputEvent, TimelineBlock, TimelineDay } from "../types";
import { outputMeta } from "./OutputEvents";


type ViewMode = "overview" | "exact";
type CatFilter = "all" | "productive" | "distraction" | "study" | "business";
const CAT_FILTERS: { id: CatFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "productive", label: "Productive" },
  { id: "distraction", label: "Distractions" },
  { id: "study", label: "Study" },
  { id: "business", label: "Business" },
];

const SOURCE_LABEL: Record<string, string> = {
  desktop: "Desktop",
  browser: "Browser",
  screen: "Screen OCR",
};

function todayIso(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate(),
  ).padStart(2, "0")}`;
}

function timeOf(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/** The block's classifier, as a small chip (matches the Classifications). */
function SrcChip({ classifier }: { classifier: TimelineBlock["classifier"] }) {
  if (classifier === "llm")
    return <span className="src-chip llm" title="Classified by the local LLM">🤖 LLM</span>;
  if (classifier === "manual")
    return <span className="src-chip manual" title="Manual correction">✎ Manual</span>;
  return <span className="src-chip rule" title="Rule-based">Rule</span>;
}

function Badges({ b }: { b: TimelineBlock }) {
  return (
    <>
      {b.longestProductive && <span className="tl-badge win" title="Longest unbroken productive block">🏆 Longest focus</span>}
      {b.biggestDistraction && <span className="tl-badge bad" title="Biggest single distraction block">🕳️ Biggest leak</span>}
      {b.firstProductive && <span className="tl-badge first" title="First productive block of the day">🌅 First work</span>}
      {b.goalRelated && <span className="tl-badge goal" title="Tied to one of today's goals/projects">🎯 Goal</span>}
      {b.outputLinked && <span className="tl-badge output" title="Output / content-shipping work">📤 Output</span>}
    </>
  );
}

export default function Timeline() {
  const [day, setDay] = useState<string>(todayIso());

  const [viewMode, setViewMode] = useState<ViewMode>("overview");
  const [data, setData] = useState<TimelineDay | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [catF, setCatF] = useState<CatFilter>("all");
  const [sourceF, setSourceF] = useState<string>("all");
  const [projectF, setProjectF] = useState<string>("all");
  const [labelF, setLabelF] = useState<string>("all");
  const [correctingKey, setCorrectingKey] = useState<string | null>(null);

  const [outputs, setOutputs] = useState<OutputEvent[]>([]);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);

  const load = useCallback(async () => {
    try {
      const [tl, ev, cats] = await Promise.all([
        getTimelineForDay(day, 20),
        getOutputEvents(day),
        getCategoryDefinitions(),
      ]);
      setData(tl);
      setOutputs(ev);
      setCategories(cats);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [day]);

  useEffect(() => {
    load();
  }, [load]);

  const blocks = (viewMode === "overview" ? data?.overviewBlocks : data?.blocks) ?? [];

  const projects = useMemo(
    () => Array.from(new Set(blocks.map((b) => b.project).filter((p): p is string => !!p))).sort(),
    [blocks],
  );
  const labels = useMemo(
    () => Array.from(new Set(blocks.map((b) => b.label))).sort(),
    [blocks],
  );

  const visible = useMemo(
    () =>
      blocks.filter((b) => {
        if (sourceF !== "all" && b.source !== sourceF) return false;
        if (projectF !== "all" && (b.project ?? "") !== projectF) return false;
        if (labelF !== "all" && b.label !== labelF) return false;
        if (catF === "productive") return b.bucket === "productive";
        if (catF === "distraction") return b.bucket === "distracting";
        if (catF === "study") return b.category === "study";
        if (catF === "business") return b.category === "business";
        return true;
      }),
    [blocks, sourceF, projectF, labelF, catF],
  );
  const hasOutputs = Boolean(data?.goals.length || (data?.outputs.length ?? 0) > 0);

  const maxDur = useMemo(
    () => Math.max(60, ...visible.map((b) => b.durationSeconds)),
    [visible],
  );

  function corrSource(b: TimelineBlock): string {
    return b.isWeb ? "web" : b.source === "screen" ? "screen" : "app";
  }

  async function doCorrect(b: TimelineBlock, category: string) {
    try {
      await correctActivity(b.blockKey, corrSource(b), b.label, b.title, category);
      setCorrectingKey(null);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  // Output markers overlay the timeline (only on the unfiltered/all view).
  const visibleOutputs = useMemo(
    () =>
      catF === "all" && sourceF === "all" && labelF === "all"
        ? outputs.filter((o) => projectF === "all" || (o.project ?? "") === projectF)
        : [],
    [outputs, catF, sourceF, labelF, projectF],
  );

  type Item =
    | { t: number; kind: "block"; b: TimelineBlock }
    | { t: number; kind: "output"; o: OutputEvent };
  const items: Item[] = useMemo(() => {
    const arr: Item[] = visible.map((b) => ({ t: new Date(b.start).getTime(), kind: "block", b }));
    for (const o of visibleOutputs) {
      arr.push({ t: new Date(o.modifiedAt ?? o.timestamp).getTime(), kind: "output", o });
    }
    arr.sort((a, b) => a.t - b.t);
    return arr;
  }, [visible, visibleOutputs]);

  const renderOutput = (o: OutputEvent) => {
    const m = outputMeta(o.eventType);
    return (
      <div className="tl-row output-row" key={`o${o.id}`}>
        <div className="tl-time">
          <div className="tl-clock">{timeOf(o.modifiedAt ?? o.timestamp)}</div>
        </div>
        <div className="tl-rail">
          <span className="tl-dot star" style={{ background: m.color }}>★</span>
          <span className="tl-line" />
        </div>
        <div className="tl-body">
          <div className="tl-output-marker" style={{ borderColor: m.color }}>
            <span className="folder-icon">{m.icon}</span>
            <div className="tl-info">
              <div className="tl-title ellip">
                Output: {o.fileName}
                <span className="out-badge" style={{ color: m.color, borderColor: m.color, marginLeft: 8 }}>
                  {m.label}
                </span>
              </div>
              <div className="tl-sub muted-num">
                {o.project ? `${o.project} · ` : ""}
                {o.linkedLabel ? `shipped during ${o.linkedLabel}` : "detected output"}
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  };

  const renderBlock = (b: TimelineBlock) => {
    const meta = categoryMeta(b.category);
    const barPct = Math.max(5, Math.round((b.durationSeconds / maxDur) * 100));
    const isOverview = viewMode === "overview";
    const displayTitle = b.idle ? "Idle / away" : isOverview ? b.project || b.label : b.title || b.label;
    // Category confidence and project confidence are different signals. The
    // project chip must show only the deterministic project-match confidence.
    const conf = b.projectConfidence;
    return (
      <Fragment key={b.blockKey + b.start}>
        <div className={`tl-row ${b.idle ? "idle" : ""}`}>
          <div className="tl-time">
            <div className="tl-clock">{timeOf(b.start)}</div>
            <div className="tl-dur">{formatDuration(b.durationSeconds)}</div>
          </div>
          <div className="tl-rail">
            <span className="tl-dot" style={{ background: b.idle ? "#cbd5e1" : meta.color }} />
            <span className="tl-line" />
          </div>
          <div className="tl-body">
            <div className="tl-bar-track">
              <div
                className="tl-bar"
                style={{ width: `${barPct}%`, background: b.idle ? "#cbd5e1" : meta.color }}
              />
            </div>
            <div className="tl-main">
              <AppGlyph name={b.label} />
              <div className="tl-info">
                <div className="tl-title ellip">{displayTitle}</div>
                <div className="tl-sub muted-num ellip">
                  {!isOverview && <span className="src-tag">{SOURCE_LABEL[b.source] ?? b.source}</span>}
                  {b.label} · {timeOf(b.start)}–{timeOf(b.end)}
                  {!isOverview && b.sampleCount > 1 ? ` · ${b.sampleCount} samples` : ""}
                </div>
                {!isOverview && b.summary && <div className="tl-summary ellip">{b.summary}</div>}
                <div className="tl-badges">
                  <Badges b={b} />
                  {isOverview && b.absorbedCount > 0 && (
                    <span
                      className="tl-badge smooth"
                      title={`${b.absorbedCount} brief switch${b.absorbedCount === 1 ? "" : "es"} (${formatDuration(b.absorbedSeconds)}) kept in Exact view`}
                    >
                      ↪ {formatDuration(b.absorbedSeconds)} brief switch ignored
                    </span>
                  )}
                </div>
              </div>
              <div className="tl-right">
                {!isOverview && b.project && <ProjectTag name={b.project} confidence={conf} />}
                <CategoryBadge category={b.category} />
                {!isOverview && <SrcChip classifier={b.classifier} />}
                {!isOverview && (
                  <button
                    className="icon-btn"
                    title="Correct classification"
                    onClick={() => setCorrectingKey(correctingKey === b.blockKey ? null : b.blockKey)}
                  >
                    ✎
                  </button>
                )}
              </div>
            </div>
            {!isOverview && correctingKey === b.blockKey && (
              <div className="correct-bar tl-correct">
                <span className="correct-label">Mark as</span>
                {categories.map((c) => (
                  <button key={c.id} className="correct-btn" onClick={() => doCorrect(b, c.id)}>
                    <span className="dot" style={{ background: c.color }} />
                    {c.label}
                  </button>
                ))}
                <button className="correct-btn ignore" onClick={() => doCorrect(b, "ignore")}>
                  Ignore block
                </button>
              </div>
            )}
          </div>
        </div>
      </Fragment>
    );
  };

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Activity</h1>
          <div className="page-subtitle">
            {viewMode === "overview"
              ? "Meaningful runs of work, without a one-sample accidental switch breaking your flow."
              : "Every captured app and website switch, with full classification details."}
          </div>
        </div>
        <div className="head-actions tl-controls">
          <div className="segmented" aria-label="Activity detail level">
            <button
              className={viewMode === "overview" ? "on" : ""}
              onClick={() => { setViewMode("overview"); setCorrectingKey(null); }}
            >
              Overview{data ? ` (${data.overviewBlocks.length})` : ""}
            </button>
            <button className={viewMode === "exact" ? "on" : ""} onClick={() => setViewMode("exact")}>
              Exact{data ? ` (${data.blocks.length})` : ""}
            </button>
          </div>
          <input
            type="date"
            className="tl-date"
            value={day}
            max={todayIso()}
            onChange={(e) => setDay(e.target.value || todayIso())}
          />

        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      {!data ? (
        <div className="card"><div className="loading">Loading timeline…</div></div>
      ) : (
        <>
          {viewMode === "overview" && (
            <div className="tl-mode-note" role="note">
              Showing {data.overviewBlocks.length} meaningful runs from {data.blocks.length} exact runs. Overview ignores only an accidental detour of 20 seconds or less when the same activity resumes immediately. Exact data and totals are unchanged.
            </div>
          )}
          {viewMode === "exact" && (
            <div className="tl-mode-note" role="note">
              Exact groups consecutive 10-second samples of the same activity into a run, but does not join them across a longer gap. Use Classifications to audit or correct a category.
            </div>
          )}

          <div className="stat-grid">
            <StatCard
              label="Active time"
              value={formatDuration(data.activeSeconds)}
              foot={`${formatDuration(data.idleSeconds)} idle`}
            />
            <StatCard
              label="Productive"
              value={formatDuration(data.productiveSeconds)}
              chip={categoryMeta("productive").color}
            />
            <StatCard
              label="Distraction"
              value={formatDuration(data.distractedSeconds)}
              chip={categoryMeta("distraction").color}
            />
            <StatCard
              label="First productive"
              value={data.firstProductiveStart ? timeOf(data.firstProductiveStart) : "—"}
              foot={data.firstProductiveStart ? "earliest real work" : "no work yet"}
            />
          </div>

          {hasOutputs ? (
            <div className="card card-pad tl-outputs">
              <span className="tl-outputs-title">✓ Logged today</span>
              {data.outputs.map((c) => (
                <span key={c.id} className="tl-out-chip on">
                  {c.icon} {c.kind === "counter" && c.value > 1 ? `${c.label} ×${c.value}` : c.label}
                </span>
              ))}
              {data.goals.length > 0 && (
                <span className="tl-goals" title="Today's goals">🎯 {data.goals.join(" · ")}</span>
              )}
            </div>
          ) : null}

          {/* Filters */}
          <div className="tl-filters">
            <div className="segmented">
              {CAT_FILTERS.map((f) => (
                <button key={f.id} className={catF === f.id ? "on" : ""} onClick={() => setCatF(f.id)}>
                  {f.label}
                </button>
              ))}
            </div>
            <select className="pf-select" value={sourceF} onChange={(e) => setSourceF(e.target.value)}>
              <option value="all">All sources</option>
              <option value="desktop">Desktop</option>
              <option value="browser">Browser</option>
              <option value="screen">Screen OCR</option>
            </select>
            <select className="pf-select" value={projectF} onChange={(e) => setProjectF(e.target.value)}>
              <option value="all">All projects</option>
              {projects.map((p) => (
                <option key={p} value={p}>{p}</option>
              ))}
            </select>
            <select className="pf-select" value={labelF} onChange={(e) => setLabelF(e.target.value)}>
              <option value="all">All apps / sites</option>
              {labels.map((l) => (
                <option key={l} value={l}>{l}</option>
              ))}
            </select>
          </div>

          <div className="card tl-list">
            {items.length === 0 ? (
              <div className="empty">
                <div className="empty-glyph">📈</div>
                <h3>No blocks for this view</h3>
                <p>Use your computer for a bit, or loosen the filters.</p>
              </div>
            ) : (
              items.map((it) => (it.kind === "output" ? renderOutput(it.o) : renderBlock(it.b)))
            )}
          </div>
        </>
      )}
    </>
  );
}
