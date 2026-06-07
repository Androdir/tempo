import { useCallback, useEffect, useState } from "react";
import { getStreakDefinitions, getStreaks, updateStreakDefinition } from "../api";
import type { Streak, StreakDay, StreakDefinition } from "../types";

export const STREAK_ICON: Record<string, string> = {
  posted_video: "🎬",
  main_goal: "🎯",
  coding_60: "💻",
  business_90: "💼",
  study_60: "📚",
  studied: "📖",
  edited_video: "✂️",
  analysed_content: "🔍",
  gym: "🏋️",
  wrestling: "🤼",
  productive_block_60: "🧠",
  no_major_distraction: "🛡️",
};

export function streakIcon(id: string): string {
  return STREAK_ICON[id] ?? "✅";
}

const THRESHOLD_KINDS = new Set(["category", "block", "distraction"]);

function fmtDay(iso: string): string {
  const d = new Date(`${iso}T00:00:00`);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function StreakHeatmap({ calendar }: { calendar: StreakDay[] }) {
  const todayIdx = calendar.length - 1;
  return (
    <div className="streak-grid" role="img" aria-label="Streak calendar">
      {calendar.map((c, i) => (
        <span
          key={c.day}
          className={`streak-cell ${c.met ? "met" : ""} ${i === todayIdx ? "today" : ""}`}
          title={`${fmtDay(c.day)} — ${c.met ? "done" : "missed"}`}
        />
      ))}
    </div>
  );
}

export default function Streaks() {
  const [streaks, setStreaks] = useState<Streak[] | null>(null);
  const [defs, setDefs] = useState<StreakDefinition[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [manage, setManage] = useState(false);

  const load = useCallback(async () => {
    try {
      const [s, d] = await Promise.all([getStreaks(), getStreakDefinitions()]);
      setStreaks(s);
      setDefs(d);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function toggle(id: string, enabled: boolean) {
    try {
      await updateStreakDefinition(id, { enabled });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function setThreshold(id: string, threshold: number) {
    try {
      await updateStreakDefinition(id, { threshold });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Streaks</h1>
          <div className="page-subtitle">Consistency on what matters — outputs and habits, not just hours logged.</div>
        </div>
        <div className="head-actions">
          <button className="btn" onClick={() => setManage((v) => !v)}>
            {manage ? "Done" : "Manage streaks"}
          </button>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      {manage && (
        <div className="card card-pad">
          <h2 className="card-title">Manage streaks</h2>
          <p className="card-hint">Turn streaks on/off and tune the thresholds that count as a “win”.</p>
          {defs.length === 0 ? (
            <p className="muted-num" style={{ marginBottom: 0 }}>
              No streaks created yet.
            </p>
          ) : (
            <ul className="streak-manage-list">
              {defs.map((d) => (
                <li key={d.id} className="streak-manage-row">
                  <span className="folder-icon">{streakIcon(d.id)}</span>
                  <span className="streak-manage-name">{d.name}</span>
                  {THRESHOLD_KINDS.has(d.kind) && (
                    <label className="folder-num">
                      <input
                        className="pf-input"
                        type="number"
                        min={1}
                        defaultValue={d.threshold}
                        onBlur={(e) => {
                          const v = parseInt(e.target.value, 10);
                          if (v > 0 && v !== d.threshold) setThreshold(d.id, v);
                        }}
                        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                      />
                      min
                    </label>
                  )}
                  <button
                    className={`pill-toggle ${d.enabled ? "on" : ""}`}
                    onClick={() => toggle(d.id, !d.enabled)}
                  >
                    {d.enabled ? "On" : "Off"}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {!streaks ? (
        <div className="card"><div className="loading">Loading streaks…</div></div>
      ) : streaks.length === 0 ? (
        <div className="card">
          <div className="empty">
            <div className="empty-glyph">🗓️</div>
            <h3>No streaks yet</h3>
            <p>Create streaks when you know what habits you want to track.</p>
          </div>
        </div>
      ) : (
        <div className="streak-grid-cards">
          {streaks.map((s) => (
            <div key={s.id} className={`card card-pad streak-card ${s.current > 0 ? "active" : ""}`}>
              <div className="streak-head">
                <span className="streak-icon">{streakIcon(s.id)}</span>
                <span className="streak-name">{s.name}</span>
              </div>
              <div className="streak-figures">
                <div className="streak-current">
                  <span className="streak-flame">{s.current > 0 ? "🔥" : "·"}</span>
                  <span className="streak-num">{s.current}</span>
                  <span className="streak-unit">day{s.current === 1 ? "" : "s"}</span>
                </div>
                <div className="streak-best">best {s.best}</div>
              </div>
              <StreakHeatmap calendar={s.calendar} />
              <div className="streak-foot muted-num">
                {s.lastCompletedDay ? `last: ${fmtDay(s.lastCompletedDay)}` : "not yet"}
                <span className="streak-window">4-week view</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
