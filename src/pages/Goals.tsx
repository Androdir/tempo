import { FormEvent, useCallback, useEffect, useState } from "react";
import {
  addGoal,
  copyLockinPlanToGoals,
  copyPreviousGoals,
  deleteGoal,
  getCheckins,
  getGoals,
  getProjects,
  setCheckin,
  setGoalRecurring,
  toggleGoal,
  updateGoal,
} from "../api";
import { isoOffset } from "../components/LockinPlan";
import type { CheckinState, Goal, GoalDraft, Priority, Project } from "../types";

const PRIORITIES: Priority[] = ["high", "medium", "low"];

// Quick check-in toggles (videos posted is a separate counter, below).
const TOGGLES: { field: string; key: keyof CheckinState; label: string; icon: string }[] = [
  { field: "gym_logged", key: "gymLogged", label: "Went gym", icon: "🏋️" },
  { field: "wrestled", key: "wrestled", label: "Wrestled", icon: "🤼" },
  { field: "studied", key: "studied", label: "Studied", icon: "📚" },
  { field: "edited_video", key: "editedVideo", label: "Edited video", icon: "✂️" },
  { field: "analysed_content", key: "analysedContent", label: "Analysed content", icon: "🔍" },
];

export default function Goals() {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [checkins, setCheckins] = useState<CheckinState | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [title, setTitle] = useState("");
  const [project, setProject] = useState("");
  const [target, setTarget] = useState("");
  const [priority, setPriority] = useState<Priority>("medium");
  const [recurring, setRecurring] = useState(false);
  const [editing, setEditing] = useState<{
    id: number;
    title: string;
    project: string;
    target: string;
    priority: Priority;
    recurring: boolean;
    completed: boolean;
  } | null>(null);

  const load = useCallback(async () => {
    try {
      const [g, c, p] = await Promise.all([getGoals(), getCheckins(), getProjects()]);
      setGoals(g);
      setCheckins(c);
      setProjects(p);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function add(e: FormEvent) {
    e.preventDefault();
    const t = title.trim();
    if (!t || busy) return;
    const minutes = target ? Math.max(1, parseInt(target, 10) || 0) : null;
    const draft: GoalDraft = {
      title: t,
      project: project || null,
      targetMinutes: minutes,
      priority,
      recurring,
    };
    setBusy(true);
    try {
      await addGoal(draft);
      setTitle("");
      setProject("");
      setTarget("");
      setPriority("medium");
      setRecurring(false);
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function toggle(g: Goal) {
    try {
      await toggleGoal(g.id, !g.completed);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleRecurring(g: Goal) {
    try {
      await setGoalRecurring(g.id, !g.recurring);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  function startEdit(g: Goal) {
    setEditing({
      id: g.id,
      title: g.title,
      project: g.project ?? "",
      target: g.targetMinutes == null ? "" : String(g.targetMinutes),
      priority: g.priority,
      recurring: g.recurring,
      completed: g.completed,
    });
  }

  async function saveEdit(e: FormEvent) {
    e.preventDefault();
    if (!editing) return;
    const t = editing.title.trim();
    if (!t) return;
    const minutes = editing.target ? Math.max(1, parseInt(editing.target, 10) || 0) : null;
    try {
      await updateGoal({
        id: editing.id,
        title: t,
        project: editing.project || null,
        targetMinutes: minutes,
        priority: editing.priority,
        completed: editing.completed,
        recurring: editing.recurring,
      });
      setEditing(null);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function copyYesterday() {
    try {
      await copyPreviousGoals();
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function useSuggestedPlan() {
    try {
      // Yesterday's plan was generated FOR today.
      await copyLockinPlanToGoals(isoOffset(-1));
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(id: number) {
    try {
      await deleteGoal(id);
      if (editing?.id === id) setEditing(null);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function flip(field: string, current: boolean) {
    try {
      await setCheckin(field, current ? 0 : 1);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function setVideos(n: number) {
    try {
      await setCheckin("videos_posted", Math.max(0, n));
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  const total = goals.length;
  const done = goals.filter((g) => g.completed).length;
  const endOfDay = new Date().getHours() >= 18 && total > 0 && done < total;
  const hasQuickCheckins = Boolean(
    checkins &&
      (checkins.videosPosted > 0 ||
        checkins.gymLogged ||
        checkins.wrestled ||
        checkins.studied ||
        checkins.editedVideo ||
        checkins.analysedContent),
  );

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Daily Goals</h1>
          <div className="page-subtitle">Main missions today — tick off what you finish</div>
        </div>
        {total > 0 && (
          <div className="goal-progress">
            <b>{done}</b> / {total} done
          </div>
        )}
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      {endOfDay && (
        <div className="eod-banner">
          🌙 End of day — confirm what you actually finished before the score &amp; review lock in.
        </div>
      )}

      <div className="card card-pad">
        <div className="missions-head">
          <h2 className="card-title">🎯 Main missions today</h2>
          <div className="missions-head-actions">
            <button className="link-btn" onClick={useSuggestedPlan}>
              🌙 Use suggested plan
            </button>
            <button className="link-btn" onClick={copyYesterday}>
              Copy yesterday →
            </button>
          </div>
        </div>
        {goals.length === 0 ? (
          <p className="empty-hint">
            No missions yet. Add your first one below, or copy yesterday’s — recurring ones (↻) come
            back automatically each day.
          </p>
        ) : (
          <ul className="goal-list">
            {goals.map((g) => (
              <li key={g.id} className={`goal-row ${g.completed ? "done" : ""} ${editing?.id === g.id ? "editing" : ""}`}>
                {editing?.id === g.id ? (
                  <form className="goal-edit-form" onSubmit={saveEdit}>
                    <input
                      className="pf-input goal-title-input"
                      value={editing.title}
                      onChange={(e) => setEditing({ ...editing, title: e.target.value })}
                      autoFocus
                    />
                    <select
                      className="pf-select"
                      value={editing.project}
                      onChange={(e) => setEditing({ ...editing, project: e.target.value })}
                    >
                      <option value="">No project</option>
                      {projects.map((p) => (
                        <option key={p.id} value={p.name}>
                          {p.name}
                        </option>
                      ))}
                    </select>
                    <input
                      className="pf-input goal-target-input"
                      type="number"
                      min="1"
                      placeholder="min"
                      value={editing.target}
                      onChange={(e) => setEditing({ ...editing, target: e.target.value })}
                    />
                    <select
                      className="pf-select"
                      value={editing.priority}
                      onChange={(e) => setEditing({ ...editing, priority: e.target.value as Priority })}
                    >
                      {PRIORITIES.map((p) => (
                        <option key={p} value={p}>
                          {p}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      className={`recur-toggle ${editing.recurring ? "on" : ""}`}
                      onClick={() => setEditing({ ...editing, recurring: !editing.recurring })}
                      title="Repeat this goal every day"
                    >
                      ↻ daily
                    </button>
                    <button className="btn btn-primary" type="submit" disabled={!editing.title.trim()}>
                      Save
                    </button>
                    <button className="btn" type="button" onClick={() => setEditing(null)}>
                      Cancel
                    </button>
                  </form>
                ) : (
                  <>
                    <button
                      className="goal-check"
                      onClick={() => toggle(g)}
                      aria-label={g.completed ? "Mark not done" : "Mark done"}
                    >
                      {g.completed ? "✓" : ""}
                    </button>
                    <div className="goal-main">
                      <div className="goal-title">{g.title}</div>
                      <div className="goal-meta">
                        <span className={`prio prio-${g.priority}`}>{g.priority}</span>
                        {g.project && <span className="goal-chip">{g.project}</span>}
                        {g.targetMinutes != null && <span className="goal-chip">{g.targetMinutes}m</span>}
                        {g.recurring && <span className="goal-chip recurring">↻ daily</span>}
                      </div>
                    </div>
                    <button className="goal-edit" onClick={() => startEdit(g)} aria-label="Edit goal">
                      Edit
                    </button>
                    <button
                      className={`goal-recur ${g.recurring ? "on" : ""}`}
                      onClick={() => toggleRecurring(g)}
                      title={g.recurring ? "Stop repeating daily" : "Repeat this goal daily"}
                      aria-label="Toggle recurring"
                    >
                      ↻
                    </button>
                    <button className="goal-del" onClick={() => remove(g.id)} aria-label="Delete goal">
                      ✕
                    </button>
                  </>
                )}
              </li>
            ))}
          </ul>
        )}

        <form className="goal-form" onSubmit={add}>
          <input
            className="pf-input goal-title-input"
            placeholder="Add a mission… e.g. Code for 60 min"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
          <select className="pf-select" value={project} onChange={(e) => setProject(e.target.value)}>
            <option value="">No project</option>
            {projects.map((p) => (
              <option key={p.id} value={p.name}>
                {p.name}
              </option>
            ))}
          </select>
          <input
            className="pf-input goal-target-input"
            type="number"
            min="1"
            placeholder="min"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
          />
          <select
            className="pf-select"
            value={priority}
            onChange={(e) => setPriority(e.target.value as Priority)}
          >
            {PRIORITIES.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
          <button
            type="button"
            className={`recur-toggle ${recurring ? "on" : ""}`}
            onClick={() => setRecurring((v) => !v)}
            title="Repeat this goal every day"
          >
            ↻ daily
          </button>
          <button className="btn btn-primary" type="submit" disabled={busy || !title.trim()}>
            Add
          </button>
        </form>
      </div>

      {checkins && hasQuickCheckins && (
        <div className="card card-pad section-gap">
          <h2 className="card-title">Quick check-ins</h2>
          <p className="card-hint">
            One tap for things the tracker can’t see. These feed your daily score and AI review.
          </p>
          <div className="checkin-grid">
            <div className={`checkin-chip ${checkins.videosPosted > 0 ? "on" : ""}`}>
              <span className="ci-icon">🎬</span>
              <span className="ci-label">Posted video</span>
              <span className="ci-stepper">
                <button type="button" onClick={() => setVideos(checkins.videosPosted - 1)}>
                  −
                </button>
                <b>{checkins.videosPosted}</b>
                <button type="button" onClick={() => setVideos(checkins.videosPosted + 1)}>
                  +
                </button>
              </span>
            </div>
            {TOGGLES.map((t) => {
              const on = Boolean(checkins[t.key]);
              return (
                <button
                  key={t.field}
                  type="button"
                  className={`checkin-chip ${on ? "on" : ""}`}
                  onClick={() => flip(t.field, on)}
                >
                  <span className="ci-icon">{t.icon}</span>
                  <span className="ci-label">{t.label}</span>
                  <span className="ci-state">{on ? "✓" : "+"}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </>
  );
}
