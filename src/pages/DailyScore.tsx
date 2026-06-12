import { useCallback, useEffect, useState } from "react";
import {
  deleteScoreRule,
  getCategoryDefinitions,
  getCheckinDefinitions,
  getDailyScore,
  resetScoringWeights,
  setScoringThreshold,
  setScoringWeight,
  upsertScoreRule,
} from "../api";
import { formatDuration, formatLongDate } from "../format";
import type {
  CategoryDefinition,
  CheckinDefinition,
  ScoreLine,
  ScoreReport,
  ScoreRule,
  ScoreRuleKind,
} from "../types";

const VERDICT_COLOR: Record<string, string> = {
  excellent: "#16a34a",
  good: "#2563eb",
  mid: "#d97706",
  bad: "#ea580c",
  cooked: "#dc2626",
};

const RULE_KIND_OPTIONS: { value: ScoreRuleKind; label: string; needsMetric: "checkin" | "category" | "text" | "none"; hasThreshold: boolean }[] = [
  { value: "checkin", label: "Check-in logged", needsMetric: "checkin", hasThreshold: true },
  { value: "category", label: "Minutes in a category", needsMetric: "category", hasThreshold: true },
  { value: "target", label: "Minutes on an app/site", needsMetric: "text", hasThreshold: true },
  { value: "goal", label: "Main goal completed", needsMetric: "none", hasThreshold: false },
  { value: "no_goal", label: "Main goal NOT completed", needsMetric: "none", hasThreshold: false },
  { value: "late_start", label: "First productive block after N o'clock", needsMetric: "none", hasThreshold: true },
];

export default function DailyScore() {
  const [report, setReport] = useState<ScoreReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showWeights, setShowWeights] = useState(false);
  const [checkinDefs, setCheckinDefs] = useState<CheckinDefinition[]>([]);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);

  const [newLabel, setNewLabel] = useState("");
  const [newKind, setNewKind] = useState<ScoreRuleKind>("checkin");
  const [newMetric, setNewMetric] = useState("");
  const [newWeight, setNewWeight] = useState("10");
  const [newThreshold, setNewThreshold] = useState("1");

  const load = useCallback(async () => {
    try {
      const [r, c, cats] = await Promise.all([
        getDailyScore(),
        getCheckinDefinitions(),
        getCategoryDefinitions(),
      ]);
      setReport(r);
      setCheckinDefs(c);
      setCategories(cats);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function run(fn: () => Promise<unknown>) {
    try {
      await fn();
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function addRule() {
    const label = newLabel.trim();
    if (!label) {
      setError("Rule label is required");
      return;
    }
    const kindMeta = RULE_KIND_OPTIONS.find((k) => k.value === newKind);
    const metric = kindMeta?.needsMetric === "none" ? "" : newMetric.trim().toLowerCase();
    if (kindMeta?.needsMetric !== "none" && !metric) {
      setError(
        kindMeta?.needsMetric === "checkin"
          ? "Pick which check-in this rule reads"
          : kindMeta?.needsMetric === "category"
            ? "Pick a category"
            : "Enter the app/site name to watch",
      );
      return;
    }
    const id = label.toLowerCase().replace(/\s+/g, "_").replace(/[^a-z0-9_-]/g, "");
    const rule: ScoreRule = {
      id,
      label,
      kind: newKind,
      metric,
      weight: Math.max(-100, Math.min(100, parseInt(newWeight, 10) || 0)),
      threshold: kindMeta?.hasThreshold ? Math.max(0, parseInt(newThreshold, 10) || 0) : null,
      builtIn: false,
    };
    await run(() => upsertScoreRule(rule));
    setNewLabel("");
  }

  async function removeRule(l: ScoreLine) {
    if (!confirm(`Delete the rule "${l.label}"?`)) return;
    await run(() => deleteScoreRule(l.id));
  }

  if (error && !report) {
    return (
      <>
        <Head />
        <div className="error-box">{error}</div>
      </>
    );
  }
  if (!report) {
    return (
      <>
        <Head />
        <div className="loading">Computing today's score…</div>
      </>
    );
  }

  const r = report;
  const color = VERDICT_COLOR[r.verdict] ?? "#64748b";
  const checkinPills = [
    r.mainGoalName || r.mainGoalCompleted ? (
      <span key="main" className={`ci-pill ${r.mainGoalCompleted ? "on" : "off"}`}>
        {r.mainGoalCompleted ? "✓" : "○"} Main goal{r.mainGoalName ? `: ${r.mainGoalName}` : ""}
      </span>
    ) : null,
    ...r.checkins.map((c) => (
      <span key={c.id} className="ci-pill on">
        {c.icon} {c.label}
        {c.kind === "counter" && c.value > 1 ? ` ×${c.value}` : ""}
      </span>
    )),
  ].filter(Boolean);
  const hasScoreInputs =
    checkinPills.length > 0 ||
    r.categoryMinutes.length > 0 ||
    r.topWins.length > 0 ||
    r.biggestLeaks.length > 0;
  const showBreakdown = showWeights || hasScoreInputs;

  return (
    <>
      <Head date={r.date} />
      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      <div className="card card-pad score-hero">
        <div
          className="score-ring"
          style={{ ["--score" as string]: r.score, ["--ring" as string]: color } as React.CSSProperties}
        >
          <div className="score-ring-inner">
            <div className="score-num">{r.score}</div>
            <div className="score-den">/ 100</div>
          </div>
        </div>
        <div className="score-hero-body">
          <div className="verdict-pill" style={{ background: color }}>{r.verdict.toUpperCase()}</div>
          <p className="score-suggestion">💡 {r.suggestion}</p>
        </div>
      </div>

      {checkinPills.length > 0 && (
        <div className="card card-pad section-gap">
          <h2 className="card-title">Check-ins</h2>
          <p className="card-hint">
            Your goals &amp; quick check-ins live on the <b>Daily Goals</b> page — they feed this score
            automatically.
          </p>
          <div className="checkin-summary">{checkinPills}</div>
        </div>
      )}

      {hasScoreInputs ? (
        <div className="two-col section-gap">
          <div className="card card-pad">
            <h2 className="card-title">Top wins</h2>
            {r.topWins.length ? (
              r.topWins.map((l) => <LineRow key={l.id} l={l} />)
            ) : (
              <p className="muted-num">No wins logged yet today.</p>
            )}
          </div>
          <div className="card card-pad">
            <h2 className="card-title">Biggest time leaks</h2>
            {r.biggestLeaks.length ? (
              r.biggestLeaks.map((l) => <LineRow key={l.id} l={l} />)
            ) : (
              <p className="muted-num">No leaks — clean day.</p>
            )}
          </div>
        </div>
      ) : (
        <div className="card card-pad section-gap">
          <h2 className="card-title">No score inputs yet</h2>
          <p className="card-hint" style={{ margin: 0 }}>
            Add your own goals or let the tracker collect activity, then the score will populate.
          </p>
        </div>
      )}

      {r.categoryMinutes.length > 0 && (
        <div className="card card-pad section-gap">
          <h2 className="card-title">Time today</h2>
          <div className="cat-min-row">
            {r.categoryMinutes.map((c) => (
              <span className="cat-min" key={c.category}>
                <b>{formatDuration(c.minutes * 60)}</b> {c.category}
              </span>
            ))}
          </div>
        </div>
      )}

      {showBreakdown && <div className="card section-gap">
        <div className="card-pad score-breakdown-head">
          <div>
            <h2 className="card-title">Full breakdown</h2>
            <p className="card-hint" style={{ margin: 0 }}>Every rule and its contribution today.</p>
          </div>
          <button className="btn" onClick={() => setShowWeights((v) => !v)}>
            {showWeights ? "Done editing" : "Edit rules"}
          </button>
        </div>
        <table className="app-table">
          <thead>
            <tr>
              <th>Rule</th>
              <th style={{ width: 90 }}>Today</th>
              {showWeights && <th style={{ width: 170 }}>Weight / threshold</th>}
              <th className="right" style={{ width: 80 }}>Points</th>
              {showWeights && <th style={{ width: 36 }} />}
            </tr>
          </thead>
          <tbody>
            {r.lines.map((l) => (
              <tr key={l.id} className={l.triggered ? "" : "row-dim"}>
                <td>
                  <span className={`tick ${l.triggered ? (l.positive ? "good" : "bad") : "off"}`}>
                    {l.triggered ? (l.positive ? "✓" : "✕") : "·"}
                  </span>
                  {l.label}
                </td>
                <td className="muted-num">{l.value}</td>
                {showWeights && (
                  <td>
                    <span className="inline-edit">
                      <input
                        className="search"
                        style={{ width: 58 }}
                        type="number"
                        defaultValue={l.weight}
                        onBlur={(e) => {
                          const v = Number(e.target.value);
                          if (v !== l.weight) run(() => setScoringWeight(l.id, v));
                        }}
                      />
                      {l.hasThreshold && (
                        <input
                          className="search"
                          style={{ width: 58 }}
                          type="number"
                          title="threshold"
                          defaultValue={l.threshold ?? 0}
                          onBlur={(e) => {
                            const v = Number(e.target.value);
                            if (v !== l.threshold) run(() => setScoringThreshold(l.id, v));
                          }}
                        />
                      )}
                    </span>
                  </td>
                )}
                <td
                  className="right"
                  style={{
                    fontVariantNumeric: "tabular-nums",
                    fontWeight: 600,
                    color: l.triggered ? (l.weight >= 0 ? "var(--good)" : "var(--bad)") : "var(--text-faint)",
                  }}
                >
                  {l.triggered ? (l.weight >= 0 ? `+${l.weight}` : `${l.weight}`) : "—"}
                </td>
                {showWeights && (
                  <td>
                    <button
                      className="icon-btn"
                      title="Delete rule"
                      aria-label={`Delete ${l.label}`}
                      onClick={() => removeRule(l)}
                    >
                      ×
                    </button>
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
        {showWeights && (
          <div className="card-pad rule-editor" style={{ borderTop: "1px solid var(--border)" }}>
            <h3 className="mini-card-title">Add a rule</h3>
            <p className="card-hint">
              Positive weights reward hitting the threshold; negative weights penalize going over it.
            </p>
            <div className="rule-add-row">
              <input
                className="search"
                placeholder="Label, e.g. Posted 2+ affiliate videos"
                value={newLabel}
                onChange={(e) => setNewLabel(e.target.value)}
              />
              <select
                className="select"
                value={newKind}
                onChange={(e) => {
                  setNewKind(e.target.value as ScoreRuleKind);
                  setNewMetric("");
                }}
              >
                {RULE_KIND_OPTIONS.map((k) => (
                  <option key={k.value} value={k.value}>{k.label}</option>
                ))}
              </select>
              {RULE_KIND_OPTIONS.find((k) => k.value === newKind)?.needsMetric === "checkin" && (
                <select className="select" value={newMetric} onChange={(e) => setNewMetric(e.target.value)}>
                  <option value="">Pick check-in…</option>
                  {checkinDefs.map((c) => (
                    <option key={c.id} value={c.id}>{c.icon} {c.label}</option>
                  ))}
                </select>
              )}
              {RULE_KIND_OPTIONS.find((k) => k.value === newKind)?.needsMetric === "category" && (
                <select className="select" value={newMetric} onChange={(e) => setNewMetric(e.target.value)}>
                  <option value="">Pick category…</option>
                  {categories.map((c) => (
                    <option key={c.id} value={c.id}>{c.label}</option>
                  ))}
                </select>
              )}
              {RULE_KIND_OPTIONS.find((k) => k.value === newKind)?.needsMetric === "text" && (
                <input
                  className="search"
                  placeholder="app/site, e.g. tiktok"
                  value={newMetric}
                  onChange={(e) => setNewMetric(e.target.value)}
                />
              )}
              <label className="folder-num">
                <input
                  className="pf-input"
                  type="number"
                  title="weight (points)"
                  value={newWeight}
                  onChange={(e) => setNewWeight(e.target.value)}
                />
                pts
              </label>
              {RULE_KIND_OPTIONS.find((k) => k.value === newKind)?.hasThreshold && (
                <label className="folder-num">
                  <input
                    className="pf-input"
                    type="number"
                    min={0}
                    title="threshold (minutes / count / hour)"
                    value={newThreshold}
                    onChange={(e) => setNewThreshold(e.target.value)}
                  />
                  thr
                </label>
              )}
              <button className="btn btn-primary" onClick={addRule}>
                Add rule
              </button>
            </div>
            <button className="btn btn-danger" onClick={() => run(resetScoringWeights)}>
              Reset rules to defaults
            </button>
          </div>
        )}
      </div>}
    </>
  );
}

function Head({ date }: { date?: string }) {
  return (
    <div className="page-head">
      <div>
        <h1 className="page-title">Daily Score</h1>
        <div className="page-subtitle">{date ? formatLongDate(date) : "Your productivity score for today"}</div>
      </div>
    </div>
  );
}

function LineRow({ l }: { l: ScoreLine }) {
  return (
    <div className="line-row">
      <span className="line-pts" style={{ color: l.weight >= 0 ? "var(--good)" : "var(--bad)" }}>
        {l.weight >= 0 ? `+${l.weight}` : l.weight}
      </span>
      <span className="line-label">{l.label}</span>
      <span className="muted-num">{l.value}</span>
    </div>
  );
}
