import { useCallback, useEffect, useMemo, useState } from "react";
import {
  addStreakDefinition,
  deleteStreakDefinition,
  getCategoryDefinitions,
  getCheckinDefinitions,
  getStreakDefinitions,
  getStreaks,
  seedDefaultStreaks,
  updateStreakDefinition,
} from "../api";
import type {
  CategoryDefinition,
  CheckinDefinition,
  Streak,
  StreakDay,
  StreakDefinition,
} from "../types";

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

const KIND_ICON: Record<string, string> = {
  checkin: "✅",
  goal: "🎯",
  category: "⏱️",
  output: "📤",
  block: "🧠",
  distraction: "🛡️",
};

export function streakIcon(id: string, kind?: string): string {
  return STREAK_ICON[id] ?? KIND_ICON[kind ?? ""] ?? "✅";
}

const THRESHOLD_KINDS = new Set(["category", "block", "distraction"]);

const KIND_OPTIONS: { value: string; label: string; hint: string }[] = [
  { value: "checkin", label: "Check-in logged", hint: "met when you tap the check-in that day" },
  { value: "category", label: "Minutes in a category", hint: "met after N tracked minutes" },
  { value: "goal", label: "Main goal completed", hint: "met when the top goal is ticked" },
  { value: "block", label: "Unbroken focus block", hint: "met after one N-minute productive block" },
  { value: "distraction", label: "No major distraction", hint: "met when no distraction block exceeds N minutes" },
];

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
  const [checkinDefs, setCheckinDefs] = useState<CheckinDefinition[]>([]);
  const [categories, setCategories] = useState<CategoryDefinition[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [manage, setManage] = useState(false);

  const [newName, setNewName] = useState("");
  const [newKind, setNewKind] = useState("checkin");
  const [newMetric, setNewMetric] = useState("");
  const [newThreshold, setNewThreshold] = useState("60");
  const [newCadence, setNewCadence] = useState("0"); // 0 = every day, 1..7 = days/week

  const load = useCallback(async () => {
    try {
      const [s, d, c, cats] = await Promise.all([
        getStreaks(),
        getStreakDefinitions(),
        getCheckinDefinitions(),
        getCategoryDefinitions(),
      ]);
      setStreaks(s);
      setDefs(d);
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

  const checkinIcon = useMemo(() => {
    const map: Record<string, string> = {};
    for (const c of checkinDefs) map[c.id] = c.icon;
    return map;
  }, [checkinDefs]);

  function iconFor(id: string, kind?: string, metric?: string): string {
    if (kind === "checkin" && metric) {
      const first = metric.split(",")[0].trim();
      if (checkinIcon[first]) return checkinIcon[first];
    }
    return streakIcon(id, kind);
  }

  async function run(fn: () => Promise<unknown>) {
    try {
      await fn();
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function addStreak() {
    const name = newName.trim();
    if (!name) {
      setError("Streak name is required");
      return;
    }
    const id = name.toLowerCase().replace(/\s+/g, "_").replace(/[^a-z0-9_-]/g, "");
    const needsMetric = newKind === "checkin" || newKind === "category";
    const metric = needsMetric ? newMetric : newKind === "distraction" ? "max_block" : newKind;
    if (needsMetric && !metric) {
      setError(newKind === "checkin" ? "Pick a check-in to track" : "Pick a category to track");
      return;
    }
    const threshold = THRESHOLD_KINDS.has(newKind) ? Math.max(1, parseInt(newThreshold, 10) || 0) : 0;
    const daysPerWeek = parseInt(newCadence, 10) || 0;
    await run(() => addStreakDefinition({ id, name, kind: newKind, metric, threshold, daysPerWeek }));
    setNewName("");
  }

  async function removeStreak(d: StreakDefinition) {
    if (!confirm(`Delete streak "${d.name}"? Its history view disappears (check-in data is kept).`)) return;
    await run(() => deleteStreakDefinition(d.id));
  }

  const metricOptions =
    newKind === "checkin"
      ? checkinDefs.map((c) => ({ value: c.id, label: `${c.icon} ${c.label}` }))
      : newKind === "category"
        ? categories.map((c) => ({ value: c.id, label: c.label }))
        : [];

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
          <p className="card-hint">
            Create streaks for the habits you're building, tune thresholds, or delete the ones that no longer
            matter.
          </p>

          <div className="streak-add-row">
            <input
              className="search"
              placeholder="Streak name, e.g. Posted affiliate video"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
            />
            <select
              className="select"
              value={newKind}
              onChange={(e) => {
                setNewKind(e.target.value);
                setNewMetric("");
              }}
            >
              {KIND_OPTIONS.map((k) => (
                <option key={k.value} value={k.value}>{k.label}</option>
              ))}
            </select>
            {metricOptions.length > 0 && (
              <select className="select" value={newMetric} onChange={(e) => setNewMetric(e.target.value)}>
                <option value="">{newKind === "checkin" ? "Pick check-in…" : "Pick category…"}</option>
                {metricOptions.map((m) => (
                  <option key={m.value} value={m.value}>{m.label}</option>
                ))}
              </select>
            )}
            {THRESHOLD_KINDS.has(newKind) && (
              <label className="folder-num">
                <input
                  className="pf-input"
                  type="number"
                  min={1}
                  value={newThreshold}
                  onChange={(e) => setNewThreshold(e.target.value)}
                />
                min
              </label>
            )}
            <select
              className="select"
              value={newCadence}
              onChange={(e) => setNewCadence(e.target.value)}
              title="How often this streak must be met"
            >
              <option value="0">Every day</option>
              {[1, 2, 3, 4, 5, 6].map((n) => (
                <option key={n} value={String(n)}>{n}×/week</option>
              ))}
            </select>
            <button className="btn btn-primary" onClick={addStreak}>
              Add streak
            </button>
          </div>
          <p className="card-hint">
            {KIND_OPTIONS.find((k) => k.value === newKind)?.hint}
            {newCadence !== "0" &&
              ` — weekly cadence: the streak counts consecutive weeks with ${newCadence}+ met days, so rest days never break it.`}
          </p>

          {defs.length === 0 ? (
            <p className="muted-num" style={{ marginBottom: 0 }}>
              No streaks yet — add one above, or{" "}
              <button className="link-inline" onClick={() => run(seedDefaultStreaks)}>
                start from the suggested set
              </button>
              .
            </p>
          ) : (
            <ul className="streak-manage-list">
              {defs.map((d) => (
                <li key={d.id} className="streak-manage-row">
                  <span className="folder-icon">{iconFor(d.id, d.kind, d.metric)}</span>
                  <span className="streak-manage-name">
                    {d.name}
                    {d.daysPerWeek > 0 && <span className="streak-cadence"> {d.daysPerWeek}×/wk</span>}
                  </span>
                  {THRESHOLD_KINDS.has(d.kind) && (
                    <label className="folder-num">
                      <input
                        className="pf-input"
                        type="number"
                        min={1}
                        defaultValue={d.threshold}
                        onBlur={(e) => {
                          const v = parseInt(e.target.value, 10);
                          if (v > 0 && v !== d.threshold) run(() => updateStreakDefinition(d.id, { threshold: v }));
                        }}
                        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                      />
                      min
                    </label>
                  )}
                  <button
                    className={`pill-toggle ${d.enabled ? "on" : ""}`}
                    onClick={() => run(() => updateStreakDefinition(d.id, { enabled: !d.enabled }))}
                  >
                    {d.enabled ? "On" : "Off"}
                  </button>
                  <button
                    className="icon-btn"
                    title="Delete streak"
                    aria-label={`Delete ${d.name}`}
                    onClick={() => removeStreak(d)}
                  >
                    ×
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
            <p>Create streaks for the habits you want to keep, or start from a suggested set.</p>
            <div className="empty-actions">
              <button className="btn btn-primary" onClick={() => setManage(true)}>
                Create a streak
              </button>
              <button className="btn" onClick={() => run(seedDefaultStreaks)}>
                Add suggested streaks
              </button>
            </div>
          </div>
        </div>
      ) : (
        <div className="streak-grid-cards">
          {streaks.map((s) => (
            <div key={s.id} className={`card card-pad streak-card ${s.current > 0 ? "active" : ""}`}>
              <div className="streak-head">
                <span className="streak-icon">{iconFor(s.id, s.kind, s.metric)}</span>
                <span className="streak-name">{s.name}</span>
                {s.daysPerWeek > 0 && (
                  <span className="streak-cadence" title={`Met on ${s.daysPerWeek}+ days per week`}>
                    {s.daysPerWeek}×/wk
                  </span>
                )}
              </div>
              <div className="streak-figures">
                <div className="streak-current">
                  <span className="streak-flame">{s.current > 0 ? "🔥" : "·"}</span>
                  <span className="streak-num">{s.current}</span>
                  <span className="streak-unit">
                    {s.daysPerWeek > 0
                      ? `week${s.current === 1 ? "" : "s"}`
                      : `day${s.current === 1 ? "" : "s"}`}
                  </span>
                </div>
                <div className="streak-best">best {s.best}</div>
              </div>
              {s.daysPerWeek > 0 && (
                <div className="streak-weekly muted-num">
                  this week: <b>{s.weekMetDays}</b>/{s.daysPerWeek} days
                  {s.weekMetDays >= s.daysPerWeek ? " ✓" : ""}
                </div>
              )}
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
