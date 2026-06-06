import { useCallback, useEffect, useState } from "react";
import {
  copyLockinPlanToGoals,
  generateLockinPlan,
  getLockinAuto,
  getLockinPlan,
  saveLockinPlan,
  setLockinAuto,
} from "../api";
import type { LockinPlan } from "../types";

export function isoOffset(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

const SOURCE_LABEL: Record<string, string> = {
  llm: "🤖 AI",
  fallback: "rule-based",
  manual: "✎ edited",
};

/** Full "Tomorrow's Lock-In Plan" card. `day` is the source day (defaults today). */
export function LockinPlanCard({ day, onCopied }: { day?: string; onCopied?: () => void }) {
  const sourceDay = day ?? isoOffset(0);
  const [plan, setPlan] = useState<LockinPlan | null>(null);
  const [auto, setAuto] = useState(true);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<LockinPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);

  const load = useCallback(async () => {
    try {
      const [p, a] = await Promise.all([getLockinPlan(sourceDay), getLockinAuto()]);
      setAuto(a);
      if (p) setPlan(p);
      else if (a) setPlan(await generateLockinPlan(sourceDay));
      else setPlan(null);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoaded(true);
    }
  }, [sourceDay]);

  useEffect(() => {
    load();
  }, [load]);

  async function regenerate() {
    setBusy(true);
    try {
      setPlan(await generateLockinPlan(sourceDay));
      setCopied(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function copy() {
    try {
      setCopied(await copyLockinPlanToGoals(sourceDay));
      onCopied?.();
    } catch (e) {
      setError(String(e));
    }
  }

  async function toggleAuto() {
    const v = !auto;
    setAuto(v);
    await setLockinAuto(v);
    if (v && !plan) regenerate();
  }

  async function saveEdit() {
    if (!draft) return;
    setBusy(true);
    try {
      await saveLockinPlan(sourceDay, draft);
      setPlan({ ...draft, source: "manual", edited: true });
      setEditing(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const upd = (patch: Partial<LockinPlan>) => setDraft((d) => (d ? { ...d, ...patch } : d));

  return (
    <div className="card card-pad section-gap lockin-card">
      <div className="missions-head">
        <h2 className="card-title">🌙 Tomorrow's Lock-In Plan</h2>
        <div className="lockin-head-actions">
          {plan && <span className="src-chip rule">{SOURCE_LABEL[plan.source] ?? plan.source}</span>}
          <label className="lockin-auto" title="Auto-generate this plan each day">
            <input type="checkbox" checked={auto} onChange={toggleAuto} /> auto
          </label>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 10 }}>{error}</div>}

      {!loaded ? (
        <div className="loading">Building tomorrow's plan…</div>
      ) : !plan ? (
        <div className="empty-hint">
          No plan yet.{" "}
          <button className="link-inline" onClick={regenerate}>Generate one</button> from today's results.
        </div>
      ) : editing && draft ? (
        <div className="lockin-edit">
          <Field label="Main mission">
            <input className="pf-input" value={draft.mainMission} onChange={(e) => upd({ mainMission: e.target.value })} />
          </Field>
          <Field label="Secondary missions (one per line, max 2)">
            <textarea
              className="pf-input"
              rows={2}
              value={draft.secondaryMissions.join("\n")}
              onChange={(e) => upd({ secondaryMissions: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean).slice(0, 2) })}
            />
          </Field>
          <Field label="First block">
            <input className="pf-input" value={draft.firstBlock} onChange={(e) => upd({ firstBlock: e.target.value })} />
          </Field>
          <Field label="Distraction rule">
            <input className="pf-input" value={draft.distractionRule} onChange={(e) => upd({ distractionRule: e.target.value })} />
          </Field>
          <Field label="Focus mode">
            <input className="pf-input" value={draft.focusMode} onChange={(e) => upd({ focusMode: e.target.value })} />
          </Field>
          <Field label="Avoid this trap">
            <input className="pf-input" value={draft.avoidTrap} onChange={(e) => upd({ avoidTrap: e.target.value })} />
          </Field>
          <Field label="Roast line">
            <input className="pf-input" value={draft.roastLine} onChange={(e) => upd({ roastLine: e.target.value })} />
          </Field>
          <div className="lockin-actions">
            <button className="btn btn-primary" onClick={saveEdit} disabled={busy}>Save plan</button>
            <button className="btn" onClick={() => setEditing(false)}>Cancel</button>
          </div>
        </div>
      ) : (
        <>
          <div className="lockin-main">
            <span className="lockin-tag main">Main</span>
            <span>{plan.mainMission}</span>
          </div>
          {plan.secondaryMissions.length > 0 && (
            <ul className="lockin-secondary">
              {plan.secondaryMissions.map((s, i) => (
                <li key={i}>
                  <span className="lockin-tag">Then</span> {s}
                </li>
              ))}
            </ul>
          )}
          <div className="lockin-grid">
            <Tile icon="⏰" title="Start with" body={plan.firstBlock} />
            <Tile icon="🚫" title="Distraction rule" body={plan.distractionRule} />
            <Tile icon="🧘" title="Focus mode" body={plan.focusMode} />
            <Tile icon="⚠️" title="Avoid this trap" body={plan.avoidTrap} />
          </div>
          {plan.roastLine && <div className="lockin-roast">🔥 {plan.roastLine}</div>}
          <div className="lockin-actions">
            <button className="btn btn-primary" onClick={copy}>Send to tomorrow's goals</button>
            <button className="btn" onClick={() => { setDraft(plan); setEditing(true); }}>Edit</button>
            <button className="btn" onClick={regenerate} disabled={busy}>{busy ? "…" : "Regenerate"}</button>
            {copied !== null && (
              <span className="lockin-copied">{copied > 0 ? `Added ${copied} goal${copied > 1 ? "s" : ""} ✓` : "Already in your goals"}</span>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="lockin-field">
      <span className="lockin-field-label">{label}</span>
      {children}
    </label>
  );
}

function Tile({ icon, title, body }: { icon: string; title: string; body: string }) {
  if (!body) return null;
  return (
    <div className="lockin-tile">
      <div className="lockin-tile-head">
        {icon} {title}
      </div>
      <div className="lockin-tile-body">{body}</div>
    </div>
  );
}
