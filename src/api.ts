import { invoke } from "@tauri-apps/api/core";
import { categoryMeta, DEFAULT_CATEGORY_DEFINITIONS, registerCategoryDefinitions } from "./categories";
import type {
  AccountabilityExport,
  AccountabilityExportOptions,
  ActivityDetail,
  ActivityLogEntry,
  BrowserActivityView,
  CaptureMode,
  Category,
  CategoryDefinition,
  ClassificationPolicy,
  AccountabilitySettings,
  CategoryRule,
  CheckinDefinition,
  CheckinValue,
  CorrectionHistoryEntry,
  DailyAiReview,
  DatabaseBackup,
  DeviceUsage,
  DistractionWarning,
  DomainRule,
  FocusSession,
  FocusSummary,
  FocusViolation,
  Goal,
  GoalDraft,
  LlmErrorEntry,
  LlmSettings,
  LockinPlan,
  OllamaTestResult,
  OutputEvent,
  PrivacySettings,
  Project,
  ProjectMatchTest,
  WatchedFolder,
  ScoreLine,
  ScoreReport,
  ScoreRule,
  Streak,
  SyncStatus,
  StreakDefinition,
  TimelineBlock,
  TimeBreakdown,
  TimelineDay,
  TodaySummary,
  TrackingHealth,
  TrackedApp,
  TrackedDomain,
  WebsiteUsage,
  WeeklyDay,
  WeeklyReview,
} from "./types";

/**
 * The whole app talks to the Rust backend through this module.
 *
 * When the bundle is opened inside Tauri we call real `#[tauri::command]`s.
 * When it is opened in a plain browser (e.g. `npm run dev` for UI work) we
 * serve in-memory mock data instead, so the UI is fully explorable without a
 * compiled backend. Nothing here ever touches the network.
 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Remote mode: a production build served by the Tempo Hub (not Tauri, not the
 * vite dev server). In this mode read/write commands go to the hub's
 * `/api/invoke` REST endpoint instead of mocks, so the phone/browser see the
 * same shared dashboard. `npm run dev` stays on mocks.
 */
export function isRemote(): boolean {
  const env = (import.meta as unknown as { env?: { PROD?: boolean } }).env;
  return !isTauri() && env?.PROD === true;
}

function webToken(): string {
  let t = (typeof localStorage !== "undefined" && localStorage.getItem("tempo_web_token")) || "";
  if (!t && typeof window !== "undefined") {
    const hash = new URLSearchParams(window.location.hash.slice(1));
    const seeded = hash.get("tempo_token") || "";
    if (seeded) {
      t = seeded;
      localStorage.setItem("tempo_web_token", t);
      hash.delete("tempo_token");
      const rest = hash.toString();
      window.history.replaceState(
        null,
        "",
        `${window.location.pathname}${window.location.search}${rest ? `#${rest}` : ""}`,
      );
    }
  }
  if (!t && typeof window !== "undefined") {
    t = window.prompt("Enter the Tempo Hub secret to view the dashboard:") || "";
    if (t) localStorage.setItem("tempo_web_token", t);
  }
  return t;
}

async function remoteInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  let res: Response;
  try {
    res = await fetch("/api/invoke", {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${webToken()}` },
      body: JSON.stringify({ cmd, args }),
    });
  } catch {
    throw new Error(
      "Tempo Hub connection lost. Make sure Tailscale is connected, then reload the app. " +
      "If the screen was already open, it may be showing a cached page even though the Hub is unreachable.",
    );
  }
  if (res.status === 401) {
    if (typeof localStorage !== "undefined") localStorage.removeItem("tempo_web_token");
    throw new Error("Unauthorized — re-enter the hub secret and reload.");
  }
  if (!res.ok) throw new Error(`Hub command "${cmd}" failed (${res.status})`);
  return (await res.json()) as T;
}

const SHARED_HUB_READ_COMMANDS = new Set([
  "get_today_summary",
  "get_time_breakdown",
  "get_timeline_for_day",
  "get_daily_score",
  "get_streaks",
  "get_weekly_review",
  "get_tracked_apps",
  "get_tracked_domains",
  "get_device_breakdown",
]);

type HubReadResult<T> = { active: boolean; value: T | null };

const SHARED_READ_TTL_MS = 10_000;
const sharedReadCache = new Map<string, { storedAt: number; value: unknown }>();
const sharedReadInflight = new Map<string, Promise<unknown>>();

async function uncachedBackend<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return remoteInvoke<T>(cmd, args);
  try {
    const shared = await invoke<HubReadResult<T>>("read_from_hub", { cmd, args });
    if (shared.active) return shared.value as T;
  } catch (error) {
    console.warn(`Tempo Hub read failed for ${cmd}; using this device's local data.`, error);
  }
  return invoke<T>(cmd, args);
}

/**
 * Shared activity reads are cached briefly and identical in-flight requests are
 * deduplicated. Navigating away no longer forces the same Pi/SQLite calculation
 * again, while the ten-second TTL stays aligned with Tempo's tracking cadence.
 */
async function callBackend<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (!SHARED_HUB_READ_COMMANDS.has(cmd)) {
    return isTauri() ? invoke<T>(cmd, args) : remoteInvoke<T>(cmd, args);
  }

  const key = `${cmd}:${JSON.stringify(args)}`;
  const cached = sharedReadCache.get(key);
  if (cached && Date.now() - cached.storedAt < SHARED_READ_TTL_MS) {
    return cached.value as T;
  }
  const pending = sharedReadInflight.get(key);
  if (pending) return pending as Promise<T>;

  const request = uncachedBackend<T>(cmd, args)
    .then((value) => {
      sharedReadCache.set(key, { storedAt: Date.now(), value });
      return value;
    })
    .finally(() => {
      sharedReadInflight.delete(key);
    });
  sharedReadInflight.set(key, request);
  return request;
}

// ---------------------------------------------------------------------------
// Real backend calls
// ---------------------------------------------------------------------------

export async function getTodaySummary(): Promise<TodaySummary> {
  if (isTauri() || isRemote()) return callBackend<TodaySummary>("get_today_summary");
  return mockSummary();
}

const timeBreakdownRequests = new Map<string, Promise<TimeBreakdown>>();

export async function getTimeBreakdown(startDate: string, endDate: string): Promise<TimeBreakdown> {
  const key = `${startDate}:${endDate}`;
  const existing = timeBreakdownRequests.get(key);
  if (existing) return existing;

  const request = (async () => {
    if (isTauri() || isRemote()) {
      return callBackend<TimeBreakdown>("get_time_breakdown", { startDate, endDate });
    }
    const summary = mockSummary();
    const parsedDays = Math.round((Date.parse(endDate) - Date.parse(startDate)) / 86_400_000) + 1;
    return {
      startDate, endDate, dayCount: Math.max(1, parsedDays),
      totalActiveSeconds: summary.totalActiveSeconds,
      totalIdleSeconds: summary.totalIdleSeconds,
      totalBrowserSeconds: summary.totalBrowserSeconds,
      perApp: summary.perApp, perWebsite: summary.perWebsite,
      perCategory: summary.perCategory, perBucket: summary.perBucket,
    };
  })();

  timeBreakdownRequests.set(key, request);
  void request.then(
    () => {
      window.setTimeout(() => {
        if (timeBreakdownRequests.get(key) === request) timeBreakdownRequests.delete(key);
      }, 15_000);
    },
    () => {
      if (timeBreakdownRequests.get(key) === request) timeBreakdownRequests.delete(key);
    },
  );
  return request;
}

export function preloadDefaultTimeBreakdown(): void {
  const end = new Date();
  const start = new Date(end);
  start.setDate(start.getDate() - 6);
  const iso = (date: Date) =>
    `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
  void getTimeBreakdown(iso(start), iso(end)).catch(() => undefined);
}

export async function getTrackedApps(): Promise<TrackedApp[]> {
  if (isTauri() || isRemote()) return callBackend<TrackedApp[]>("get_tracked_apps");
  return mockTrackedApps();
}

export async function getCategoryRules(): Promise<CategoryRule[]> {
  if (isTauri() || isRemote()) return callBackend<CategoryRule[]>("get_category_rules");
  return [...mockCategories]
    .filter(([, c]) => c)
    .map(([appName, category]) => ({ appName, category: category as Category }));
}

export async function getCategoryDefinitions(): Promise<CategoryDefinition[]> {
  if (isTauri() || isRemote()) {
    const defs = await callBackend<CategoryDefinition[]>("get_category_definitions");
    registerCategoryDefinitions(defs);
    return defs;
  }
  registerCategoryDefinitions(mockCategoryDefs);
  return mockCategoryDefs.map((c) => ({ ...c }));
}

export async function getClassificationPolicies(): Promise<ClassificationPolicy[]> {
  if (isTauri() || isRemote()) return callBackend<ClassificationPolicy[]>("get_classification_policies");
  return mockClassificationPolicies.map((policy) => ({ ...policy, kinds: [...policy.kinds], terms: [...policy.terms] }));
}

export async function setClassificationPolicies(policies: ClassificationPolicy[]): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_classification_policies", { policies });
    return;
  }
  mockClassificationPolicies = policies.map((policy) => ({ ...policy, kinds: [...policy.kinds], terms: [...policy.terms] }));
}
export async function upsertCategoryDefinition(category: CategoryDefinition): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("upsert_category_definition", { category });
    return;
  }
  const i = mockCategoryDefs.findIndex((c) => c.id === category.id);
  if (i >= 0) mockCategoryDefs[i] = { ...category, builtIn: mockCategoryDefs[i].builtIn };
  else mockCategoryDefs.push({ ...category, builtIn: false });
  registerCategoryDefinitions(mockCategoryDefs);
}

export async function deleteCategoryDefinition(id: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_category_definition", { id });
    return;
  }
  if (mockCategoryDefs.length <= 1) throw new Error("At least one category must remain");
  mockCategoryDefs = mockCategoryDefs.filter((c) => c.id !== id);
  const fallback = mockCategoryDefs.find((c) => c.bucket === "neutral")?.id ?? mockCategoryDefs[0]?.id ?? "uncategorized";
  for (const [app, category] of mockCategories.entries()) {
    if (category === id) mockCategories.set(app, fallback);
  }
  for (const [domain, rule] of mockDomainRules.entries()) {
    if (rule.category === id) mockDomainRules.set(domain, { ...rule, category: null });
  }
  for (let i = 0; i < mockProjects.length; i += 1) {
    if (mockProjects[i].category === id) mockProjects[i] = { ...mockProjects[i], category: fallback };
  }
  registerCategoryDefinitions(mockCategoryDefs);
}

export async function setCategoryRule(
  appName: string,
  category: Category,
  aiReview = false
): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_category_rule", { appName, category, aiReview });
    return;
  }
  mockCategories.set(appName, category);
  mockAppAi.set(appName, aiReview);
}

export async function deleteCategoryRule(appName: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_category_rule", { appName });
    return;
  }
  mockCategories.set(appName, null);
}

export async function getCorrectionHistory(blockKey: string): Promise<CorrectionHistoryEntry[]> {
  if (isTauri()) return invoke<CorrectionHistoryEntry[]>("get_correction_history", { blockKey });
  return [];
}

export async function undoCorrection(id: number): Promise<void> {
  if (isTauri()) await invoke("undo_correction", { id });
}
export async function correctActivity(
  blockKey: string,
  source: string,
  label: string,
  title: string,
  category: string
): Promise<void> {
  if (isTauri()) {
    await invoke("correct_activity", { blockKey, source, label, title, category });
    return;
  }
  // Preview mode: corrections aren't persisted.
}

export async function getTimelineForDay(
  day: string,
  maxGapSeconds?: number,
): Promise<TimelineDay> {
  if (isTauri() || isRemote()) return callBackend<TimelineDay>("get_timeline_for_day", { day, maxGapSeconds });
  return mockTimeline(day);
}

// ---- proof-of-output detection ----

export async function getOutputEvents(day: string): Promise<OutputEvent[]> {
  if (isTauri() || isRemote()) return callBackend<OutputEvent[]>("get_output_events", { day });
  return mockOutputEvents(day);
}

export async function getWatchedFolders(): Promise<WatchedFolder[]> {
  if (isTauri()) return invoke<WatchedFolder[]>("get_watched_folders");
  return mockFolders.map((f) => ({ ...f }));
}

export async function addWatchedFolder(folder: Partial<WatchedFolder>): Promise<number> {
  if (isTauri()) return invoke<number>("add_watched_folder", { folder });
  const id = mockFolderId++;
  mockFolders.push({
    id,
    path: folder.path ?? "",
    label: folder.label || "Folder",
    project: folder.project ?? null,
    outputType: folder.outputType ?? "other",
    enabled: folder.enabled ?? true,
    extensions: folder.extensions ?? [],
    minSizeBytes: folder.minSizeBytes ?? 0,
    debounceSeconds: folder.debounceSeconds ?? 5,
    createdAt: new Date().toISOString(),
  });
  return id;
}

export async function updateWatchedFolder(folder: WatchedFolder): Promise<void> {
  if (isTauri()) {
    await invoke("update_watched_folder", { folder });
    return;
  }
  const i = mockFolders.findIndex((f) => f.id === folder.id);
  if (i >= 0) mockFolders[i] = { ...folder };
}

export async function removeWatchedFolder(id: number): Promise<void> {
  if (isTauri()) {
    await invoke("remove_watched_folder", { id });
    return;
  }
  mockFolders = mockFolders.filter((f) => f.id !== id);
}

export async function scanOutputsNow(): Promise<number> {
  if (isTauri()) return invoke<number>("scan_outputs_now");
  return 0;
}

export async function linkOutputEventsToBlocks(day: string): Promise<number> {
  if (isTauri()) return invoke<number>("link_output_events_to_blocks", { day });
  return 0;
}

export function onOutputsUpdated(cb: () => void): Promise<() => void> {
  return listenTo<number>("outputs-updated", () => cb());
}

// ---- streaks ----

export async function getStreaks(): Promise<Streak[]> {
  if (isTauri() || isRemote()) return callBackend<Streak[]>("get_streaks");
  return mockStreaks();
}

/**
 * Per-device active time today. A paired desktop reads the Hub's multi-device
 * ledger; local and preview modes return no synthetic device breakdown.
 */
export async function getDeviceBreakdown(day: string): Promise<DeviceUsage[]> {
  if (isTauri() || isRemote()) return callBackend<DeviceUsage[]>("get_device_breakdown", { day });
  return mockDeviceBreakdown();
}

function mockDeviceBreakdown(): DeviceUsage[] {
  return [];
}

export async function getStreakDefinitions(): Promise<StreakDefinition[]> {
  if (isTauri() || isRemote()) return callBackend<StreakDefinition[]>("get_streak_definitions");
  return mockStreakDefs.map((d) => ({ ...d }));
}

export async function updateStreakDefinition(
  id: string,
  patch: { enabled?: boolean; threshold?: number; name?: string; daysPerWeek?: number },
): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("update_streak_definition", {
      id,
      enabled: patch.enabled,
      threshold: patch.threshold,
      name: patch.name,
    });
    return;
  }
  const d = mockStreakDefs.find((x) => x.id === id);
  if (d) {
    if (patch.enabled !== undefined) d.enabled = patch.enabled;
    if (patch.threshold !== undefined) d.threshold = patch.threshold;
    if (patch.name !== undefined) d.name = patch.name;
    if (patch.daysPerWeek !== undefined) d.daysPerWeek = patch.daysPerWeek;
  }
}

export async function addStreakDefinition(def: {
  id: string;
  name: string;
  kind: string;
  metric: string;
  threshold: number;
  daysPerWeek: number;
}): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("add_streak_definition", { ...def });
    return;
  }
  const existing = mockStreakDefs.find((d) => d.id === def.id);
  if (existing) Object.assign(existing, def, { enabled: true });
  else mockStreakDefs.push({ ...def, enabled: true });
}

export async function deleteStreakDefinition(id: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_streak_definition", { id });
    return;
  }
  mockStreakDefs = mockStreakDefs.filter((d) => d.id !== id);
}

/** Add suggestions based on the user's actual goals and used/custom check-ins. */
export async function seedDefaultStreaks(): Promise<number> {
  if (isTauri() || isRemote()) return callBackend<number>("seed_default_streaks");
  const suggested: StreakDefinition[] = [
    { id: "main_goal", name: "Completed main goal", kind: "goal", metric: "main_goal", threshold: 0, enabled: true, daysPerWeek: 0 },
    ...mockCheckinDefs
      .filter((d) => !d.builtIn || (mockCheckinValues.get(d.id) ?? 0) > 0)
      .map((d) => ({
        id: `checkin_${d.id}`,
        name: d.label,
        kind: "checkin",
        metric: d.id,
        threshold: 1,
        enabled: true,
        daysPerWeek: 0,
      })),
  ];
  let added = 0;
  for (const s of suggested) {
    if (!mockStreakDefs.some((d) => d.id === s.id)) {
      mockStreakDefs.push({ ...s });
      added++;
    }
  }
  return added;
}
// ---- daily lock-in plan ----

export async function generateLockinPlan(day: string): Promise<LockinPlan> {
  if (isTauri() || isRemote()) return callBackend<LockinPlan>("generate_lockin_plan", { day });
  return mockGenLockin(day);
}

export async function getLockinPlan(day: string): Promise<LockinPlan | null> {
  if (isTauri() || isRemote()) return callBackend<LockinPlan | null>("get_lockin_plan", { day });
  return mockLockins[day] ?? null;
}

export async function saveLockinPlan(day: string, plan: LockinPlan): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("save_lockin_plan", { day, plan });
    return;
  }
  mockLockins[day] = { ...plan, day, source: "manual", edited: true };
}

export async function copyLockinPlanToGoals(day: string): Promise<number> {
  if (isTauri() || isRemote()) return callBackend<number>("copy_lockin_plan_to_goals", { day });
  const p = mockLockins[day];
  if (!p) return 0;
  let n = 0;
  const addG = (title: string, priority: "high" | "medium") => {
    if (title.trim() && !mockGoals.some((g) => g.title === title)) {
      mockGoals.push({ id: mockGoalId++, title, project: null, targetMinutes: null, targetCount: null, targetUnit: null, priority, completed: false, recurring: false });
      n++;
    }
  };
  addG(p.mainMission, "high");
  p.secondaryMissions.forEach((s) => addG(s, "medium"));
  return n;
}

export async function getLockinAuto(): Promise<boolean> {
  if (isTauri()) return invoke<boolean>("get_lockin_auto");
  return mockLockinAuto;
}

// ---- Tempo Hub sync (desktop controls) ----

export async function getSyncStatus(): Promise<SyncStatus> {
  if (isTauri()) return invoke<SyncStatus>("get_sync_status");
  return { mode: "local", connected: false, lastSync: null, queued: 0, hubUrl: "", paired: false };
}

export async function setAppMode(mode: "local" | "hub"): Promise<void> {
  if (isTauri()) await invoke("set_app_mode", { mode });
}

export async function pairWithHub(hubUrl: string, pairingSecret: string): Promise<void> {
  if (isTauri()) {
    await invoke("pair_with_hub", { hubUrl, pairingSecret });
    return;
  }
  throw new Error("Pairing runs from the desktop app.");
}

export async function importHistoryToHub(): Promise<void> {
  if (isTauri()) await invoke("import_history_to_hub");
}

export async function syncConfigurationNow(): Promise<number> {
  if (isTauri()) return invoke<number>("sync_configuration_now");
  throw new Error("Configuration sync runs from the desktop app.");
}

export function onSyncStatus(cb: () => void): Promise<() => void> {
  return listenTo<unknown>("sync-status", () => cb());
}

export async function setLockinAuto(enabled: boolean): Promise<void> {
  if (isTauri()) {
    await invoke("set_lockin_auto", { enabled });
    return;
  }
  mockLockinAuto = enabled;
}

export async function getDailyReview(): Promise<DailyAiReview> {
  if (isTauri() || isRemote()) return callBackend<DailyAiReview>("get_daily_review");
  return { ...mockReview, date: localDateIso(), notes: mockNotes };
}

export async function generateDailyReview(): Promise<DailyAiReview> {
  if (isTauri() || isRemote()) return callBackend<DailyAiReview>("generate_daily_review");
  // Preview: pretend the local LLM produced a sharper review.
  mockReview = {
    ...mockReview,
    verdict: "Decent day — you shipped, but the afternoon got soft.",
    wins: [
      "2 hours of real coding before lunch.",
      "Main goal actually done, not just 'in progress'.",
      "Posted a video instead of overthinking it.",
    ],
    problems: [
      "Instagram robbed you of 35 minutes.",
      "Recovery time ran long — that's just procrastination with a candle.",
      "Studying stalled at 40 minutes.",
    ],
    tomorrow: "One 90-minute deep-work block before you touch your phone.",
    roast: "You shipped, sure, but Instagram still mugged you like a Maltese parking ticket.",
    source: "llm",
    model: "llama3.1:8b",
    generatedAt: new Date().toISOString(),
    notes: mockNotes,
  };
  return { ...mockReview };
}

export async function setDailyNote(notes: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_daily_note", { notes });
    return;
  }
  mockNotes = notes;
}

/**
 * Subscribe to the backend's "a new sample was recorded" event so the
 * dashboard can refresh live. Returns an unsubscribe function. No-op in the
 * browser preview.
 */
export async function onTrackingUpdated(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const unlisten = await listen("tracking-updated", () => cb());
  return unlisten;
}

// ---------------------------------------------------------------------------
// Accountability: focus mode, distraction warnings, weekly review
// ---------------------------------------------------------------------------

async function listenTo<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => cb(e.payload));
}

export function onDistractionWarning(cb: (w: DistractionWarning) => void) {
  return listenTo<DistractionWarning>("distraction-warning", cb);
}
export function onFocusViolation(cb: (v: FocusViolation) => void) {
  return listenTo<FocusViolation>("focus-violation", cb);
}
export function onDailyReviewDue(cb: () => void) {
  return listenTo<unknown>("daily-review-due", () => cb());
}

/** Show a native desktop notification. Browser and Hub previews remain no-ops. */
export async function showNativeNotification(title: string, body: string): Promise<void> {
  if (isTauri()) await invoke("show_native_notification", { title, body });
}

let mockLaunchAtLogin = true;

/** OS startup state. Hidden on the Hub because it controls only the desktop device. */
export async function getLaunchAtLogin(): Promise<boolean | null> {
  if (isTauri()) return invoke<boolean>("get_launch_at_login");
  if (isRemote()) return null;
  return mockLaunchAtLogin;
}

export async function setLaunchAtLogin(enabled: boolean): Promise<void> {
  if (isTauri()) {
    await invoke("set_launch_at_login", { enabled });
    return;
  }
  if (!isRemote()) mockLaunchAtLogin = enabled;
}

export async function getAccountabilitySettings(): Promise<AccountabilitySettings> {
  if (isTauri() || isRemote()) return callBackend<AccountabilitySettings>("get_accountability_settings");
  return { ...mockAccountability };
}

export async function setAccountabilitySetting(key: string, value: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_accountability_setting", { key, value });
    return;
  }
  if (key === "distraction_warn_enabled") mockAccountability.distractionWarnEnabled = value === "1";
  else if (key === "distraction_warn_minutes")
    mockAccountability.distractionWarnMinutes = Math.max(1, Math.min(240, parseInt(value, 10) || 20));
  else if (key === "eod_popup_enabled") mockAccountability.eodPopupEnabled = value === "1";
  else if (key === "eod_popup_time") mockAccountability.eodPopupTime = value;
  else if (key === "main_goal_deadline") mockAccountability.mainGoalDeadline = value;
}

export async function getFocusSession(): Promise<FocusSession | null> {
  if (isTauri() || isRemote()) return callBackend<FocusSession | null>("get_focus_session");
  // Mirror the backend: only an *active* session is returned.
  return mockFocus && mockFocus.status === "active"
    ? { ...mockFocus, remainingSeconds: mockRemaining() }
    : null;
}

export async function startFocusSession(
  goal: string | null,
  durationMinutes: number,
  allowed: string[],
  blocked: string[],
): Promise<FocusSession> {
  if (isTauri())
    return invoke<FocusSession>("start_focus_session", { goal, durationMinutes, allowed, blocked });
  const now = new Date();
  mockFocus = {
    id: Date.now(),
    goal: goal && goal.trim() ? goal.trim() : null,
    startedAt: now.toISOString(),
    durationMinutes,
    endsAt: new Date(now.getTime() + durationMinutes * 60000).toISOString(),
    allowed,
    blocked,
    status: "active",
    endedAt: null,
    remainingSeconds: durationMinutes * 60,
  };
  return { ...mockFocus };
}

export async function endFocusSession(): Promise<void> {
  if (isTauri()) {
    await invoke("end_focus_session");
    return;
  }
  if (mockFocus) mockFocus = { ...mockFocus, status: "ended", endedAt: new Date().toISOString() };
}

export async function getFocusSummary(id: number): Promise<FocusSummary> {
  if (isTauri() || isRemote()) return callBackend<FocusSummary>("get_focus_summary", { id });
  return {
    goal: mockFocus?.goal ?? "",
    durationMinutes: mockFocus?.durationMinutes ?? 0,
    status: mockFocus?.status ?? "completed",
    focusedSeconds: 0,
    distractedSeconds: 0,
    otherSeconds: 0,
    topDistraction: null,
    adherence: 0,
  };
}

export async function getWeeklyReview(): Promise<WeeklyReview> {
  if (isTauri() || isRemote()) return callBackend<WeeklyReview>("get_weekly_review");
  return mockWeekly();
}

export async function setDistractionSnooze(minutes: number): Promise<void> {
  if (isTauri()) await invoke("set_distraction_snooze", { minutes });
}

export async function setDistractionIntentional(target: string, minutes: number): Promise<void> {
  if (isTauri()) await invoke("set_distraction_intentional", { target, minutes });
}

export async function pruneOldData(): Promise<number> {
  if (isTauri()) return invoke<number>("prune_old_data");
  return 0;
}

export async function listDatabaseBackups(): Promise<DatabaseBackup[]> {
  if (isTauri()) return invoke<DatabaseBackup[]>("list_database_backups");
  return [];
}

export async function createDatabaseBackup(): Promise<DatabaseBackup> {
  if (isTauri()) return invoke<DatabaseBackup>("create_database_backup");
  return { name: "preview-backup.db", createdAt: new Date().toISOString(), bytes: 0, automatic: false };
}

export async function restoreDatabaseBackup(name: string): Promise<void> {
  if (isTauri()) await invoke("restore_database_backup", { name });
}

export async function generateAccountabilityExport(
  options: AccountabilityExportOptions,
): Promise<AccountabilityExport> {
  if (isTauri() || isRemote()) {
    return callBackend<AccountabilityExport>("generate_accountability_export", { options });
  }
  const dayCount = Math.max(1, Math.round((Date.parse(options.endDate) - Date.parse(options.startDate)) / 86400000) + 1);
  const names = options.includeActivityNames ? "DaVinci Resolve and youtube.com" : "Desktop app #1 and Website #1";
  return {
    filename: `tempo-accountability-${options.startDate}_to_${options.endDate}.md`,
    startDate: options.startDate,
    endDate: options.endDate,
    dayCount,
    trackedDays: Math.min(dayCount, 5),
    activeSeconds: 18_900,
    markdown: `# Tempo accountability report\n\n> **Suggested prompt for ChatGPT:** Be my brutally honest but practical accountability coach. Identify where I waste time and give me three changes for the next seven days.\n\n## Scope and privacy\n\n- Date range: **${options.startDate} to ${options.endDate}**\n- Window/page titles: **${options.includeTitles ? "included" : "excluded"}**\n- Raw captured text: **${options.includeRawText ? "included" : "excluded"}**\n\n## Executive snapshot\n\n- Active tracked time: **5h 15m**\n- Productive: **3h 40m**\n- Distracting: **1h 05m**\n\n## Biggest recorded distractions\n\n- ${names}\n`,
  };
}

export async function saveAccountabilityExport(options: AccountabilityExportOptions): Promise<string | null> {
  if (!isTauri()) return null;
  return invoke<string>("save_accountability_export", { options });
}
export async function getTrackingHealth(): Promise<TrackingHealth> {
  if (isTauri()) return invoke<TrackingHealth>("get_tracking_health");
  return {
    status: "healthy",
    checkedAt: new Date().toISOString(),
    databaseOk: true,
    lastDesktopAt: new Date().toISOString(),
    lastBrowserAt: null,
    lastScreenAt: null,
    browserConnected: false,
    smartEnabled: false,
    pendingSyncEvents: 0,
    lastBackupAt: null,
    issues: [],
  };
}
export async function resetDatabase(): Promise<number> {
  if (isTauri()) return invoke<number>("reset_database");
  return 0;
}

// ---------------------------------------------------------------------------
// Browser-preview mock data (not used inside the Tauri app)
// ---------------------------------------------------------------------------

function localDateIso(d = new Date()): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

let mockClassificationPolicies: ClassificationPolicy[] = [
  {
    id: "recognized-gameplay",
    name: "Recognized gameplay",
    category: "distraction",
    kinds: ["game"],
    terms: ["gameplay", "video game", "steam", "minecraft", "roblox", "fortnite", "valorant"],
    enabled: true,
    builtIn: true,
    priority: 100,
  },
  {
    id: "generic-telegram",
    name: "Generic Telegram use",
    category: "distraction",
    kinds: ["chat"],
    terms: ["telegram"],
    enabled: true,
    builtIn: true,
    priority: 80,
  },
];
const mockCategories = new Map<string, Category | null>();
const mockAppAi = new Map<string, boolean>();
const mockSeconds = new Map<string, number>();
let mockCategoryDefs: CategoryDefinition[] = DEFAULT_CATEGORY_DEFINITIONS.map((c) => ({ ...c }));

function mockTrackedApps(): TrackedApp[] {
  return [...mockSeconds]
    .map(([appName, totalSeconds]) => ({
      appName,
      totalSeconds,
      category: mockCategories.get(appName) ?? null,
      aiReview: mockAppAi.get(appName) ?? false,
    }))
    .sort((a, b) => b.totalSeconds - a.totalSeconds);
}

function mockSummary(): TodaySummary {
  const apps = [...mockSeconds].map(([appName, seconds]) => ({
    appName,
    seconds,
    category: mockCategories.get(appName) ?? null,
  }));
  const websites = mockWebsites();

  const perCat = new Map<string, number>();
  const perBucket = new Map<string, number>();
  const add = (cat: string | null, seconds: number) => {
    const key = cat ?? "uncategorized";
    perCat.set(key, (perCat.get(key) ?? 0) + seconds);
    const bucket = categoryMeta(cat).bucket;
    perBucket.set(bucket, (perBucket.get(bucket) ?? 0) + seconds);
  };
  for (const a of apps) add(a.category, a.seconds);
  for (const w of websites) add(w.category, w.seconds);

  const browserSeconds = websites.reduce((s, w) => s + w.seconds, 0);
  const totalActive = apps.reduce((s, a) => s + a.seconds, 0) + browserSeconds;

  return {
    date: localDateIso(),
    totalActiveSeconds: totalActive,
    totalIdleSeconds: 0,
    totalBrowserSeconds: browserSeconds,
    perApp: apps.sort((a, b) => b.seconds - a.seconds),
    perWebsite: websites.sort((a, b) => b.seconds - a.seconds),
    perCategory: [...perCat.entries()]
      .map(([category, seconds]) => ({ category: category as TodaySummary["perCategory"][number]["category"], seconds }))
      .sort((a, b) => b.seconds - a.seconds),
    perBucket: [...perBucket.entries()]
      .map(([bucket, seconds]) => ({ bucket: bucket as TodaySummary["perBucket"][number]["bucket"], seconds }))
      .sort((a, b) => b.seconds - a.seconds),
  };
}

let mockNotes = "";
let mockReview: DailyAiReview = {
  date: localDateIso(),
  verdict: "No activity tracked yet.",
  wins: [],
  problems: [],
  tomorrow: "Add goals or start tracking to generate a review.",
  roast: "",
  source: "fallback",
  model: null,
  generatedAt: null,
  notes: "",
};

// ---------------------------------------------------------------------------
// Browser tracking commands
// ---------------------------------------------------------------------------

export async function getBrowserActivity(): Promise<BrowserActivityView> {
  if (isTauri()) return invoke<BrowserActivityView>("get_browser_activity");
  return mockBrowserActivity();
}

export async function getActivityDetails(id: number): Promise<ActivityDetail> {
  if (isTauri()) return invoke<ActivityDetail>("get_activity_details", { id });
  return mockActivityDetail(id);
}

export async function getDomainRules(): Promise<DomainRule[]> {
  if (isTauri() || isRemote()) return callBackend<DomainRule[]>("get_domain_rules");
  return [...mockDomainRules.values()];
}

export async function getTrackedDomains(): Promise<TrackedDomain[]> {
  if (isTauri() || isRemote()) return callBackend<TrackedDomain[]>("get_tracked_domains");
  return mockTrackedDomains();
}

export async function setDomainRule(
  domain: string,
  category: Category | null,
  captureMode: CaptureMode,
  aiReview = false
): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_domain_rule", { domain, category, captureMode, aiReview });
    return;
  }
  mockDomainRules.set(domain, { domain, category, captureMode, aiReview });
}

export async function deleteDomainRule(domain: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_domain_rule", { domain });
    return;
  }
  mockDomainRules.delete(domain);
}

export async function getPrivacySettings(): Promise<PrivacySettings> {
  if (isTauri()) return invoke<PrivacySettings>("get_privacy_settings");
  return { ...mockPrivacy };
}

export async function setTrackingPause(minutes: number): Promise<string | null> {
  if (isTauri()) return invoke<string | null>("set_tracking_pause", { minutes });
  mockPrivacy = { ...mockPrivacy, trackingPausedUntil: minutes > 0 ? new Date(Date.now() + minutes * 60_000).toISOString() : null };
  return mockPrivacy.trackingPausedUntil;
}
export async function setPrivacySetting(key: string, value: string): Promise<void> {
  if (isTauri()) {
    await invoke("set_privacy_setting", { key, value });
    return;
  }
  applyMockPrivacy(key, value);
}

export async function purgeRawContent(): Promise<number> {
  if (isTauri()) return invoke<number>("purge_raw_content");
  return 0;
}

export async function deleteAllCapturedContent(): Promise<number> {
  if (isTauri()) return invoke<number>("delete_all_captured_content");
  return 0;
}

// ---------------------------------------------------------------------------
// Browser-preview mock data
// ---------------------------------------------------------------------------

const mockDomainRules = new Map<string, DomainRule>();

let mockPrivacy: PrivacySettings = {
  capturePageContent: false,
  storeRawText: false,
  maxTextLength: 8000,
  deleteRawAfterClassification: false,
  smartTrackingEnabled: false,
  smartIntervalSeconds: 60,
  smartOcrAvailable: true,
  ingestPort: 48710,
  ingestToken: "demo-token-preview-only",
  endpoint: "http://127.0.0.1:48710",
  retentionDays: 90,
  idleThresholdSeconds: 60,
  countMediaAsActive: true,
  trackingPausedUntil: null,
  titleExcludedApps: [],
};

function applyMockPrivacy(key: string, value: string) {
  const on = value === "1" || value.toLowerCase() === "true";
  if (key === "capture_page_content") mockPrivacy = { ...mockPrivacy, capturePageContent: on };
  else if (key === "title_excluded_apps") {
    try {
      mockPrivacy = { ...mockPrivacy, titleExcludedApps: JSON.parse(value) as string[] };
    } catch {
      mockPrivacy = { ...mockPrivacy, titleExcludedApps: [] };
    }
  }
  else if (key === "store_raw_text") mockPrivacy = { ...mockPrivacy, storeRawText: on };
  else if (key === "delete_raw_after_classification")
    mockPrivacy = { ...mockPrivacy, deleteRawAfterClassification: on };
  else if (key === "smart_tracking_enabled")
    mockPrivacy = { ...mockPrivacy, smartTrackingEnabled: on };
  else if (key === "smart_interval_seconds")
    mockPrivacy = { ...mockPrivacy, smartIntervalSeconds: Number(value) || 60 };
  else if (key === "max_text_length")
    mockPrivacy = { ...mockPrivacy, maxTextLength: Number(value) || 8000 };
  else if (key === "retention_days")
    mockPrivacy = { ...mockPrivacy, retentionDays: Math.max(0, Number(value) || 0) };
  else if (key === "idle_threshold_seconds")
    mockPrivacy = { ...mockPrivacy, idleThresholdSeconds: Number(value) || 60 };
  else if (key === "count_media_as_active")
    mockPrivacy = { ...mockPrivacy, countMediaAsActive: on };
}

interface MockPage {
  id: number;
  domain: string;
  url: string;
  pageTitle: string;
  durationSeconds: number;
  contentType: string;
  category: Category;
  contentSummary: string;
  detectedKeywords: string[];
  projectName: string | null;
  projectConfidence: number;
  projectSignals: string[];
}

const mockPages: MockPage[] = [];

function mockWebsites(): WebsiteUsage[] {
  const map = new Map<string, WebsiteUsage>();
  for (const p of mockPages) {
    const rule = mockDomainRules.get(p.domain);
    const ex = map.get(p.domain);
    if (ex) {
      ex.seconds += p.durationSeconds;
      ex.pageViews += 1;
    } else {
      map.set(p.domain, {
        domain: p.domain,
        seconds: p.durationSeconds,
        category: rule?.category ?? null,
        pageViews: 1,
      });
    }
  }
  return [...map.values()];
}

function mockBrowserActivity(): BrowserActivityView {
  return {
    date: localDateIso(),
    perDomain: mockWebsites().sort((a, b) => b.seconds - a.seconds),
    recentPages: mockPages.map((p) => ({
      id: p.id,
      timestamp: new Date().toISOString(),
      domain: p.domain,
      url: p.url,
      pageTitle: p.pageTitle,
      durationSeconds: p.durationSeconds,
      contentType: p.contentType,
      category: p.category,
      contentSummary: p.contentSummary,
      detectedKeywords: p.detectedKeywords,
      hasRaw: false,
      isIdle: false,
      projectName: p.projectName,
      projectConfidence: p.projectConfidence,
      projectSignals: p.projectSignals,
    })),
  };
}

function mockTrackedDomains(): TrackedDomain[] {
  return mockWebsites()
    .map((w) => {
      const rule = mockDomainRules.get(w.domain);
      return {
        domain: w.domain,
        totalSeconds: w.seconds,
        category: rule?.category ?? null,
        captureMode: rule?.captureMode ?? ("meta" as CaptureMode),
        aiReview: rule?.aiReview ?? false,
      };
    })
    .sort((a, b) => b.totalSeconds - a.totalSeconds);
}

function mockActivityDetail(id: number): ActivityDetail {
  const p = mockPages.find((x) => x.id === id) ?? mockPages[0];
  return {
    id: p.id,
    timestamp: new Date().toISOString(),
    domain: p.domain,
    url: p.url,
    pageTitle: p.pageTitle,
    durationSeconds: p.durationSeconds,
    contentType: p.contentType,
    category: p.category,
    classificationReason: `content signals → ${p.category}`,
    contentSummary: p.contentSummary,
    detectedKeywords: p.detectedKeywords,
    rawTextExcerpt: null,
    contentCaptureEnabled: true,
    isIdle: false,
    projectName: p.projectName,
    projectConfidence: p.projectConfidence,
    projectSignals: p.projectSignals,
    classifier: p.id === 1 ? "llm" : "rule",
    llmConfidence: p.id === 1 ? 0.95 : null,
    confidence: p.id === 1 ? 0.6 : 0.85,
    blockKey: `web-${p.id}`,
  };
}

// ---------------------------------------------------------------------------
// Projects & Activity Log
// ---------------------------------------------------------------------------

export async function getProjects(): Promise<Project[]> {
  if (isTauri() || isRemote()) return callBackend<Project[]>("get_projects");
  return mockProjects.map((p) => ({ ...p }));
}

export async function createProject(p: Omit<Project, "id">): Promise<number> {
  if (isTauri() || isRemote()) return callBackend<number>("create_project", { project: { ...p, id: 0 } });
  const id = Math.max(0, ...mockProjects.map((x) => x.id)) + 1;
  mockProjects.push({ ...p, id });
  return id;
}

export async function updateProject(p: Project): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("update_project", { project: p });
    return;
  }
  const i = mockProjects.findIndex((x) => x.id === p.id);
  if (i >= 0) mockProjects[i] = { ...p };
}

export async function deleteProject(id: number): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_project", { id });
    return;
  }
  const i = mockProjects.findIndex((x) => x.id === id);
  if (i >= 0) mockProjects.splice(i, 1);
}

export async function testProjectMatch(
  project: Project,
  identifier: string,
  title: string,
  extra: string,
): Promise<ProjectMatchTest> {
  if (isTauri() || isRemote()) {
    return callBackend<ProjectMatchTest>("test_project_match", { project, identifier, title, extra });
  }
  const normalize = (value: string) => value.toLowerCase().replace(/[^a-z0-9]/g, "");
  const id = normalize(identifier);
  const excludedId = id.length > 0 ? [...project.excludedApps, ...project.excludedDomains].find((value) => {
    const item = normalize(value);
    return item.length >= 3 && (id.includes(item) || item.includes(id));
  }) : undefined;
  const hay = `${title} ${extra}`.toLowerCase();
  const excludedKeyword = project.excludedKeywords.find((value) => value.length >= 2 && hay.includes(value.toLowerCase()));
  if (excludedId || excludedKeyword) {
    const reason = excludedId ? `excluded app/domain: ${identifier}` : `excluded keyword: ${excludedKeyword}`;
    return { status: "excluded", confidence: 0, signals: [reason], explanation: `This project is blocked by ${reason}.` };
  }
  const signals: string[] = [];
  const relatedId = id.length > 0 && [...project.apps, ...project.domains].some((value) => {
    const item = normalize(value);
    return item.length >= 3 && (id.includes(item) || item.includes(id));
  });
  if (relatedId) signals.push(`app/domain: ${identifier}`);
  let hits = 0;
  for (const keyword of project.keywords) {
    const value = keyword.trim().toLowerCase();
    if (value.length < 2) continue;
    if (title.toLowerCase().includes(value)) {
      signals.push(`title keyword: ${keyword}`);
      hits += 1;
    } else if (extra.toLowerCase().includes(value)) {
      signals.push(`content keyword: ${keyword}`);
      hits += 1;
    }
  }
  if (!relatedId && hits === 0) {
    return { status: "no_match", confidence: 0, signals: [], explanation: "No related app, domain, or keyword was found." };
  }
  const confidence = Math.min(100, (relatedId ? 50 : 0) + (hits >= 1 ? 35 : 0) + (hits >= 2 ? 20 : 0) + (hits >= 3 ? 10 : 0) + (hits >= 4 ? 10 : 0));
  return {
    status: confidence >= 60 ? "assigned" : "candidate",
    confidence,
    signals,
    explanation: confidence >= 60
      ? `Would assign ${project.name} at ${confidence}% confidence.`
      : `Found some evidence, but ${confidence}% is below the 60% assignment threshold.`,
  };
}

export async function excludeActivityFromProject(
  projectName: string,
  source: ActivityLogEntry["source"],
  label: string,
): Promise<void> {
  if (isTauri()) {
    await invoke("exclude_activity_from_project", { projectName, source, label });
    return;
  }
  const project = mockProjects.find((item) => item.name === projectName);
  if (!project) return;
  const list = source === "web" ? project.excludedDomains : project.excludedApps;
  if (!list.some((item) => item.toLowerCase() === label.toLowerCase())) list.push(label);
}

export async function getRecentActivity(): Promise<ActivityLogEntry[]> {
  if (isTauri()) return invoke<ActivityLogEntry[]>("get_recent_activity");
  return mockRecentActivity();
}

// ---------------------------------------------------------------------------
// Local LLM (Ollama)
// ---------------------------------------------------------------------------

export async function getLlmSettings(): Promise<LlmSettings> {
  if (isTauri()) return invoke<LlmSettings>("get_llm_settings");
  return { ...mockLlm };
}

export async function setLlmSetting(key: string, value: string): Promise<void> {
  if (isTauri()) {
    await invoke("set_llm_setting", { key, value });
    return;
  }
  if (key === "llm_enabled") mockLlm = { ...mockLlm, enabled: value === "1" || value.toLowerCase() === "true" };
  else if (key === "llm_provider") mockLlm = { ...mockLlm, provider: value as "ollama" | "openai" };
  else if (key === "ollama_url") mockLlm = { ...mockLlm, url: value };
  else if (key === "ollama_model") mockLlm = { ...mockLlm, model: value };
  else if (key === "openai_classification_model") mockLlm = { ...mockLlm, openaiClassificationModel: value };
  else if (key === "openai_review_model") mockLlm = { ...mockLlm, openaiReviewModel: value };
  else if (key === "openai_include_content") mockLlm = { ...mockLlm, openaiIncludeContent: value === "1" || value.toLowerCase() === "true" };
}

export async function testOllamaConnection(url: string, model: string): Promise<OllamaTestResult> {
  if (isTauri()) return invoke<OllamaTestResult>("test_ollama_connection", { url, model });
  return {
    ok: false,
    message: "Preview mode: run the desktop app to test against a local Ollama server.",
    models: [],
    modelAvailable: false,
  };
}

export async function setOpenAiApiKey(apiKey: string): Promise<void> {
  if (isTauri()) {
    await invoke("set_openai_api_key", { apiKey });
    return;
  }
  mockLlm = { ...mockLlm, openaiKeyConfigured: Boolean(apiKey.trim()) };
}

export async function clearOpenAiApiKey(): Promise<void> {
  if (isTauri()) {
    await invoke("clear_openai_api_key");
    return;
  }
  mockLlm = { ...mockLlm, openaiKeyConfigured: false };
}

export async function testOpenAiConnection(model: string): Promise<OllamaTestResult> {
  if (isTauri()) return invoke<OllamaTestResult>("test_openai_connection", { model });
  return {
    ok: false,
    message: "Preview mode: run the Windows desktop app to test OpenAI.",
    models: [],
    modelAvailable: false,
  };
}

export async function getLlmErrors(): Promise<LlmErrorEntry[]> {
  if (isTauri()) return invoke<LlmErrorEntry[]>("get_llm_errors");
  return [];
}

let mockLlm: LlmSettings = {
  enabled: false,
  provider: "ollama",
  url: "http://localhost:11434",
  model: "llama3.1:8b",
  openaiClassificationModel: "gpt-5.4-nano",
  openaiReviewModel: "gpt-5.4-mini",
  openaiIncludeContent: false,
  openaiKeyConfigured: false,
  lastError: null,
};

const mockProjects: Project[] = [];

function mockRecentActivity(): ActivityLogEntry[] {
  const web: ActivityLogEntry[] = mockPages.map((p): ActivityLogEntry => ({
    source: "web",
    label: p.domain,
    title: p.pageTitle,
    seconds: p.durationSeconds,
    category: p.category,
    activityKind: p.contentType === "video" ? "video" : "research",
    reason:
      p.id === 1
        ? "Studying assignment-problem algorithms"
        : p.projectConfidence >= 60
          ? `project: ${p.projectName} (${p.projectConfidence}%)`
          : "content",
    contentType: p.contentType,
    lastSeen: new Date().toISOString(),
    detailId: p.id,
    summary: p.contentSummary,
    projectName: p.projectName,
    projectConfidence: p.projectConfidence,
    projectSignals: p.projectSignals,
    classifier: p.id === 1 ? "llm" : "rule",
    llmConfidence: p.id === 1 ? 0.95 : null,
    confidence: p.id === 1 ? 0.6 : 0.85,
    blockKey: `web-${p.id}`,
  }));
  const apps: ActivityLogEntry[] = [];

  const screen: ActivityLogEntry[] = [];
  return [...screen, ...apps, ...web];
}

// ---------------------------------------------------------------------------
// Daily score
// ---------------------------------------------------------------------------

export async function getDailyScore(): Promise<ScoreReport> {
  if (isTauri() || isRemote()) return callBackend<ScoreReport>("get_daily_score");
  return mockScore();
}

export async function setCheckin(field: string, value: number): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_checkin", { field, value });
    return;
  }
  if (field === "main_goal_completed") {
    mockMainGoalCompleted = value !== 0;
    return;
  }
  const def = mockCheckinDefs.find((d) => d.id === field);
  if (def) mockCheckinValues.set(field, def.kind === "counter" ? Math.max(0, value) : value ? 1 : 0);
}

/** Drop today's manual value so an auto check-in returns to live detection. */
export async function clearCheckin(field: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("clear_checkin", { field });
    return;
  }
  mockCheckinValues.delete(field);
}

export async function getCheckins(): Promise<CheckinValue[]> {
  if (isTauri() || isRemote()) return callBackend<CheckinValue[]>("get_checkins");
  return mockCheckinDefs.map((d) => mockCheckinValue(d));
}

/** Preview has no tracker, so auto check-ins show as "nothing detected yet". */
function mockCheckinValue(d: CheckinDefinition): CheckinValue {
  const manual = mockCheckinValues.get(d.id);
  return {
    id: d.id,
    label: d.label,
    icon: d.icon,
    kind: d.kind,
    value: manual ?? 0,
    auto: d.autoKind !== "",
    detected: 0,
    overridden: d.autoKind !== "" && manual !== undefined,
  };
}

export async function getCheckinDefinitions(): Promise<CheckinDefinition[]> {
  if (isTauri() || isRemote()) return callBackend<CheckinDefinition[]>("get_checkin_definitions");
  return mockCheckinDefs.map((d) => ({ ...d }));
}

export async function upsertCheckinDefinition(checkin: CheckinDefinition): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("upsert_checkin_definition", { checkin });
    return;
  }
  const i = mockCheckinDefs.findIndex((d) => d.id === checkin.id);
  if (i >= 0) mockCheckinDefs[i] = { ...checkin, builtIn: mockCheckinDefs[i].builtIn };
  else mockCheckinDefs.push({ ...checkin, builtIn: false });
}

export async function deleteCheckinDefinition(id: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_checkin_definition", { id });
    return;
  }
  mockCheckinDefs = mockCheckinDefs.filter((d) => d.id !== id);
  mockCheckinValues.delete(id);
  mockStreakDefs = mockStreakDefs.filter((d) => !(d.kind === "checkin" && d.metric === id));
  mockScoreRules = mockScoreRules.filter((r) => !(r.kind === "checkin" && r.metric.split(",").includes(id)));
}

// --------------------------------------------------------------- daily goals

export async function getGoals(): Promise<Goal[]> {
  if (isTauri() || isRemote()) return callBackend<Goal[]>("get_goals");
  return sortMockGoals();
}

export async function addGoal(goal: GoalDraft): Promise<number> {
  if (isTauri() || isRemote()) return callBackend<number>("add_goal", { goal });
  const id = mockGoalId++;
  mockGoals.push({ id, completed: false, ...goal });
  return id;
}

export async function updateGoal(goal: Goal): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("update_goal", { goal });
    return;
  }
  const i = mockGoals.findIndex((g) => g.id === goal.id);
  if (i >= 0) mockGoals[i] = { ...goal };
}

export async function toggleGoal(id: number, completed: boolean): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("toggle_goal", { id, completed });
    return;
  }
  const g = mockGoals.find((x) => x.id === id);
  if (g) g.completed = completed;
}

export async function deleteGoal(id: number): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_goal", { id });
    return;
  }
  mockGoals = mockGoals.filter((g) => g.id !== id);
}

export async function setGoalRecurring(id: number, recurring: boolean): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_goal_recurring", { id, recurring });
    return;
  }
  const g = mockGoals.find((x) => x.id === id);
  if (g) g.recurring = recurring;
}

export async function copyPreviousGoals(): Promise<number> {
  if (isTauri() || isRemote()) return callBackend<number>("copy_previous_goals");
  // Preview: pretend yesterday had one extra mission we don't already have.
  const extra = "Review yesterday's notes";
  if (!mockGoals.some((g) => g.title === extra)) {
    mockGoals.push({
      id: mockGoalId++,
      title: extra,
      project: null,
      targetMinutes: null, targetCount: null, targetUnit: null,
      priority: "medium",
      completed: false,
      recurring: false,
    });
    return 1;
  }
  return 0;
}

const PRIORITY_RANK: Record<string, number> = { high: 0, medium: 1, low: 2 };
function sortMockGoals(): Goal[] {
  return [...mockGoals].sort(
    (a, b) => PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority] || a.id - b.id,
  );
}

export async function setScoringWeight(id: string, weight: number): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_scoring_weight", { id, weight });
    return;
  }
  const r = mockScoreRules.find((x) => x.id === id);
  if (r) r.weight = weight;
}

export async function setScoringThreshold(id: string, threshold: number): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_scoring_threshold", { id, threshold });
    return;
  }
  const r = mockScoreRules.find((x) => x.id === id);
  if (r && r.threshold != null) r.threshold = threshold;
}

export async function resetScoringWeights(): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("reset_scoring_weights");
    return;
  }
  mockScoreRules = DEFAULT_SCORE_RULES.map((r) => ({ ...r }));
}

export async function getScoreRules(): Promise<ScoreRule[]> {
  if (isTauri() || isRemote()) return callBackend<ScoreRule[]>("get_score_rules");
  return mockScoreRules.map((r) => ({ ...r }));
}

export async function upsertScoreRule(rule: ScoreRule): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("upsert_score_rule", { rule });
    return;
  }
  const i = mockScoreRules.findIndex((r) => r.id === rule.id);
  if (i >= 0) mockScoreRules[i] = { ...rule, builtIn: mockScoreRules[i].builtIn };
  else mockScoreRules.push({ ...rule, builtIn: false });
}

export async function deleteScoreRule(id: string): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("delete_score_rule", { id });
    return;
  }
  mockScoreRules = mockScoreRules.filter((r) => r.id !== id);
}

const DEFAULT_SCORE_RULES: ScoreRule[] = [
  { id: "main_goal", label: "Completed main daily goal", kind: "goal", metric: "", weight: 30, threshold: null, builtIn: true },
  { id: "no_main_goal", label: "No main goal completed", kind: "no_goal", metric: "", weight: -25, threshold: null, builtIn: true },
];
const DEFAULT_CHECKIN_DEFS: CheckinDefinition[] = [
  { id: "videos_posted", label: "Posted video", icon: "🎬", kind: "counter", builtIn: true, autoKind: "", autoMetric: "", autoThreshold: 0 },
  { id: "gym_logged", label: "Went gym", icon: "🏋️", kind: "toggle", builtIn: true, autoKind: "", autoMetric: "", autoThreshold: 0 },
  { id: "wrestled", label: "Wrestled", icon: "🤼", kind: "toggle", builtIn: true, autoKind: "", autoMetric: "", autoThreshold: 0 },
  { id: "studied", label: "Studied", icon: "📚", kind: "toggle", builtIn: true, autoKind: "", autoMetric: "", autoThreshold: 0 },
  { id: "edited_video", label: "Edited video", icon: "✂️", kind: "toggle", builtIn: true, autoKind: "", autoMetric: "", autoThreshold: 0 },
  { id: "analysed_content", label: "Analysed content", icon: "🔍", kind: "toggle", builtIn: true, autoKind: "", autoMetric: "", autoThreshold: 0 },
];

let mockMainGoalCompleted = false;
let mockCheckinDefs: CheckinDefinition[] = DEFAULT_CHECKIN_DEFS.map((d) => ({ ...d }));
const mockCheckinValues = new Map<string, number>();
let mockScoreRules: ScoreRule[] = DEFAULT_SCORE_RULES.map((r) => ({ ...r }));
let mockGoalId = 1;
let mockGoals: Goal[] = [];

function mockLoggedCheckins(): CheckinValue[] {
  return mockCheckinDefs.map((d) => mockCheckinValue(d)).filter((c) => c.value > 0);
}

function mockScore(): ScoreReport {
  const sortedGoals = sortMockGoals();
  // Main goal is derived from the top-priority goal when any goals exist.
  const mainGoalDone = sortedGoals.length ? sortedGoals[0].completed : mockMainGoalCompleted;
  const checkinVal = (metric: string) =>
    Math.max(...metric.split(",").map((m) => mockCheckinValues.get(m.trim()) ?? 0), 0);

  let raw = 0;
  const lines: ScoreLine[] = mockScoreRules.map((r) => {
    const positive = r.weight >= 0;
    let triggered = false;
    let value = "";
    switch (r.kind) {
      case "goal":
        triggered = sortedGoals.length > 0 && mainGoalDone;
        value = sortedGoals.length ? (mainGoalDone ? "done" : "not done") : "no goal";
        break;
      case "no_goal":
        triggered = sortedGoals.length > 0 && !mainGoalDone;
        value = sortedGoals.length ? (mainGoalDone ? "completed" : "not completed") : "no goal";
        break;
      case "checkin": {
        const v = checkinVal(r.metric);
        const thr = Math.max(1, r.threshold ?? 1);
        triggered = v >= thr;
        value = thr <= 1 && v <= 1 ? (v > 0 ? "logged" : "not logged") : `${v}×`;
        break;
      }
      // Preview has no tracked time, so time-based rules stay untriggered.
      case "category":
      case "target":
        value = "0m";
        break;
      case "late_start":
        value = "no productive block";
        break;
      case "output":
        value = "0 detected";
        break;
    }
    if (triggered) raw += r.weight;
    return {
      id: r.id,
      label: r.label,
      weight: r.weight,
      threshold: r.threshold,
      hasThreshold: r.threshold != null,
      positive,
      triggered,
      value,
    };
  });

  const score = Math.max(0, Math.min(100, raw));
  const verdict = score >= 85 ? "excellent" : score >= 70 ? "good" : score >= 50 ? "mid" : score >= 30 ? "bad" : "cooked";
  const topWins = lines.filter((l) => l.positive && l.triggered).sort((a, b) => b.weight - a.weight).slice(0, 3);
  const biggestLeaks = lines.filter((l) => !l.positive && l.triggered).sort((a, b) => a.weight - b.weight).slice(0, 3);

  const suggestion = !sortedGoals.length
    ? "Add a main goal and start tracking to build today's score."
    : !mainGoalDone
      ? mockAccountability.mainGoalDeadline
        ? `Finish your main goal by your preferred ${new Date(`2000-01-01T${mockAccountability.mainGoalDeadline}`).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} deadline — it's worth the most points.`
        : "Finish your main goal when your schedule allows — it's worth the most points."
      : "Strong, balanced day — keep the momentum tomorrow.";

  return {
    date: localDateIso(),
    score,
    verdict,
    topWins,
    biggestLeaks,
    suggestion,
    lines,
    categoryMinutes: [],
    mainGoalCompleted: mainGoalDone,
    checkins: mockLoggedCheckins(),
    mainGoalName: sortedGoals[0]?.title ?? null,
  };
}

// --------------------------------------------------------- accountability mocks

let mockAccountability: AccountabilitySettings = {
  distractionWarnEnabled: true,
  distractionWarnMinutes: 20,
  eodPopupEnabled: false,
  eodPopupTime: "21:00",
  mainGoalDeadline: "",
};

let mockFocus: FocusSession | null = null;
function mockRemaining(): number {
  if (!mockFocus || mockFocus.status !== "active") return 0;
  return Math.max(0, Math.round((new Date(mockFocus.endsAt).getTime() - Date.now()) / 1000));
}

function mockWeekly(): WeeklyReview {
  const days: WeeklyDay[] = Array.from({ length: 7 }, (_, i) => {
    const d = new Date();
    d.setDate(d.getDate() - (6 - i));
    return {
      day: localDateIso(d),
      weekday: d.toLocaleDateString(undefined, { weekday: "short" }),
      score: 0,
      productiveSeconds: 0,
      distractionSeconds: 0,
      trackedSeconds: 0,
    };
  });
  return {
    startDay: days[0].day,
    endDay: days[days.length - 1].day,
    productiveSeconds: 0,
    distractionSeconds: 0,
    studySeconds: 0,
    checkinTotals: [],
    bestDay: null,
    worstDay: null,
    mostCommonLeak: null,
    mostCommonLeakSeconds: 0,
    days,
  };
}

function mockTimeline(day: string): TimelineDay {
  const blocks: TimelineBlock[] = [];

  // Flag highlights exactly the way the backend does.
  const nonIdle = blocks.filter((b) => !b.idle);
  const prod = nonIdle.filter((b) => b.bucket === "productive");
  const dist = nonIdle.filter((b) => b.bucket === "distracting");
  const qualifyingProd = prod.filter((b) => b.durationSeconds >= 5 * 60);
  if (prod.length) {
    prod.reduce((a, b) => (b.durationSeconds > a.durationSeconds ? b : a)).longestProductive = true;
  }
  if (qualifyingProd.length) {
    qualifyingProd[0].firstProductive = true;
  }
  if (dist.length) {
    dist.reduce((a, b) => (b.durationSeconds > a.durationSeconds ? b : a)).biggestDistraction = true;
  }
  const goalProjects = new Set(["coding", "video content business", "exam studying"]);
  for (const b of blocks) {
    if (b.project && goalProjects.has(b.project.toLowerCase())) b.goalRelated = true;
    if (b.category === "business") b.outputLinked = true;
  }

  const sum = (pred: (b: TimelineBlock) => boolean) =>
    blocks.filter(pred).reduce((n, b) => n + b.durationSeconds, 0);

  return {
    day,
    maxGapSeconds: 120,
    blocks,
    overviewBlocks: blocks,
    outputs: mockLoggedCheckins(),
    activeSeconds: sum((b) => !b.idle),
    idleSeconds: sum((b) => b.idle),
    productiveSeconds: sum((b) => !b.idle && b.bucket === "productive"),
    distractedSeconds: sum((b) => !b.idle && b.bucket === "distracting"),
    firstProductiveStart: qualifyingProd.length ? qualifyingProd[0].start : null,
    goals: [],
  };
}

let mockFolderId = 1;
let mockFolders: WatchedFolder[] = [];

function mockOutputEvents(_day: string): OutputEvent[] {
  return [];
}

let mockStreakDefs: StreakDefinition[] = [];

const MOCK_CURRENT: Record<string, number> = {
  posted_video: 3, main_goal: 5, coding_60: 8, business_90: 2, study_60: 4,
  studied: 4, edited_video: 1, analysed_content: 0, gym: 6, wrestling: 0,
  productive_block_60: 8, no_major_distraction: 2,
};

function runLengths(arr: boolean[]): number[] {
  const out: number[] = [];
  let c = 0;
  for (const b of arr) {
    if (b) c++;
    else {
      if (c) out.push(c);
      c = 0;
    }
  }
  if (c) out.push(c);
  return out.length ? out : [0];
}

function mockStreaks(): Streak[] {
  const days: string[] = [];
  for (let i = 27; i >= 0; i--) {
    const d = new Date();
    d.setDate(d.getDate() - i);
    days.push(localDateIso(d));
  }
  return mockStreakDefs
    .filter((d) => d.enabled)
    .map((def) => {
      const cur = MOCK_CURRENT[def.id] ?? 0;
      const calendar = days.map((day, idx) => {
        const fromEnd = days.length - 1 - idx;
        let met = fromEnd < cur;
        if (!met) met = (idx * 7 + def.id.length * 3) % 5 === 0;
        return { day, met };
      });
      const best = Math.max(cur, ...runLengths(calendar.map((c) => c.met)));
      const last = [...calendar].reverse().find((c) => c.met)?.day ?? null;
      return {
        id: def.id,
        name: def.name,
        kind: def.kind,
        metric: def.id,
        threshold: def.threshold,
        enabled: def.enabled,
        daysPerWeek: def.daysPerWeek,
        current: cur,
        best,
        weekMetDays: def.daysPerWeek > 0 ? Math.min(cur, def.daysPerWeek) : 0,
        lastCompletedDay: last,
        calendar,
      };
    });
}

let mockLockinAuto = true;
const mockLockins: Record<string, LockinPlan> = {};

function mockGenLockin(day: string): LockinPlan {
  const p: LockinPlan = {
    day,
    mainMission: "Add tomorrow's main mission",
    secondaryMissions: [],
    firstBlock: "Choose a first focused block after you add a goal.",
    distractionRule: "No distraction rule yet.",
    focusMode: "Start a Focus session once you have a mission.",
    avoidTrap: "No tracked pattern yet.",
    roastLine: "",
    source: "fallback",
    edited: false,
  };
  mockLockins[day] = p;
  return p;
}
