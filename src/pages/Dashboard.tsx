import { useCallback, useEffect, useRef, useState } from "react";
import {
  getDeviceBreakdown,
  getGoals,
  getLockinPlan,
  getOutputEvents,
  getStreaks,
  getTodaySummary,
  insertSampleData,
  onOutputsUpdated,
  onTrackingUpdated,
  toggleGoal,
} from "../api";
import { BUCKET_META, BUCKET_LIST, categoryMeta } from "../categories";
import { isoOffset } from "../components/LockinPlan";
import { AppGlyph, BarRow, StackedBar, StatCard } from "../components/ui";
import type { Page } from "../components/Sidebar";
import { formatDuration, formatLongDate, percent } from "../format";
import type { Bucket, DeviceUsage, Goal, LockinPlan, OutputEvent, Streak, TodaySummary } from "../types";
import { outputMeta } from "./OutputEvents";
import { streakIcon } from "./Streaks";

function localTodayIso(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export default function Dashboard({ onNavigate }: { onNavigate: (p: Page) => void }) {
  const [summary, setSummary] = useState<TodaySummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [seeding, setSeeding] = useState(false);
  const firstLoad = useRef(true);

  const load = useCallback(async () => {
    try {
      const data = await getTodaySummary();
      setSummary(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      firstLoad.current = false;
    }
  }, []);

  useEffect(() => {
    load();
    // Refresh when the tracker records a new sample, plus a slow safety poll.
    const unlistenPromise = onTrackingUpdated(load);
    const timer = window.setInterval(load, 15000);
    return () => {
      window.clearInterval(timer);
      unlistenPromise.then((un) => un());
    };
  }, [load]);

  async function seed() {
    setSeeding(true);
    try {
      await insertSampleData();
      await load();
    } finally {
      setSeeding(false);
    }
  }

  if (!summary && firstLoad.current) {
    return <div className="loading">Loading today's activity…</div>;
  }

  if (error && !summary) {
    return (
      <>
        <PageHead />
        <div className="error-box">Couldn't load activity: {error}</div>
      </>
    );
  }

  const s = summary as TodaySummary;
  const bucketSeconds = (b: Bucket) =>
    s.perBucket.find((x) => x.bucket === b)?.seconds ?? 0;
  const productiveSeconds = bucketSeconds("productive");
  const productivePct = percent(productiveSeconds, s.totalActiveSeconds);
  const maxApp = s.perApp[0]?.seconds ?? 0;
  const maxCat = s.perCategory[0]?.seconds ?? 0;

  const isEmpty = s.totalActiveSeconds === 0;

  return (
    <>
      <PageHead date={s.date} />

      <MissionsCard onNavigate={onNavigate} />

      <TodaysPlanCard onNavigate={onNavigate} />

      <StreaksStrip onNavigate={onNavigate} />

      <OutputsCard onNavigate={onNavigate} />

      <DevicesCard />

      {isEmpty ? (
        <div className="card card-pad">
          <div className="empty">
            <div className="empty-glyph">🌱</div>
            <h3>No activity tracked yet</h3>
            <p>
              The tracker records the active window every 10 seconds while the app
              is running. Come back in a moment — or load some sample data to see
              how the dashboard looks.
            </p>
            <button className="btn btn-primary" onClick={seed} disabled={seeding} style={{ marginTop: 8 }}>
              {seeding ? "Loading…" : "Load sample data"}
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="stat-grid">
            <StatCard
              label="Total tracked"
              value={formatDuration(s.totalActiveSeconds)}
              foot={`${formatDuration(s.totalIdleSeconds)} idle`}
            />
            <StatCard
              chip={BUCKET_META.productive.color}
              label="Productive"
              value={`${productivePct}%`}
              foot={formatDuration(productiveSeconds)}
            />
            <StatCard
              label="Apps used"
              value={String(s.perApp.length)}
              foot="active today"
            />
            <StatCard
              label="Idle"
              value={formatDuration(s.totalIdleSeconds)}
              foot="away from keyboard"
            />
          </div>

          {/* Productive / neutral / distracting split */}
          <div className="card card-pad section-gap">
            <h2 className="card-title">Focus balance</h2>
            <p className="card-hint">How your active time splits across the three buckets.</p>
            <StackedBar
              total={s.totalActiveSeconds}
              segments={BUCKET_LIST.map((b) => ({
                color: BUCKET_META[b].color,
                value: bucketSeconds(b),
              }))}
            />
            <div className="legend">
              {BUCKET_LIST.map((b) => (
                <div className="legend-item" key={b}>
                  <span className="legend-dot" style={{ background: BUCKET_META[b].color }} />
                  <span>{BUCKET_META[b].label}</span>
                  <span className="legend-val">
                    {formatDuration(bucketSeconds(b))} · {percent(bucketSeconds(b), s.totalActiveSeconds)}%
                  </span>
                </div>
              ))}
            </div>
          </div>

          <div className="two-col">
            {/* Time per app */}
            <div className="card card-pad section-gap">
              <h2 className="card-title">Time per app</h2>
              <p className="card-hint">Bars are colored by each app's category.</p>
              <div className="bars">
                {s.perApp.map((app) => {
                  const meta = categoryMeta(app.category);
                  return (
                    <BarRow
                      key={app.appName}
                      color={meta.color}
                      value={app.seconds}
                      max={maxApp}
                      left={
                        <>
                          <AppGlyph name={app.appName} size={24} />
                          <span className="label">{app.appName}</span>
                        </>
                      }
                    />
                  );
                })}
              </div>
            </div>

            {/* Time per category */}
            <div className="card card-pad section-gap">
              <h2 className="card-title">Time per category</h2>
              <p className="card-hint">Active time grouped by the categories you assigned.</p>
              <div className="bars">
                {s.perCategory.map((c) => {
                  const meta = categoryMeta(c.category);
                  return (
                    <BarRow
                      key={c.category}
                      color={meta.color}
                      value={c.seconds}
                      max={maxCat}
                      left={
                        <>
                          <span className="dot" style={{ background: meta.color }} />
                          <span className="label">{meta.label}</span>
                        </>
                      }
                    />
                  );
                })}
              </div>
            </div>
          </div>

          {s.perWebsite.length > 0 && (
            <div className="card card-pad section-gap">
              <h2 className="card-title">Top websites</h2>
              <p className="card-hint">
                Browser time today ({formatDuration(s.totalBrowserSeconds)}), merged into the totals
                above. Open Browser Activity for page-level detail.
              </p>
              <div className="bars">
                {s.perWebsite.slice(0, 8).map((w) => {
                  const meta = categoryMeta(w.category);
                  return (
                    <BarRow
                      key={w.domain}
                      color={meta.color}
                      value={w.seconds}
                      max={s.perWebsite[0].seconds}
                      left={
                        <>
                          <AppGlyph name={w.domain} size={24} />
                          <span className="label">{w.domain}</span>
                        </>
                      }
                    />
                  );
                })}
              </div>
            </div>
          )}
        </>
      )}
    </>
  );
}

function MissionsCard({ onNavigate }: { onNavigate: (p: Page) => void }) {
  const [goals, setGoals] = useState<Goal[]>([]);

  const load = useCallback(async () => {
    try {
      setGoals(await getGoals());
    } catch {
      /* ignore — missions are non-critical to the dashboard */
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  async function toggle(g: Goal) {
    await toggleGoal(g.id, !g.completed);
    load();
  }

  const done = goals.filter((g) => g.completed).length;

  return (
    <div className="card card-pad section-gap missions-card">
      <div className="missions-head">
        <h2 className="card-title">🎯 Main missions today</h2>
        <button className="link-btn" onClick={() => onNavigate("goals")}>
          {goals.length ? `${done}/${goals.length} done` : "Set goals"} →
        </button>
      </div>
      {goals.length === 0 ? (
        <p className="empty-hint" style={{ margin: 0 }}>
          No missions set yet.{" "}
          <button className="link-inline" onClick={() => onNavigate("goals")}>
            Add your missions
          </button>{" "}
          to start the day with intent.
        </p>
      ) : (
        <ul className="mission-mini-list">
          {goals.map((g) => (
            <li key={g.id} className={`mission-mini ${g.completed ? "done" : ""}`}>
              <button
                className="goal-check sm"
                onClick={() => toggle(g)}
                aria-label={g.completed ? "Mark not done" : "Mark done"}
              >
                {g.completed ? "✓" : ""}
              </button>
              <span className="mm-title">{g.title}</span>
              {g.targetMinutes != null && <span className="mm-target">{g.targetMinutes}m</span>}
              <span className={`prio prio-${g.priority}`}>{g.priority}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function TodaysPlanCard({ onNavigate }: { onNavigate: (p: Page) => void }) {
  const [plan, setPlan] = useState<LockinPlan | null>(null);

  useEffect(() => {
    // The plan generated yesterday is the plan FOR today.
    getLockinPlan(isoOffset(-1)).then(setPlan).catch(() => {});
  }, []);

  if (!plan) return null;

  return (
    <div className="card card-pad section-gap todays-plan">
      <div className="missions-head">
        <h2 className="card-title">🌙 Today's lock-in plan</h2>
        <button className="link-btn" onClick={() => onNavigate("review")}>Full plan →</button>
      </div>
      <div className="lockin-main">
        <span className="lockin-tag main">Main</span>
        <span>{plan.mainMission}</span>
      </div>
      {plan.firstBlock && <div className="muted-num" style={{ marginTop: 6 }}>⏰ {plan.firstBlock}</div>}
    </div>
  );
}

function StreaksStrip({ onNavigate }: { onNavigate: (p: Page) => void }) {
  const [streaks, setStreaks] = useState<Streak[]>([]);

  useEffect(() => {
    getStreaks().then(setStreaks).catch(() => {});
  }, []);

  const active = streaks.filter((s) => s.current > 0).sort((a, b) => b.current - a.current).slice(0, 6);
  if (active.length === 0) return null;

  return (
    <div className="card card-pad section-gap">
      <div className="missions-head">
        <h2 className="card-title">🔥 Active streaks</h2>
        <button className="link-btn" onClick={() => onNavigate("streaks")}>View all →</button>
      </div>
      <div className="streak-chips">
        {active.map((s) => (
          <span key={s.id} className="streak-chip">
            <span>{streakIcon(s.id)}</span> {s.name} <b>🔥{s.current}</b>
          </span>
        ))}
      </div>
    </div>
  );
}

function OutputsCard({ onNavigate }: { onNavigate: (p: Page) => void }) {
  const [events, setEvents] = useState<OutputEvent[]>([]);

  const load = useCallback(async () => {
    try {
      setEvents(await getOutputEvents(localTodayIso()));
    } catch {
      /* outputs are non-critical to the dashboard */
    }
  }, []);

  useEffect(() => {
    load();
    let un: (() => void) | undefined;
    onOutputsUpdated(load).then((u) => (un = u));
    return () => un?.();
  }, [load]);

  if (events.length === 0) return null;

  const counts = new Map<string, number>();
  for (const e of events) counts.set(e.eventType, (counts.get(e.eventType) ?? 0) + 1);

  return (
    <div className="card card-pad section-gap">
      <div className="missions-head">
        <h2 className="card-title">📦 Outputs today</h2>
        <button className="link-btn" onClick={() => onNavigate("outputs")}>
          {events.length} detected →
        </button>
      </div>
      <div className="outputs-strip">
        {[...counts].map(([t, n]) => {
          const m = outputMeta(t);
          return (
            <span key={t} className="output-tally" style={{ borderColor: m.color }}>
              <span>{m.icon}</span> {n} {m.label}
              {n > 1 ? "s" : ""}
            </span>
          );
        })}
      </div>
    </div>
  );
}

const PLATFORM_ICON: Record<string, string> = {
  android: "📱",
  ios: "📱",
  windows: "🖥️",
  macos: "🖥️",
  linux: "🖥️",
};

/**
 * Per-device active time today (desktop vs phone …). Hub-only: on the single-device
 * desktop app `getDeviceBreakdown` returns nothing, so this renders null there.
 */
function DevicesCard() {
  const [devices, setDevices] = useState<DeviceUsage[]>([]);

  const load = useCallback(async () => {
    try {
      setDevices(await getDeviceBreakdown(localTodayIso()));
    } catch {
      /* device breakdown is non-critical to the dashboard */
    }
  }, []);

  useEffect(() => {
    load();
    const timer = window.setInterval(load, 30000);
    return () => window.clearInterval(timer);
  }, [load]);

  // Only meaningful once more than one device reports (e.g. desktop + phone via the hub).
  if (devices.length < 2) return null;

  const max = devices[0]?.activeSeconds ?? 0;
  return (
    <div className="card card-pad section-gap">
      <h2 className="card-title">🖥️ 📱 Devices today</h2>
      <p className="card-hint">
        Active time per device — idle time is excluded, so an always-on but unused machine
        doesn't inflate its share.
      </p>
      <div className="bars">
        {devices.map((d) => (
          <BarRow
            key={d.deviceId}
            color="#6C5CE7"
            value={d.activeSeconds}
            max={max}
            left={
              <>
                <span style={{ fontSize: 18 }}>
                  {PLATFORM_ICON[(d.platform || "").toLowerCase()] ?? "💻"}
                </span>
                <span className="label">
                  {d.name}
                  {d.topLabel ? <span style={{ opacity: 0.55 }}> · {d.topLabel}</span> : null}
                </span>
              </>
            }
          />
        ))}
      </div>
    </div>
  );
}

function PageHead({ date }: { date?: string }) {
  return (
    <div className="page-head">
      <div>
        <h1 className="page-title">Today</h1>
        <div className="page-subtitle">{date ? formatLongDate(date) : "Your activity at a glance"}</div>
      </div>
      <div className="head-actions">
        <span className="live-dot">
          <span className="pulse" />
          Tracking every 10s
        </span>
      </div>
    </div>
  );
}
