import { useCallback, useEffect, useState } from "react";
import {
  getDailyScore,
  resetScoringWeights,
  setScoringThreshold,
  setScoringWeight,
} from "../api";
import { formatDuration, formatLongDate } from "../format";
import type { ScoreLine, ScoreReport } from "../types";

const VERDICT_COLOR: Record<string, string> = {
  excellent: "#16a34a",
  good: "#2563eb",
  mid: "#d97706",
  bad: "#ea580c",
  cooked: "#dc2626",
};

export default function DailyScore() {
  const [report, setReport] = useState<ScoreReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showWeights, setShowWeights] = useState(false);

  const load = useCallback(async () => {
    try {
      setReport(await getDailyScore());
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

      <div className="card card-pad section-gap">
        <h2 className="card-title">Check-ins</h2>
        <p className="card-hint">
          Your goals &amp; quick check-ins live on the <b>Daily Goals</b> page — they feed this score
          automatically.
        </p>
        <div className="checkin-summary">
          <span className={`ci-pill ${r.mainGoalCompleted ? "on" : "off"}`}>
            {r.mainGoalCompleted ? "✓" : "○"} Main goal{r.mainGoalName ? `: ${r.mainGoalName}` : ""}
          </span>
          <span className={`ci-pill ${r.gymLogged ? "on" : "off"}`}>
            {r.gymLogged ? "✓" : "○"} Gym / wrestling
          </span>
          <span className={`ci-pill ${r.videosPosted > 0 ? "on" : "off"}`}>
            🎬 {r.videosPosted} video{r.videosPosted === 1 ? "" : "s"}
          </span>
        </div>
      </div>

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

      <div className="card section-gap">
        <div className="card-pad score-breakdown-head">
          <div>
            <h2 className="card-title">Full breakdown</h2>
            <p className="card-hint" style={{ margin: 0 }}>Every rule and its contribution today.</p>
          </div>
          <button className="btn" onClick={() => setShowWeights((v) => !v)}>
            {showWeights ? "Hide weights" : "Edit weights"}
          </button>
        </div>
        <table className="app-table">
          <thead>
            <tr>
              <th>Rule</th>
              <th style={{ width: 90 }}>Today</th>
              {showWeights && <th style={{ width: 170 }}>Weight / threshold</th>}
              <th className="right" style={{ width: 80 }}>Points</th>
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
              </tr>
            ))}
          </tbody>
        </table>
        {showWeights && (
          <div className="card-pad" style={{ borderTop: "1px solid var(--border)" }}>
            <button className="btn btn-danger" onClick={() => run(resetScoringWeights)}>
              Reset weights to defaults
            </button>
          </div>
        )}
      </div>
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

