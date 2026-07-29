import { FormEvent, useCallback, useEffect, useState } from "react";
import {
  addGoal,
  clearCheckin,
  copyLockinPlanToGoals,
  copyPreviousGoals,
  deleteCheckinDefinition,
  deleteGoal,
  getCheckinDefinitions,
  getCheckins,
  getGoals,
  getOutputEvents,
  getProjects,
  setCheckin,
  setGoalRecurring,
  toggleGoal,
  updateGoal,
  upsertCheckinDefinition,
} from "../api";
import { isoOffset } from "../components/LockinPlan";
import type {
  CheckinAutoKind,
  CheckinDefinition,
  CheckinValue,
  Goal,
  GoalDraft,
  OutputEvent,
  Priority,
  Project,
} from "../types";

const PRIORITIES: Priority[] = ["high", "medium", "low"];
type TargetMode = "none" | "time" | "count";

const EMPTY_CHECKIN: CheckinDefinition = {
  id: "",
  label: "",
  icon: "✅",
  kind: "toggle",
  builtIn: false,
  autoKind: "",
  autoMetric: "",
  autoThreshold: 0,
};

const AUTO_OPTIONS: { value: CheckinAutoKind; label: string; hint: string }[] = [
  { value: "", label: "Manual only", hint: "You tap it yourself." },
  {
    value: "target",
    label: "Auto: app/site time",
    hint: "Ticks itself after N active minutes on a matching app or website — idle/AFK time never counts. You can still override it by hand.",
  },
  {
    value: "output",
    label: "Auto: detected file output",
    hint: "Ticks itself when the proof-of-output watcher detects N new files (match an output type or a watched-folder label; blank = any output). Counters count the files.",
  },
];

export default function Goals() {
  const [goals, setGoals] = useState<Goal[]>([]);
  const [checkins, setCheckins] = useState<CheckinValue[] | null>(null);
  const [checkinDefs, setCheckinDefs] = useState<CheckinDefinition[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [outputs, setOutputs] = useState<OutputEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [manageCheckins, setManageCheckins] = useState(false);
  const [checkinDraft, setCheckinDraft] = useState<CheckinDefinition>({ ...EMPTY_CHECKIN });
  const [checkinNotice, setCheckinNotice] = useState<string | null>(null);

  const [title, setTitle] = useState("");
  const [project, setProject] = useState("");
  const [targetMode, setTargetMode] = useState<TargetMode>("none");
  const [target, setTarget] = useState("");
  const [targetUnit, setTargetUnit] = useState("");
  const [priority, setPriority] = useState<Priority>("medium");
  const [recurring, setRecurring] = useState(false);
  const [editing, setEditing] = useState<{
    id: number;
    title: string;
    project: string;
    targetMode: TargetMode;
    target: string;
    targetUnit: string;
    priority: Priority;
    recurring: boolean;
    completed: boolean;
  } | null>(null);

  const load = useCallback(async () => {
    try {
      const [g, c, d, p, o] = await Promise.all([
        getGoals(),
        getCheckins(),
        getCheckinDefinitions(),
        getProjects(),
        getOutputEvents(new Date().toLocaleDateString("en-CA")),
      ]);
      setGoals(g);
      setCheckins(c);
      setCheckinDefs(d);
      setProjects(p);
      setOutputs(o);
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
    const value = target ? Math.max(1, parseInt(target, 10) || 0) : null;
    if (targetMode === "count" && value != null && !targetUnit.trim()) {
      setError("Enter what the count represents, such as video or post");
      return;
    }
    const draft: GoalDraft = {
      title: t,
      project: project || null,
      targetMinutes: targetMode === "time" ? value : null,
      targetCount: targetMode === "count" ? value : null,
      targetUnit: targetMode === "count" ? targetUnit.trim() : null,
      priority,
      recurring,
    };
    setBusy(true);
    try {
      await addGoal(draft);
      setTitle("");
      setProject("");
      setTargetMode("none");
      setTarget("");
      setTargetUnit("");
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
      targetMode: g.targetMinutes != null ? "time" : g.targetCount != null ? "count" : "none",
      target: g.targetMinutes != null ? String(g.targetMinutes) : g.targetCount != null ? String(g.targetCount) : "",
      targetUnit: g.targetUnit ?? "",
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
    const value = editing.target ? Math.max(1, parseInt(editing.target, 10) || 0) : null;
    if (editing.targetMode === "count" && value != null && !editing.targetUnit.trim()) {
      setError("Enter what the count represents, such as video or post");
      return;
    }
    try {
      await updateGoal({
        id: editing.id,
        title: t,
        project: editing.project || null,
        targetMinutes: editing.targetMode === "time" ? value : null,
        targetCount: editing.targetMode === "count" ? value : null,
        targetUnit: editing.targetMode === "count" ? editing.targetUnit.trim() : null,
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
    const nextValue = Math.max(0, value);
    const item = checkins?.find((c) => c.id === id);
    try {
      await setCheckin(id, nextValue);
      await load();
      setCheckinNotice(
        nextValue > 0
          ? `${item?.label ?? "Check-in"} logged for today. It now appears in Activity → Logged today and can count toward score rules, streaks, and reviews. It does not add tracked minutes or complete a Daily Goal.`
          : `${item?.label ?? "Check-in"} removed from today.`,
      );
    } catch (e) {
      setError(String(e));
    }
  }

  async function revertToAuto(id: string) {
    try {
      await clearCheckin(id);
      await load();
    } catch (e) {
      setError(String(e));
    }
  }

  function editCheckinDef(id: string) {
    const def = checkinDefs.find((d) => d.id === id);
    if (def) setCheckinDraft({ ...def });
  }

  async function saveCheckinDef() {
    const id =
      (checkinDraft.id || checkinDraft.label).trim().toLowerCase().replace(/\s+/g, "_").replace(/[^a-z0-9_-]/g, "");
    if (!id || !checkinDraft.label.trim()) {
      setError("Check-in label is required");
      return;
    }
    if (checkinDraft.autoKind === "target" && !checkinDraft.autoMetric.trim()) {
      setError("Enter the app/site name this check-in watches");
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

  function outputEvidence(goal: Goal): OutputEvent[] {
    if (goal.completed || goal.targetCount == null || !goal.targetUnit) return [];
    const unit = goal.targetUnit.toLowerCase();
    const allowed = unit.includes("video") || unit.includes("clip") || unit.includes("reel") || unit.includes("short")
      ? ["video_export"]
      : unit.includes("post")
        ? ["video_export"]
        : unit.includes("code") || unit.includes("commit") || unit.includes("feature")
          ? ["code_change"]
          : unit.includes("document") || unit.includes("proposal") || unit.includes("script") || unit.includes("article")
            ? ["document_created"]
            : [];
    if (allowed.length === 0) return [];
    return outputs.filter((output) =>
      allowed.includes(output.eventType)
      && (!goal.project || output.project?.toLowerCase() === goal.project.toLowerCase())
    );
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
                    <select
                      className="pf-select goal-target-mode"
                      value={editing.targetMode}
                      onChange={(e) => setEditing({ ...editing, targetMode: e.target.value as TargetMode, target: "", targetUnit: "" })}
                    >
                      <option value="none">No target</option>
                      <option value="time">Time target</option>
                      <option value="count">Output / count</option>
                    </select>
                    {editing.targetMode !== "none" && (
                      <input
                        className="pf-input goal-target-input"
                        type="number"
                        min="1"
                        placeholder={editing.targetMode === "time" ? "minutes" : "count"}
                        value={editing.target}
                        onChange={(e) => setEditing({ ...editing, target: e.target.value })}
                      />
                    )}
                    {editing.targetMode === "count" && (
                      <input
                        className="pf-input goal-unit-input"
                        placeholder="unit, e.g. video"
                        value={editing.targetUnit}
                        onChange={(e) => setEditing({ ...editing, targetUnit: e.target.value })}
                      />
                    )}
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
                        {g.targetMinutes != null && <span className="goal-chip">{g.targetMinutes} min</span>}
                        {g.targetCount != null && <span className="goal-chip">{g.targetCount} {g.targetUnit}</span>}
                        {g.recurring && <span className="goal-chip recurring">↻ daily</span>}
                        {outputEvidence(g).length > 0 && (
                          <>
                            <span className="goal-evidence-chip" title="File evidence is a suggestion, not proof it was published">
                              {outputEvidence(g).length} possible {g.targetUnit} output{outputEvidence(g).length === 1 ? "" : "s"}
                            </span>
                            <button className="goal-evidence-confirm" onClick={() => toggle(g)}>Confirm complete</button>
                          </>
                        )}
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

        <p className="card-hint goal-target-help">Targets are optional. Choose time for a timed mission, or output/count for something shippable such as 1 video, 3 clips, or 1 proposal. Completion is manual so Tempo never guesses whether you actually posted it.</p>
        <form className="goal-form" onSubmit={add}>
          <input
            className="pf-input goal-title-input"
            placeholder="Add a mission… e.g. Publish a video"
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
          <select
            className="pf-select goal-target-mode"
            value={targetMode}
            onChange={(e) => {
              setTargetMode(e.target.value as TargetMode);
              setTarget("");
              setTargetUnit("");
            }}
          >
            <option value="none">No target</option>
            <option value="time">Time target</option>
            <option value="count">Output / count</option>
          </select>
          {targetMode !== "none" && (
            <input
              className="pf-input goal-target-input"
              type="number"
              min="1"
              placeholder={targetMode === "time" ? "minutes" : "count"}
              value={target}
              onChange={(e) => setTarget(e.target.value)}
            />
          )}
          {targetMode === "count" && (
            <input
              className="pf-input goal-unit-input"
              placeholder="unit, e.g. video"
              value={targetUnit}
              onChange={(e) => setTargetUnit(e.target.value)}
            />
          )}
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
            Log something that happened but cannot be timed, such as reading offline or publishing a video. A check-in does not add minutes or finish a Daily Goal; it appears in Activity and affects a score or streak only when one uses it.
          </p>
          {checkinNotice && (
            <div className="checkin-feedback" role="status">
              <span aria-hidden="true">✓</span>
              <span>{checkinNotice}</span>
            </div>
          )}
          {checkins.length === 0 ? (
            <p className="empty-hint">No check-ins defined — add the habits you want to log below.</p>
          ) : (
            <div className="checkin-grid">
              {checkins.map((c) => {
                const def = checkinDefs.find((d) => d.id === c.id);
                const autoBadge = c.auto ? (
                  <span
                    className="ci-auto"
                    title={
                      (def?.autoKind === "output"
                        ? `Auto: ${c.detected} file(s) detected today`
                        : `Auto: ${c.detected} active min on “${def?.autoMetric ?? ""}” today (needs ${def?.autoThreshold ?? 0}m)`) +
                      (c.overridden ? " — manually overridden" : "")
                    }
                  >
                    ⚡{def?.autoKind === "output" ? c.detected : `${c.detected}m`}
                  </span>
                ) : null;
                const revert = c.overridden ? (
                  <button
                    type="button"
                    className="icon-btn"
                    title="Remove manual override — back to auto detection"
                    onClick={() => revertToAuto(c.id)}
                  >
                    ↺
                  </button>
                ) : null;
                return c.kind === "counter" ? (
                  <div key={c.id} className={`checkin-chip ${c.value > 0 ? "on" : ""}`}>
                    <span className="ci-icon">{c.icon}</span>
                    <span className="ci-label">{c.label}</span>
                    {autoBadge}
                    <span className="ci-stepper">
                      <button type="button" onClick={() => setValue(c.id, c.value - 1)}>
                        −
                      </button>
                      <b>{c.value}</b>
                      <button type="button" onClick={() => setValue(c.id, c.value + 1)}>
                        +
                      </button>
                    </span>
                    {revert}
                    {manageCheckins && (
                      <CheckinManageButtons
                        onEdit={() => editCheckinDef(c.id)}
                        onDelete={() => removeCheckinDef(c)}
                      />
                    )}
                  </div>
                ) : (
                  <div key={c.id} className={`checkin-chip ${c.value > 0 ? "on" : ""}`}>
                    <button
                      type="button"
                      className="checkin-flip"
                      aria-pressed={c.value > 0}
                      onClick={() => setValue(c.id, c.value > 0 ? 0 : 1)}
                    >
                      <span className="ci-icon">{c.icon}</span>
                      <span className="ci-label">{c.label}</span>
                      {autoBadge}
                      <span className="ci-state">{c.value > 0 ? "✓ Logged" : "+ Log"}</span>
                    </button>
                    {revert}
                    {manageCheckins && (
                      <CheckinManageButtons
                        onEdit={() => editCheckinDef(c.id)}
                        onDelete={() => removeCheckinDef(c)}
                      />
                    )}
                  </div>
                );
              })}
            </div>
          )}
          {manageCheckins && (
            <>
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
              <div className="checkin-editor-auto">
                <select
                  className="select"
                  value={checkinDraft.autoKind}
                  onChange={(e) => {
                    const autoKind = e.target.value as CheckinAutoKind;
                    setCheckinDraft({
                      ...checkinDraft,
                      autoKind,
                      autoMetric: "",
                      autoThreshold: autoKind === "target" ? 30 : autoKind === "output" ? 1 : 0,
                    });
                  }}
                >
                  {AUTO_OPTIONS.map((o) => (
                    <option key={o.value} value={o.value}>{o.label}</option>
                  ))}
                </select>
                {checkinDraft.autoKind === "target" && (
                  <input
                    className="search"
                    placeholder="app/site to watch, e.g. DaVinci Resolve or youtube.com"
                    value={checkinDraft.autoMetric}
                    onChange={(e) => setCheckinDraft({ ...checkinDraft, autoMetric: e.target.value })}
                  />
                )}
                {checkinDraft.autoKind === "output" && (
                  <input
                    className="search"
                    placeholder="output type or folder label (blank = any), e.g. video_export"
                    value={checkinDraft.autoMetric}
                    onChange={(e) => setCheckinDraft({ ...checkinDraft, autoMetric: e.target.value })}
                  />
                )}
                {checkinDraft.autoKind !== "" && (
                  <label className="folder-num">
                    <input
                      className="pf-input"
                      type="number"
                      min={1}
                      value={String(checkinDraft.autoThreshold || "")}
                      onChange={(e) =>
                        setCheckinDraft({
                          ...checkinDraft,
                          autoThreshold: Math.max(0, parseInt(e.target.value, 10) || 0),
                        })
                      }
                    />
                    {checkinDraft.autoKind === "target" ? "min" : "files"}
                  </label>
                )}
              </div>
              <p className="card-hint" style={{ marginTop: 8, marginBottom: 0 }}>
                {AUTO_OPTIONS.find((o) => o.value === checkinDraft.autoKind)?.hint}
              </p>
            </>
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
