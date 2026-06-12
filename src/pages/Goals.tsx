import { FormEvent, useCallback, useEffect, useState } from "react";
import {
  addGoal,
  copyLockinPlanToGoals,
  copyPreviousGoals,
  deleteCheckinDefinition,
  deleteGoal,
  getCheckins,
  getGoals,
  getProjects,
  setCheckin,
  setGoalRecurring,
  toggleGoal,
  updateGoal,
  upsertCheckinDefinition,
} from "../api";
import { isoOffset } from "../components/LockinPlan";
import type { CheckinDefinition, CheckinValue, Goal, GoalDraft, Priority, Project } from "../types";

const PRIORITIES: Priority[] = ["high", "medium", "low"];

const EMPTY_CHECKIN: CheckinDefinition = { id: "", label: "", icon: "✅", kind: "toggle", builtIn: false };

export default function Goals() {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [checkins, setCheckins] = useState<CheckinValue[] | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [manageCheckins, setManageCheckins] = useState(false);
  const [checkinDraft, setCheckinDraft] = useState<CheckinDefinition>({ ...EMPTY_CHECKIN });

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

  async function setValue(id: string, value: number) {
    try {
      await setCheckin(id, value);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function saveCheckinDef() {
    const id =
      (checkinDraft.id || checkinDraft.label).trim().toLowerCase().replace(/\s+/g, "_").replace(/[^a-z0-9_-]/g, "");
    if (!id || !checkinDraft.label.trim()) {
      setError("Check-in label is required");
      return;
    }
    try {
      await upsertCheckinDefinition({ ...checkinDraft, id, label: checkinDraft.label.trim() });
      setCheckinDraft({ ...EMPTY_CHECKIN });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeCheckinDef(c: CheckinValue) {
    if (
      !confirm(
        `Delete check-in "${c.label}"? Its history, and any streaks or score rules built on it, will be removed.`,
      )
    )
      return;
    try {
      await deleteCheckinDefinition(c.id);
      if (checkinDraft.id === c.id) setCheckinDraft({ ...EMPTY_CHECKIN });
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  const total = goals.length;
  const done = goals.filter((g) => g.completed).length;
  const endOfDay = new Date().getHours() >= 18 && total > 0 && done < total;

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

      {checkins && (
        <div className="card card-pad section-gap">
          <div className="missions-head">
            <h2 className="card-title">Quick check-ins</h2>
            <button className="link-btn" onClick={() => setManageCheckins((v) => !v)}>
              {manageCheckins ? "Done" : "Edit check-ins"}
            </button>
          </div>
          <p className="card-hint">
            One tap for things the tracker can’t see. These feed your daily score, streaks and AI review.
          </p>
          {checkins.length === 0 ? (
            <p className="empty-hint">No check-ins defined — add the habits you want to log below.</p>
          ) : (
            <div className="checkin-grid">
              {checkins.map((c) =>
                c.kind === "counter" ? (
                  <div key={c.id} className={`checkin-chip ${c.value > 0 ? "on" : ""}`}>
                    <span className="ci-icon">{c.icon}</span>
                    <span className="ci-label">{c.label}</span>
                    <span className="ci-stepper">
                      <button type="button" onClick={() => setValue(c.id, c.value - 1)}>
                        −
                      </button>
                      <b>{c.value}</b>
                      <button type="button" onClick={() => setValue(c.id, c.value + 1)}>
                        +
                      </button>
                    </span>
                    {manageCheckins && (
                      <CheckinManageButtons
                        onEdit={() => setCheckinDraft({ id: c.id, label: c.label, icon: c.icon, kind: c.kind, builtIn: false })}
                        onDelete={() => removeCheckinDef(c)}
                      />
                    )}
                  </div>
                ) : (
                  <div key={c.id} className={`checkin-chip ${c.value > 0 ? "on" : ""}`}>
                    <button
                      type="button"
                      className="checkin-flip"
                      onClick={() => setValue(c.id, c.value > 0 ? 0 : 1)}
                    >
                      <span className="ci-icon">{c.icon}</span>
                      <span className="ci-label">{c.label}</span>
                      <span className="ci-state">{c.value > 0 ? "✓" : "+"}</span>
                    </button>
                    {manageCheckins && (
                      <CheckinManageButtons
                        onEdit={() => setCheckinDraft({ id: c.id, label: c.label, icon: c.icon, kind: c.kind, builtIn: false })}
                        onDelete={() => removeCheckinDef(c)}
                      />
                    )}
                  </div>
                ),
              )}
            </div>
          )}
          {manageCheckins && (
            <div className="checkin-editor">
              <input
                className="search"
                placeholder="Label, e.g. Posted affiliate video"
                value={checkinDraft.label}
                onChange={(e) => setCheckinDraft({ ...checkinDraft, label: e.target.value })}
              />
              <input
                className="search ci-icon-input"
                placeholder="✅"
                value={checkinDraft.icon}
                maxLength={4}
                onChange={(e) => setCheckinDraft({ ...checkinDraft, icon: e.target.value })}
                aria-label="Check-in icon (emoji)"
              />
              <select
                className="select"
                value={checkinDraft.kind}
                onChange={(e) =>
                  setCheckinDraft({ ...checkinDraft, kind: e.target.value as CheckinDefinition["kind"] })
                }
              >
                <option value="toggle">Done / not done</option>
                <option value="counter">Counter (×N)</option>
              </select>
              <button className="btn btn-primary" onClick={saveCheckinDef}>
                {checkinDraft.id ? "Save" : "Add"}
              </button>
              {checkinDraft.id && (
                <button className="btn" onClick={() => setCheckinDraft({ ...EMPTY_CHECKIN })}>
                  Cancel
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </>
  );
}

function CheckinManageButtons({ onEdit, onDelete }: { onEdit: () => void; onDelete: () => void }) {
  return (
    <span className="ci-manage">
      <button type="button" className="icon-btn" title="Edit check-in" onClick={onEdit}>
        ✎
      </button>
      <button type="button" className="icon-btn" title="Delete check-in" onClick={onDelete}>
        ×
      </button>
    </span>
  );
}
