import { useEffect, useState } from "react";
import { getStreaks, getWeeklyReview } from "../api";
import type { Page } from "../components/Sidebar";
import { StatCard } from "../components/ui";
import { formatDuration } from "../format";
import type { Streak, WeeklyDay, WeeklyReview as Weekly } from "../types";
import { StreakHeatmap, streakIcon } from "./Streaks";

const VERDICT_COLOR = (score: number) =>
  score >= 85 ? "#16a34a" : score >= 70 ? "#2563eb" : score >= 50 ? "#d97706" : score >= 30 ? "#ea580c" : "#dc2626";

export default function WeeklyReview({ onNavigate }: { onNavigate: (page: Page) => void }) {
  const [data, setData] = useState<Weekly | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getWeeklyReview().then(setData).catch((e) => setError(String(e)));
  }, []);

  if (error) {
    return (
      <>
        <Head />
        <div className="error-box">{error}</div>
      </>
    );
  }
  if (!data) {
    return (
      <>
        <Head />
        <div className="loading">Crunching your week…</div>
      </>
    );
  }

  const maxTracked = Math.max(1, ...data.days.map((d) => d.trackedSeconds));
  const nextDecisions = [
    data.bestDay
      ? `Repeat what worked on ${data.bestDay.weekday}: protect one focus block for your main mission.`
      : "Schedule one protected focus block for your main mission.",
    data.mostCommonLeak
      ? `Decide the rule for ${data.mostCommonLeak} before it pulls another ${formatDuration(data.mostCommonLeakSeconds)}.`
      : "Keep distraction rules unchanged; there is no meaningful leak to chase yet.",
    "Choose one concrete output for next week and make it your first mission.",
  ];

  return (
    <>
      <Head range={`${data.startDay} → ${data.endDay}`} />

      <div className="stat-grid">
        <StatCard chip="#16a34a" label="Productive time" value={formatDuration(data.productiveSeconds)} foot="this week" />
        <StatCard chip="#dc2626" label="Distraction time" value={formatDuration(data.distractionSeconds)} foot="this week" />
        <StatCard label="Study time" value={formatDuration(data.studySeconds)} foot="this week" />
      </div>

      {data.checkinTotals.length > 0 && (
        <div className="card card-pad section-gap">
          <h2 className="card-title">Check-ins this week</h2>
          <div className="checkin-summary">
            {data.checkinTotals.map((c) => (
              <span key={c.id} className="ci-pill on">
                {c.icon} {c.label}: {c.kind === "counter" ? `×${c.total}` : `${c.total} day${c.total === 1 ? "" : "s"}`}
              </span>
            ))}
          </div>
        </div>
      )}

      <div className="card card-pad section-gap">
        <h2 className="card-title">Daily breakdown</h2>
        <p className="card-hint">Productive vs. distraction time each day, with that day's score.</p>
        <div className="week-chart">
          {data.days.map((d) => {
            const ph = (d.productiveSeconds / maxTracked) * 100;
            const dh = (d.distractionSeconds / maxTracked) * 100;
            return (
              <div className="week-col" key={d.day}>
                <div className="week-bar">
                  <div
                    className="week-seg dist"
                    style={{ height: `${dh}%` }}
                    title={`Distraction ${formatDuration(d.distractionSeconds)}`}
                  />
                  <div
                    className="week-seg prod"
                    style={{ height: `${ph}%` }}
                    title={`Productive ${formatDuration(d.productiveSeconds)}`}
                  />
                </div>
                <div className="week-score" style={{ color: VERDICT_COLOR(d.score) }}>
                  {d.trackedSeconds > 0 ? d.score : "—"}
                </div>
                <div className="week-day">{d.weekday}</div>
              </div>
            );
          })}
        </div>
        <div className="legend" style={{ marginTop: 6 }}>
          <div className="legend-item">
            <span className="legend-dot" style={{ background: "#16a34a" }} />
            <span>Productive</span>
          </div>
          <div className="legend-item">
            <span className="legend-dot" style={{ background: "#dc2626" }} />
            <span>Distraction</span>
          </div>
        </div>
      </div>

      <div className="three-col section-gap">
        <DayCard title="🏆 Best day" day={data.bestDay} />
        <DayCard title="💀 Worst day" day={data.worstDay} />
        <div className="card card-pad">
          <h3 className="mini-card-title">🕳️ Most common time leak</h3>
          {data.mostCommonLeak ? (
            <>
              <div className="leak-name">{data.mostCommonLeak}</div>
              <div className="leak-sub">{formatDuration(data.mostCommonLeakSeconds)} this week</div>
            </>
          ) : (
            <div className="leak-sub">No distractions logged — clean week.</div>
          )}
        </div>
      </div>

      <div className="card card-pad section-gap weekly-decisions">
        <h2 className="card-title">Three decisions for next week</h2>
        <p className="card-hint">The review is useful only if it changes what you do next.</p>
        <ol>
          {nextDecisions.map((decision) => <li key={decision}>{decision}</li>)}
        </ol>
        <div className="review-actions">
          <button className="btn btn-primary" onClick={() => onNavigate("goals")}>Set next mission</button>
          {data.mostCommonLeak && <button className="btn" onClick={() => onNavigate("categories")}>Adjust distraction rule</button>}
          <button className="btn" onClick={() => onNavigate("focus")}>Start a focus block</button>
        </div>
      </div>

      <WeeklyStreaks />
    </>
  );
}

function WeeklyStreaks() {
  const [streaks, setStreaks] = useState<Streak[]>([]);
  useEffect(() => {
    getStreaks().then(setStreaks).catch(() => {});
  }, []);
  if (streaks.length === 0) return null;
  const top = [...streaks].sort((a, b) => b.current - a.current).slice(0, 6);
  return (
    <div className="card card-pad section-gap">
      <h2 className="card-title">🗓️ Streaks</h2>
      <p className="card-hint">Consistency on the behaviours that matter, across the last 4 weeks.</p>
      <div className="weekly-streaks">
        {top.map((s) => (
          <div key={s.id} className="weekly-streak-row">
            <span className="folder-icon">{streakIcon(s.id, s.kind)}</span>
            <span className="weekly-streak-name">{s.name}</span>
            <span className="weekly-streak-num">🔥 {s.current}</span>
            <span className="muted-num">best {s.best}</span>
            <StreakHeatmap calendar={s.calendar} />
          </div>
        ))}
      </div>
    </div>
  );
}

function DayCard({ title, day }: { title: string; day: WeeklyDay | null }) {
  return (
    <div className="card card-pad">
      <h3 className="mini-card-title">{title}</h3>
      {day ? (
        <>
          <div className="day-card-score" style={{ color: VERDICT_COLOR(day.score) }}>
            {day.score}
            <span className="day-card-den">/100</span>
          </div>
          <div className="leak-sub">
            {day.weekday} · {day.day}
          </div>
          <div className="leak-sub">{formatDuration(day.productiveSeconds)} productive</div>
        </>
      ) : (
        <div className="leak-sub">Not enough data yet.</div>
      )}
    </div>
  );
}

function Head({ range }: { range?: string }) {
  return (
    <div className="page-head">
      <div>
        <h1 className="page-title">Weekly Review</h1>
        <div className="page-subtitle">{range ?? "Your last 7 days at a glance"}</div>
      </div>
    </div>
  );
}
