import { FormEvent, useCallback, useEffect, useState } from "react";
import {
  endFocusSession,
  getFocusSession,
  getFocusSummary,
  startFocusSession,
} from "../api";
import { previewToast } from "../components/AccountabilityLayer";
import { formatDuration } from "../format";
import type { FocusSession, FocusSummary } from "../types";

const DURATIONS = [25, 50, 90];

function splitList(s: string): string[] {
  return s
    .split(/[\n,]+/)
    .map((x) => x.trim())
    .filter(Boolean);
}

function clock(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export default function Focus() {
  const [session, setSession] = useState<FocusSession | null>(null);
  const [summary, setSummary] = useState<FocusSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());

  const [goal, setGoal] = useState("");
  const [duration, setDuration] = useState(50);
  const [blocked, setBlocked] = useState("instagram.com, youtube.com, tiktok.com");
  const [allowed, setAllowed] = useState("");

  const load = useCallback(async () => {
    try {
      setSession(await getFocusSession());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // 1s tick for the countdown; also refetch the session each tick it might end.
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);

  async function start(e: FormEvent) {
    e.preventDefault();
    try {
      const s = await startFocusSession(goal, duration, splitList(allowed), splitList(blocked));
      setSummary(null);
      setSession(s);
    } catch (e) {
      setError(String(e));
    }
  }

  async function stop() {
    if (!session) return;
    const id = session.id;
    setSession(null); // transition immediately + guard against re-entry
    try {
      await endFocusSession();
      setSummary(await getFocusSummary(id));
    } catch (e) {
      setError(String(e));
    }
  }

  // Auto-finish into the summary when the countdown runs out.
  useEffect(() => {
    if (session && new Date(session.endsAt).getTime() <= now) {
      stop();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [now, session]);

  const remaining = session
    ? Math.max(0, Math.round((new Date(session.endsAt).getTime() - now) / 1000))
    : 0;
  const elapsedPct = session
    ? Math.min(100, 100 - (remaining / (session.durationMinutes * 60)) * 100)
    : 0;

  return (
    <>
      <div className="page-head">
        <div>
          <h1 className="page-title">Focus Mode</h1>
          <div className="page-subtitle">
            Pick a goal, set a timer, and get nudged when you drift off-task
          </div>
        </div>
      </div>

      {error && <div className="error-box" style={{ marginBottom: 14 }}>{error}</div>}

      {session ? (
        <div className="card card-pad focus-active">
          <div className="focus-timer-wrap">
            <div
              className="focus-ring"
              style={{ ["--pct" as string]: elapsedPct } as React.CSSProperties}
            >
              <div className="focus-ring-inner">
                <div className="focus-clock">{clock(remaining)}</div>
                <div className="focus-clock-sub">{remaining > 0 ? "remaining" : "time's up"}</div>
              </div>
            </div>
            <div className="focus-active-body">
              <div className="focus-live">
                <span className="live-dot">
                  <span className="pulse" />
                  Focusing
                </span>
              </div>
              {session.goal && <h2 className="focus-goal">{session.goal}</h2>}
              <div className="focus-lists">
                {session.blocked.length > 0 && (
                  <div>
                    <span className="focus-list-label">Blocked</span>
                    <div className="chip-row">
                      {session.blocked.map((b) => (
                        <span key={b} className="block-chip">🚫 {b}</span>
                      ))}
                    </div>
                  </div>
                )}
                {session.allowed.length > 0 && (
                  <div>
                    <span className="focus-list-label">Allowed</span>
                    <div className="chip-row">
                      {session.allowed.map((a) => (
                        <span key={a} className="allow-chip">✓ {a}</span>
                      ))}
                    </div>
                  </div>
                )}
              </div>
              <button className="btn btn-danger" onClick={stop}>
                End session
              </button>
            </div>
          </div>
        </div>
      ) : (
        <form className="card card-pad" onSubmit={start}>
          <h2 className="card-title">Start a focus session</h2>
          <div className="focus-form">
            <label className="pf-field">
              <span>Goal</span>
              <input
                className="pf-input"
                placeholder="e.g. Finish the editor feature"
                value={goal}
                onChange={(e) => setGoal(e.target.value)}
              />
            </label>

            <div className="pf-field">
              <span>Duration</span>
              <div className="dur-row">
                {DURATIONS.map((d) => (
                  <button
                    type="button"
                    key={d}
                    className={`dur-chip ${duration === d ? "on" : ""}`}
                    onClick={() => setDuration(d)}
                  >
                    {d}m
                  </button>
                ))}
                <input
                  className="pf-input dur-input"
                  type="number"
                  min={1}
                  max={600}
                  value={duration}
                  onChange={(e) => setDuration(Math.max(1, Math.min(600, Number(e.target.value) || 1)))}
                />
              </div>
            </div>

            <label className="pf-field">
              <span>
                Blocked apps / sites <em>— you'll get nudged if you open these</em>
              </span>
              <input
                className="pf-input"
                placeholder="instagram.com, youtube.com, tiktok.com"
                value={blocked}
                onChange={(e) => setBlocked(e.target.value)}
              />
            </label>

            <label className="pf-field">
              <span>
                Allowed apps / sites <em>— optional; anything else distracting counts as drift</em>
              </span>
              <input
                className="pf-input"
                placeholder="code.exe, github.com, localhost"
                value={allowed}
                onChange={(e) => setAllowed(e.target.value)}
              />
            </label>

            <div className="focus-form-actions">
              <button className="btn btn-primary" type="submit">
                Start focusing
              </button>
              <button
                type="button"
                className="btn"
                onClick={() =>
                  previewToast(
                    "focus",
                    "Focus mode",
                    "instagram.com isn't part of your focus session. Back to it?",
                  )
                }
              >
                Preview a nudge
              </button>
            </div>
          </div>
        </form>
      )}

      {summary && <SummaryCard summary={summary} />}

      <p className="muted-num focus-note">
        Focus mode is a <b>soft</b> guard — it nudges you, it doesn't block apps at the OS level.
        Detection runs locally; nothing leaves this device.
      </p>
    </>
  );
}

function SummaryCard({ summary }: { summary: FocusSummary }) {
  const total = summary.focusedSeconds + summary.distractedSeconds + summary.otherSeconds;
  const seg = (n: number) => (total > 0 ? (n / total) * 100 : 0);
  return (
    <div className="card card-pad section-gap">
      <h2 className="card-title">Session summary</h2>
      <div className="focus-adherence">
        <span className="focus-adherence-num">{summary.adherence}%</span> on task
        {summary.goal && <span className="focus-summary-goal"> · {summary.goal}</span>}
      </div>
      <div className="stacked-bar" style={{ marginTop: 12 }}>
        <span className="seg" style={{ width: `${seg(summary.focusedSeconds)}%`, background: "var(--good)" }} />
        <span className="seg" style={{ width: `${seg(summary.distractedSeconds)}%`, background: "var(--bad)" }} />
        <span className="seg" style={{ width: `${seg(summary.otherSeconds)}%`, background: "#cbd2dd" }} />
      </div>
      <div className="legend" style={{ marginTop: 10 }}>
        <div className="legend-item">
          <span className="legend-dot" style={{ background: "var(--good)" }} />
          <span>Focused</span>
          <span className="legend-val">{formatDuration(summary.focusedSeconds)}</span>
        </div>
        <div className="legend-item">
          <span className="legend-dot" style={{ background: "var(--bad)" }} />
          <span>Distracted</span>
          <span className="legend-val">{formatDuration(summary.distractedSeconds)}</span>
        </div>
        <div className="legend-item">
          <span className="legend-dot" style={{ background: "#cbd2dd" }} />
          <span>Other</span>
          <span className="legend-val">{formatDuration(summary.otherSeconds)}</span>
        </div>
      </div>
      {summary.topDistraction && (
        <p className="muted-num" style={{ marginTop: 10 }}>
          Biggest pull away: <b>{summary.topDistraction}</b>
        </p>
      )}
    </div>
  );
}
