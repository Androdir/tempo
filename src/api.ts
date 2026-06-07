import { invoke } from "@tauri-apps/api/core";
import { categoryMeta } from "./categories";
import type {
  ActivityDetail,
  ActivityLogEntry,
  BrowserActivityView,
  CaptureMode,
  Category,
  AccountabilitySettings,
  CategoryRule,
  CheckinState,
  DailyAiReview,
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
  WatchedFolder,
  ScoreLine,
  ScoreReport,
  Streak,
  SyncStatus,
  StreakDefinition,
  TimelineBlock,
  TimelineDay,
  TodaySummary,
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
 * When it's opened in a plain browser (e.g. `npm run dev` for UI work) we
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
    t = window.prompt("Enter the Tempo Hub secret to view the dashboard:") || "";
    if (t) localStorage.setItem("tempo_web_token", t);
  }
  return t;
}

async function remoteInvoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const res = await fetch("/api/invoke", {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${webToken()}` },
    body: JSON.stringify({ cmd, args }),
  });
  if (res.status === 401) {
    if (typeof localStorage !== "undefined") localStorage.removeItem("tempo_web_token");
    throw new Error("Unauthorized — re-enter the hub secret and reload.");
  }
  if (!res.ok) throw new Error(`Hub command "${cmd}" failed (${res.status})`);
  return (await res.json()) as T;
}

/** Route a command to Tauri (desktop) or the hub (remote). */
async function callBackend<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return isTauri() ? invoke<T>(cmd, args) : remoteInvoke<T>(cmd, args);
}

// ---------------------------------------------------------------------------
// Real backend calls
// ---------------------------------------------------------------------------

export async function getTodaySummary(): Promise<TodaySummary> {
  if (isTauri() || isRemote()) return callBackend<TodaySummary>("get_today_summary");
  return mockSummary();
}

export async function getTrackedApps(): Promise<TrackedApp[]> {
  if (isTauri()) return invoke<TrackedApp[]>("get_tracked_apps");
  return mockTrackedApps();
}

export async function getCategoryRules(): Promise<CategoryRule[]> {
  if (isTauri()) return invoke<CategoryRule[]>("get_category_rules");
  return [...mockCategories]
    .filter(([, c]) => c)
    .map(([appName, category]) => ({ appName, category: category as Category }));
}

export async function setCategoryRule(
  appName: string,
  category: Category,
  aiReview = false
): Promise<void> {
  if (isTauri()) {
    await invoke("set_category_rule", { appName, category, aiReview });
    return;
  }
  mockCategories.set(appName, category);
  mockAppAi.set(appName, aiReview);
}

export async function deleteCategoryRule(appName: string): Promise<void> {
  if (isTauri()) {
    await invoke("delete_category_rule", { appName });
    return;
  }
  mockCategories.set(appName, null);
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
 * Per-device active time today. Only the hub has the multi-device ledger, so this
 * is a remote-only view: the desktop app is a single device and returns nothing.
 */
export async function getDeviceBreakdown(day: string): Promise<DeviceUsage[]> {
  if (isRemote()) return callBackend<DeviceUsage[]>("get_device_breakdown", { day });
  if (isTauri()) return []; // desktop is one device — the hub serves this view
  return mockDeviceBreakdown();
}

function mockDeviceBreakdown(): DeviceUsage[] {
  return [];
}

export async function getStreakDefinitions(): Promise<StreakDefinition[]> {
  if (isTauri()) return invoke<StreakDefinition[]>("get_streak_definitions");
  return mockStreakDefs.map((d) => ({ ...d }));
}

export async function updateStreakDefinition(
  id: string,
  patch: { enabled?: boolean; threshold?: number },
): Promise<void> {
  if (isTauri()) {
    await invoke("update_streak_definition", { id, enabled: patch.enabled, threshold: patch.threshold });
    return;
  }
  const d = mockStreakDefs.find((x) => x.id === id);
  if (d) {
    if (patch.enabled !== undefined) d.enabled = patch.enabled;
    if (patch.threshold !== undefined) d.threshold = patch.threshold;
  }
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
      mockGoals.push({ id: mockGoalId++, title, project: null, targetMinutes: null, priority, completed: false, recurring: false });
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

export async function insertSampleData(): Promise<void> {
  if (isTauri()) {
    await invoke("insert_sample_data");
    return;
  }
  // Browser preview already starts with sample data, so this is a no-op.
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

export async function getAccountabilitySettings(): Promise<AccountabilitySettings> {
  if (isTauri()) return invoke<AccountabilitySettings>("get_accountability_settings");
  return { ...mockAccountability };
}

export async function setAccountabilitySetting(key: string, value: string): Promise<void> {
  if (isTauri()) {
    await invoke("set_accountability_setting", { key, value });
    return;
  }
  if (key === "distraction_warn_enabled") mockAccountability.distractionWarnEnabled = value === "1";
  else if (key === "distraction_warn_minutes")
    mockAccountability.distractionWarnMinutes = Math.max(1, Math.min(240, parseInt(value, 10) || 20));
  else if (key === "eod_popup_enabled") mockAccountability.eodPopupEnabled = value === "1";
  else if (key === "eod_popup_time") mockAccountability.eodPopupTime = value;
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

const mockCategories = new Map<string, Category | null>();
const mockAppAi = new Map<string, boolean>();
const mockSeconds = new Map<string, number>();

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
  if (isTauri()) return invoke<DomainRule[]>("get_domain_rules");
  return [...mockDomainRules.values()];
}

export async function getTrackedDomains(): Promise<TrackedDomain[]> {
  if (isTauri()) return invoke<TrackedDomain[]>("get_tracked_domains");
  return mockTrackedDomains();
}

export async function setDomainRule(
  domain: string,
  category: Category | null,
  captureMode: CaptureMode,
  aiReview = false
): Promise<void> {
  if (isTauri()) {
    await invoke("set_domain_rule", { domain, category, captureMode, aiReview });
    return;
  }
  mockDomainRules.set(domain, { domain, category, captureMode, aiReview });
}

export async function deleteDomainRule(domain: string): Promise<void> {
  if (isTauri()) {
    await invoke("delete_domain_rule", { domain });
    return;
  }
  mockDomainRules.delete(domain);
}

export async function getPrivacySettings(): Promise<PrivacySettings> {
  if (isTauri()) return invoke<PrivacySettings>("get_privacy_settings");
  return { ...mockPrivacy };
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
};

function applyMockPrivacy(key: string, value: string) {
  const on = value === "1" || value.toLowerCase() === "true";
  if (key === "capture_page_content") mockPrivacy = { ...mockPrivacy, capturePageContent: on };
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
  else if (key === "ollama_url") mockLlm = { ...mockLlm, url: value };
  else if (key === "ollama_model") mockLlm = { ...mockLlm, model: value };
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

export async function getLlmErrors(): Promise<LlmErrorEntry[]> {
  if (isTauri()) return invoke<LlmErrorEntry[]>("get_llm_errors");
  return [];
}

let mockLlm: LlmSettings = {
  enabled: false,
  url: "http://localhost:11434",
  model: "llama3.1:8b",
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
  if (field === "main_goal_completed") mockCheckins.mainGoalCompleted = value !== 0;
  else if (field === "videos_posted") mockCheckins.videosPosted = Math.max(0, value);
  else if (field === "gym_logged") mockCheckins.gymLogged = value !== 0;
  else if (field === "wrestled") mockCheckins.wrestled = value !== 0;
  else if (field === "studied") mockCheckins.studied = value !== 0;
  else if (field === "edited_video") mockCheckins.editedVideo = value !== 0;
  else if (field === "analysed_content") mockCheckins.analysedContent = value !== 0;
}

export async function getCheckins(): Promise<CheckinState> {
  if (isTauri() || isRemote()) return callBackend<CheckinState>("get_checkins");
  return {
    videosPosted: mockCheckins.videosPosted,
    gymLogged: mockCheckins.gymLogged,
    wrestled: mockCheckins.wrestled,
    studied: mockCheckins.studied,
    editedVideo: mockCheckins.editedVideo,
    analysedContent: mockCheckins.analysedContent,
  };
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
      targetMinutes: null,
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
  mockWeights[id] = weight;
}

export async function setScoringThreshold(id: string, threshold: number): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("set_scoring_threshold", { id, threshold });
    return;
  }
  mockThresholds[id] = threshold;
}

export async function resetScoringWeights(): Promise<void> {
  if (isTauri() || isRemote()) {
    await callBackend("reset_scoring_weights");
    return;
  }
  mockWeights = {};
  mockThresholds = {};
}

interface MockRuleDef {
  id: string;
  label: string;
  weight: number;
  threshold: number | null;
  positive: boolean;
}

const SCORE_RULES: MockRuleDef[] = [
  { id: "main_goal", label: "Completed main daily goal", weight: 30, threshold: null, positive: true },
  { id: "posted_video", label: "Posted 1+ videos", weight: 25, threshold: null, positive: true },
  { id: "business_min", label: "90+ min editing / business work", weight: 20, threshold: 90, positive: true },
  { id: "study_min", label: "60+ min studying", weight: 15, threshold: 60, positive: true },
  { id: "coding_min", label: "60+ min coding / building", weight: 15, threshold: 60, positive: true },
  { id: "gym", label: "Gym / wrestling logged", weight: 10, threshold: null, positive: true },
  { id: "instagram", label: "Instagram distraction over 30 min", weight: -15, threshold: 30, positive: false },
  { id: "youtube", label: "YouTube distraction over 45 min", weight: -10, threshold: 45, positive: false },
  { id: "recovery", label: "Music / pacing / recovery over 60 min", weight: -15, threshold: 60, positive: false },
  { id: "no_main_goal", label: "No main goal completed", weight: -25, threshold: null, positive: false },
  { id: "late_start", label: "First productive block after 14:00", weight: -10, threshold: 14, positive: false },
];

let mockCheckins = {
  mainGoalCompleted: false,
  videosPosted: 0,
  gymLogged: false,
  wrestled: false,
  studied: false,
  editedVideo: false,
  analysedContent: false,
};
let mockGoalId = 1;
let mockGoals: Goal[] = [];
let mockWeights: Record<string, number> = {};
let mockThresholds: Record<string, number> = {};
const mockStats = { business: 0, study: 0, coding: 0, recovery: 0, instagram: 0, youtube: 0, firstProductiveMin: 0 };

function mockScore(): ScoreReport {
  const s = mockStats;
  const sortedGoals = sortMockGoals();
  // Main goal is derived from the top-priority goal when any goals exist.
  const mainGoalDone = sortedGoals.length ? sortedGoals[0].completed : mockCheckins.mainGoalCompleted;
  const gymOrWrestle = mockCheckins.gymLogged || mockCheckins.wrestled;
  let raw = 0;
  const lines: ScoreLine[] = SCORE_RULES.map((r) => {
    const weight = mockWeights[r.id] ?? r.weight;
    const threshold = r.threshold == null ? null : mockThresholds[r.id] ?? r.threshold;
    let triggered = false;
    let value = "";
    switch (r.id) {
      case "main_goal": triggered = mainGoalDone; value = triggered ? "done" : "not done"; break;
      case "posted_video": triggered = mockCheckins.videosPosted >= 1; value = `${mockCheckins.videosPosted} posted`; break;
      case "business_min": triggered = s.business >= (threshold ?? 90); value = `${s.business}m`; break;
      case "study_min": triggered = s.study >= (threshold ?? 60); value = `${s.study}m`; break;
      case "coding_min": triggered = s.coding >= (threshold ?? 60); value = `${s.coding}m`; break;
      case "gym": triggered = gymOrWrestle; value = triggered ? "logged" : "not logged"; break;
      case "instagram": triggered = s.instagram > (threshold ?? 30); value = `${s.instagram}m`; break;
      case "youtube": triggered = s.youtube > (threshold ?? 45); value = `${s.youtube}m`; break;
      case "recovery": triggered = s.recovery > (threshold ?? 60); value = `${s.recovery}m`; break;
      case "no_main_goal": triggered = sortedGoals.length > 0 && !mainGoalDone; value = sortedGoals.length ? (triggered ? "not completed" : "completed") : "no goal"; break;
      case "late_start": {
        const cut = (threshold ?? 14) * 60;
        triggered = s.firstProductiveMin > cut;
        value = `first at ${String(Math.floor(s.firstProductiveMin / 60)).padStart(2, "0")}:${String(s.firstProductiveMin % 60).padStart(2, "0")}`;
        break;
      }
    }
    if (triggered) raw += weight;
    return { id: r.id, label: r.label, weight, threshold, hasThreshold: r.threshold != null, positive: r.positive, triggered, value };
  });

  const score = Math.max(0, Math.min(100, raw));
  const verdict = score >= 85 ? "excellent" : score >= 70 ? "good" : score >= 50 ? "mid" : score >= 30 ? "bad" : "cooked";
  const topWins = lines.filter((l) => l.positive && l.triggered).sort((a, b) => b.weight - a.weight).slice(0, 3);
  const biggestLeaks = lines.filter((l) => !l.positive && l.triggered).sort((a, b) => a.weight - b.weight).slice(0, 3);
  const categoryMinutes = [
    { category: "productive", minutes: s.coding },
    { category: "business", minutes: s.business },
    { category: "study", minutes: s.study },
    { category: "recovery", minutes: s.recovery },
    { category: "distraction", minutes: s.instagram },
    { category: "neutral", minutes: s.youtube },
  ].filter((c) => c.minutes > 0).sort((a, b) => b.minutes - a.minutes);

  const leak = biggestLeaks[0];
  const suggestion = !sortedGoals.length
    ? "Add a main goal and start tracking to build today's score."
    : !mainGoalDone
    ? "Finish your main goal before 2pm — worth 55 points and removes the penalty."
    : leak
      ? `Cut ${leak.id === "instagram" ? "Instagram" : leak.id === "youtube" ? "YouTube" : "recovery/music time"} below ${leak.threshold}m to save ${Math.abs(leak.weight)} points.`
      : "Strong, balanced day — keep the momentum tomorrow.";

  return {
    date: localDateIso(),
    score,
    verdict,
    topWins,
    biggestLeaks,
    suggestion,
    lines,
    categoryMinutes,
    mainGoalCompleted: mainGoalDone,
    videosPosted: mockCheckins.videosPosted,
    gymLogged: gymOrWrestle,
    mainGoalName: sortedGoals[0]?.title ?? null,
  };
}

// --------------------------------------------------------- accountability mocks

let mockAccountability: AccountabilitySettings = {
  distractionWarnEnabled: true,
  distractionWarnMinutes: 20,
  eodPopupEnabled: false,
  eodPopupTime: "21:00",
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
    videosPosted: 0,
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
  if (prod.length) {
    prod.reduce((a, b) => (b.durationSeconds > a.durationSeconds ? b : a)).longestProductive = true;
    prod[0].firstProductive = true;
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
    outputs: { videosPosted: 0, gymLogged: false, wrestled: false, studied: false, editedVideo: false, analysedContent: false },
    activeSeconds: sum((b) => !b.idle),
    idleSeconds: sum((b) => b.idle),
    productiveSeconds: sum((b) => !b.idle && b.bucket === "productive"),
    distractedSeconds: sum((b) => !b.idle && b.bucket === "distracting"),
    firstProductiveStart: prod.length ? prod[0].start : null,
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
        current: cur,
        best,
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
