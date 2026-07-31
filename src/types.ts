// Shared types. Field names are camelCase to match the Rust backend, whose
// serde structs are annotated with `#[serde(rename_all = "camelCase")]`.

export type Category = string;

export type Bucket = "productive" | "neutral" | "distracting";

export interface ClassificationPolicy {
  id: string;
  name: string;
  category: string;
  kinds: string[];
  terms: string[];
  enabled: boolean;
  builtIn: boolean;
  priority: number;
}
export interface CategoryDefinition {
  id: string;
  label: string;
  color: string;
  bucket: Bucket;
  blurb: string;
  builtIn: boolean;
}

/** One app's active time today. `category` is null when the user hasn't tagged it. */
export interface AppUsage {
  appName: string;
  seconds: number;
  category: Category | null;
}

/** Aggregated time for a single category (may include the pseudo "uncategorized"). */
export interface CategoryUsage {
  category: Category | "uncategorized";
  seconds: number;
}

/** Aggregated time rolled up into the three high-level buckets. */
export interface BucketUsage {
  bucket: Bucket;
  seconds: number;
}

export interface TodaySummary {
  date: string;
  totalActiveSeconds: number;
  totalIdleSeconds: number;
  totalBrowserSeconds: number;
  perApp: AppUsage[];
  perWebsite: WebsiteUsage[];
  perCategory: CategoryUsage[];
  perBucket: BucketUsage[];
}


export interface TimeBreakdown {
  startDate: string;
  endDate: string;
  dayCount: number;
  totalActiveSeconds: number;
  totalIdleSeconds: number;
  totalBrowserSeconds: number;
  perApp: AppUsage[];
  perWebsite: WebsiteUsage[];
  perCategory: CategoryUsage[];
  perBucket: BucketUsage[];
}
/** Active time one device contributed today (hub view; idle excluded). */
export interface DeviceUsage {
  deviceId: string;
  name: string;
  platform: string;
  activeSeconds: number;
  topLabel: string;
  topLabelSeconds: number;
}

export type ContentType =
  | "article"
  | "video"
  | "chat"
  | "docs_editor"
  | "social_feed"
  | "search_results"
  | "other";

export type CaptureMode = "text" | "meta" | "never";

export interface WebsiteUsage {
  domain: string;
  seconds: number;
  category: Category | null;
  pageViews: number;
}

export interface BrowserPage {
  id: number;
  timestamp: string;
  domain: string;
  url: string;
  pageTitle: string;
  durationSeconds: number;
  contentType: string | null;
  category: Category;
  contentSummary: string | null;
  detectedKeywords: string[];
  hasRaw: boolean;
  isIdle: boolean;
  projectName: string | null;
  projectConfidence: number;
  projectSignals: string[];
}

export interface BrowserActivityView {
  date: string;
  perDomain: WebsiteUsage[];
  recentPages: BrowserPage[];
}

export interface ActivityDetail {
  id: number;
  timestamp: string;
  domain: string;
  url: string;
  pageTitle: string;
  durationSeconds: number;
  contentType: string | null;
  category: Category | "ignore";
  classificationReason: string;
  contentSummary: string | null;
  detectedKeywords: string[];
  rawTextExcerpt: string | null;
  contentCaptureEnabled: boolean;
  isIdle: boolean;
  projectName: string | null;
  projectConfidence: number;
  projectSignals: string[];
  classifier: "llm" | "rule" | "manual";
  llmConfidence: number | null;
  confidence: number;
  blockKey: string;
}

export interface LlmSettings {
  enabled: boolean;
  url: string;
  model: string;
  lastError: string | null;
}

export interface OllamaTestResult {
  ok: boolean;
  message: string;
  models: string[];
  modelAvailable: boolean;
}

export interface LlmErrorEntry {
  timestamp: string;
  context: string | null;
  message: string;
}

export interface ScoreLine {
  id: string;
  label: string;
  weight: number;
  threshold: number | null;
  hasThreshold: boolean;
  positive: boolean;
  triggered: boolean;
  value: string;
}

export interface CategoryMinutes {
  category: string;
  minutes: number;
}

export interface ScoreReport {
  date: string;
  score: number;
  verdict: string;
  topWins: ScoreLine[];
  biggestLeaks: ScoreLine[];
  suggestion: string;
  lines: ScoreLine[];
  categoryMinutes: CategoryMinutes[];
  mainGoalCompleted: boolean;
  checkins: CheckinValue[]; // today's logged check-ins (value > 0)
  mainGoalName: string | null;
}

/** One user-editable daily-score rule. */
export type ScoreRuleKind =
  | "checkin"
  | "category"
  | "target"
  | "goal"
  | "no_goal"
  | "late_start"
  | "output";

export interface ScoreRule {
  id: string;
  label: string;
  kind: ScoreRuleKind;
  metric: string;
  weight: number;
  threshold: number | null;
  builtIn: boolean;
}

export interface CorrectionHistoryEntry {
  id: number;
  blockKey: string;
  source: string;
  label: string;
  title: string;
  previousManualCategory: string | null;
  previousRuleCategory: string | null;
  newCategory: string;
  createdAt: string;
  undoneAt: string | null;
}
export interface AccountabilityExportOptions {
  startDate: string;
  endDate: string;
  includeActivityNames: boolean;
  includeTitles: boolean;
  includeRawText: boolean;
  includeNotes: boolean;
}

export interface AccountabilityExport {
  filename: string;
  markdown: string;
  startDate: string;
  endDate: string;
  dayCount: number;
  trackedDays: number;
  activeSeconds: number;
}
export interface DatabaseBackup {
  name: string;
  createdAt: string;
  bytes: number;
  automatic: boolean;
}

export interface TrackingHealth {
  status: "healthy" | "warning";
  checkedAt: string;
  databaseOk: boolean;
  lastDesktopAt: string | null;
  lastBrowserAt: string | null;
  lastScreenAt: string | null;
  browserConnected: boolean;
  smartEnabled: boolean;
  pendingSyncEvents: number;
  lastBackupAt: string | null;
  issues: string[];
}
export interface Project {
  id: number;
  name: string;
  category: Category;
  keywords: string[];
  apps: string[];
  domains: string[];
  excludedApps: string[];
  excludedDomains: string[];
  excludedKeywords: string[];
  priority: number;
}

export interface ProjectMatchTest {
  status: "assigned" | "candidate" | "no_match" | "excluded";
  confidence: number;
  signals: string[];
  explanation: string;
}

export type Priority = "low" | "medium" | "high";

/** A user-defined daily goal ("main mission today"). */
export interface Goal {
  id: number;
  title: string;
  project: string | null;
  targetMinutes: number | null;
  targetCount: number | null;
  targetUnit: string | null;
  priority: Priority;
  completed: boolean;
  recurring: boolean;
}

/** Fields for creating/editing a goal (no id/completed). */
export interface GoalDraft {
  title: string;
  project: string | null;
  targetMinutes: number | null;
  targetCount: number | null;
  targetUnit: string | null;
  priority: Priority;
  recurring: boolean;
}

/** How a check-in auto-detects: "" = manual only, "target" = active minutes on
 * an app/site, "output" = files detected by the proof-of-output watcher. */
export type CheckinAutoKind = "" | "target" | "output";

/** A user-editable check-in definition ("things the tracker can't see" — unless
 * an auto source is configured, in which case it ticks itself). */
export interface CheckinDefinition {
  id: string;
  label: string;
  icon: string;
  kind: "toggle" | "counter";
  builtIn: boolean;
  autoKind: CheckinAutoKind;
  autoMetric: string;
  autoThreshold: number;
}

/** A check-in definition with its effective value for a day (toggles 0/1,
 * counters 0..N). Auto check-ins read live detection unless overridden. */
export interface CheckinValue {
  id: string;
  label: string;
  icon: string;
  kind: "toggle" | "counter";
  value: number;
  /** This check-in has an auto-detection source configured. */
  auto: boolean;
  /** Raw detected amount today: active minutes (target) or file count (output). */
  detected: number;
  /** Auto check-in whose value was manually overridden today. */
  overridden: boolean;
}

// ----------------------------------------------------------- accountability

/** A focus session ("focus mode"). Soft enforcement — warnings only. */
export interface FocusSession {
  id: number;
  goal: string | null;
  startedAt: string;
  durationMinutes: number;
  endsAt: string;
  allowed: string[];
  blocked: string[];
  status: "active" | "completed" | "ended";
  endedAt: string | null;
  remainingSeconds: number;
}

export interface FocusSummary {
  goal: string | null;
  durationMinutes: number;
  status: string;
  focusedSeconds: number;
  distractedSeconds: number;
  otherSeconds: number;
  topDistraction: string | null;
  adherence: number; // 0..100
}

export interface AccountabilitySettings {
  distractionWarnEnabled: boolean;
  distractionWarnMinutes: number;
  eodPopupEnabled: boolean;
  eodPopupTime: string; // "HH:MM"
  mainGoalDeadline: string; // optional "HH:MM"
}

export interface WeeklyDay {
  day: string;
  weekday: string;
  score: number;
  productiveSeconds: number;
  distractionSeconds: number;
  trackedSeconds: number;
}

/** Week-long roll-up of one check-in: counters sum values, toggles count days. */
export interface CheckinTotal {
  id: string;
  label: string;
  icon: string;
  kind: "toggle" | "counter";
  total: number;
}

export interface WeeklyReview {
  startDay: string;
  endDay: string;
  productiveSeconds: number;
  distractionSeconds: number;
  studySeconds: number;
  checkinTotals: CheckinTotal[];
  bestDay: WeeklyDay | null;
  worstDay: WeeklyDay | null;
  mostCommonLeak: string | null;
  mostCommonLeakSeconds: number;
  days: WeeklyDay[];
}

// ----------------------------------------------------------------- hub sync

export interface SyncStatus {
  mode: string; // local | hub
  connected: boolean;
  lastSync: string | null;
  queued: number;
  hubUrl: string;
  paired: boolean;
}

// ----------------------------------------------------------- daily lock-in plan

export interface LockinPlan {
  day: string;
  mainMission: string;
  secondaryMissions: string[];
  firstBlock: string;
  distractionRule: string;
  focusMode: string;
  avoidTrap: string;
  roastLine: string;
  source: string; // llm | fallback | manual
  edited: boolean;
}

// ------------------------------------------------------------------- streaks

export interface StreakDay {
  day: string;
  met: boolean;
}

export interface Streak {
  id: string;
  name: string;
  kind: string;
  metric: string;
  threshold: number;
  enabled: boolean;
  /** 0 = daily (current/best in days); 1..7 = weekly (current/best in weeks). */
  daysPerWeek: number;
  current: number;
  best: number;
  /** Met days so far in the current ISO week (weekly streaks only). */
  weekMetDays: number;
  lastCompletedDay: string | null;
  calendar: StreakDay[];
}

export interface StreakDefinition {
  id: string;
  name: string;
  kind: string;
  metric: string;
  threshold: number;
  enabled: boolean;
  /** 0 = every day; 1..7 = met on N+ days per week. */
  daysPerWeek: number;
}

// --------------------------------------------------- proof-of-output detection

export type OutputEventType =
  | "video_export"
  | "code_change"
  | "document_created"
  | "download"
  | "study_material"
  | "editing_project_changed"
  | "content_asset"
  | "other";

export interface WatchedFolder {
  id: number;
  path: string;
  label: string;
  project: string | null;
  outputType: OutputEventType;
  enabled: boolean;
  extensions: string[];
  minSizeBytes: number;
  debounceSeconds: number;
  createdAt: string;
}

export interface OutputEvent {
  id: number;
  timestamp: string;
  day: string;
  folderPath: string;
  filePath: string;
  fileName: string;
  extension: string | null;
  fileSize: number;
  eventType: OutputEventType;
  project: string | null;
  linkedBlockKey: string | null;
  linkedLabel: string | null;
  createdAt: string | null;
  modifiedAt: string | null;
}

// ------------------------------------------------------- proof-of-work timeline

export type TimelineSource = "desktop" | "browser" | "screen";

/** A continuous run of activity (adjacent samples merged). */
export interface TimelineBlock {
  source: TimelineSource;
  start: string; // RFC3339
  end: string; // RFC3339
  durationSeconds: number;
  label: string;
  title: string;
  category: string;
  bucket: Bucket;
  project: string | null;
  projectConfidence: number;
  confidence: number; // 0..1
  classifier: "rule" | "llm" | "manual";
  idle: boolean;
  summary: string | null;
  blockKey: string;
  isWeb: boolean;
  sampleCount: number;
  /** Brief intervening activity hidden inside this block in Overview mode. */
  absorbedSeconds: number;
  absorbedCount: number;
  longestProductive: boolean;
  biggestDistraction: boolean;
  firstProductive: boolean;
  goalRelated: boolean;
  outputLinked: boolean;
}

export interface TimelineDay {
  day: string;
  maxGapSeconds: number;
  /** Exact activity blocks, preserving every captured switch. */
  blocks: TimelineBlock[];
  /** Low-noise blocks with brief A → B → A switches absorbed into A. */
  overviewBlocks: TimelineBlock[];
  /** Self-reported check-ins logged for the day (value > 0). */
  outputs: CheckinValue[];
  activeSeconds: number;
  idleSeconds: number;
  productiveSeconds: number;
  distractedSeconds: number;
  firstProductiveStart: string | null;
  goals: string[];
}

/** Payload of the `distraction-warning` event. */
export interface DistractionWarning {
  label: string;
  key: string;
  minutes: number;
  message: string;
}

/** Payload of the `focus-violation` event. */
export interface FocusViolation {
  label: string;
  goal: string | null;
  message: string;
}

export interface ActivityLogEntry {
  source: "app" | "web" | "screen";
  label: string;
  title: string;
  seconds: number;
  category: Category | "ignore";
  activityKind: string;
  reason: string;
  contentType: string | null;
  lastSeen: string;
  detailId: number | null;
  summary: string | null;
  projectName: string | null;
  projectConfidence: number;
  projectSignals: string[];
  classifier: "llm" | "rule" | "manual";
  llmConfidence: number | null;
  confidence: number;
  blockKey: string;
}

export interface DomainRule {
  domain: string;
  category: Category | null;
  captureMode: CaptureMode;
  aiReview: boolean;
}

export interface TrackedDomain {
  domain: string;
  totalSeconds: number;
  category: Category | null;
  captureMode: CaptureMode;
  aiReview: boolean;
}

export interface PrivacySettings {
  capturePageContent: boolean;
  storeRawText: boolean;
  maxTextLength: number;
  deleteRawAfterClassification: boolean;
  smartTrackingEnabled: boolean;
  smartIntervalSeconds: number;
  smartOcrAvailable: boolean;
  ingestPort: number;
  ingestToken: string;
  endpoint: string;
  retentionDays: number;
  idleThresholdSeconds: number;
  countMediaAsActive: boolean;
  trackingPausedUntil: string | null;
  titleExcludedApps: string[];
}

/** An app the tracker has seen (used on the Categories page). */
export interface TrackedApp {
  appName: string;
  totalSeconds: number;
  category: Category | null;
  aiReview: boolean;
}

export interface CategoryRule {
  appName: string;
  category: Category;
}

/** Mocked, hardcoded daily review (no AI / no network for the MVP). */
export interface DailyAiReview {
  date: string;
  verdict: string;
  wins: string[];
  problems: string[];
  tomorrow: string;
  roast: string;
  source: "llm" | "fallback";
  model: string | null;
  generatedAt: string | null;
  notes: string;
}
