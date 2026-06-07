use serde::{Deserialize, Serialize};
use rusqlite::{params, Connection};

const CATEGORY_DEFAULTS_SEEDED: &str = "category_defaults_seeded";

/// Built-in starter categories. Users can edit these and add their own rows in
/// `category_definitions`.
pub const CATEGORIES: [&str; 6] = [
    "productive",
    "study",
    "business",
    "neutral",
    "distraction",
    "recovery",
];

pub fn is_valid_category(c: &str) -> bool {
    CATEGORIES.contains(&c)
}

/// Roll a fine-grained category up into one of the three dashboard buckets.
/// Fallback bucketing for older/static paths. User-edited category buckets come
/// from `category_definitions` where a database connection is available.
pub fn bucket_for(category: &str) -> &'static str {
    match category {
        "productive" | "study" | "business" => "productive",
        "distraction" => "distracting",
        // neutral, recovery, and anything uncategorized
        _ => "neutral",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryDefinition {
    pub id: String,
    pub label: String,
    pub color: String,
    pub bucket: String,
    pub blurb: String,
    pub built_in: bool,
}

const DEFAULT_CATEGORY_DEFS: &[(&str, &str, &str, &str, &str)] = &[
    ("productive", "Productive", "#16a34a", "productive", "Deep, focused work"),
    ("study", "Study", "#2563eb", "productive", "Learning & research"),
    ("business", "Business", "#0d9488", "productive", "Admin, email, ops"),
    ("neutral", "Neutral", "#64748b", "neutral", "Necessary but neutral"),
    ("distraction", "Distraction", "#dc2626", "distracting", "Off-task time"),
    ("recovery", "Recovery", "#9333ea", "neutral", "Intentional rest"),
];

pub fn ensure_category_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let already_seeded = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [CATEGORY_DEFAULTS_SEEDED],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .is_some_and(|v| v == "1");
    if already_seeded {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    for (i, (id, label, color, bucket, blurb)) in DEFAULT_CATEGORY_DEFS.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO category_definitions
               (id, label, color, bucket, blurb, built_in, sort_order, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
            params![id, label, color, bucket, blurb, i as i64, now],
        )?;
    }
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [CATEGORY_DEFAULTS_SEEDED],
    )?;
    Ok(())
}

pub fn category_exists(conn: &Connection, id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() {
        return false;
    }
    let _ = ensure_category_defaults(conn);
    conn.query_row("SELECT 1 FROM category_definitions WHERE id = ?1", [id], |_| Ok(()))
        .is_ok()
}

pub fn fallback_category(conn: &Connection, excluded: Option<&str>) -> Option<String> {
    let _ = ensure_category_defaults(conn);
    let excluded = excluded.unwrap_or("");
    conn.query_row(
        "SELECT id FROM category_definitions
         WHERE id != ?1
         ORDER BY CASE bucket WHEN 'neutral' THEN 0 WHEN 'productive' THEN 1 ELSE 2 END,
                  sort_order, label
         LIMIT 1",
        [excluded],
        |r| r.get(0),
    )
    .ok()
}

pub fn category_or_fallback(conn: &Connection, category: &str) -> String {
    if category_exists(conn, category) {
        category.to_string()
    } else {
        fallback_category(conn, None).unwrap_or_else(|| "uncategorized".to_string())
    }
}

pub fn list_category_definitions(conn: &Connection) -> rusqlite::Result<Vec<CategoryDefinition>> {
    ensure_category_defaults(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, label, color, bucket, blurb, built_in
         FROM category_definitions ORDER BY sort_order, label",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(CategoryDefinition {
            id: r.get(0)?,
            label: r.get(1)?,
            color: r.get(2)?,
            bucket: r.get(3)?,
            blurb: r.get(4)?,
            built_in: r.get::<_, i64>(5)? != 0,
        })
    })?;
    rows.collect()
}

pub fn upsert_category_definition(conn: &Connection, c: &CategoryDefinition) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let built_in = is_valid_category(&c.id);
    let sort: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM category_definitions",
        [],
        |r| r.get(0),
    ).unwrap_or(0);
    conn.execute(
        "INSERT INTO category_definitions (id, label, color, bucket, blurb, built_in, sort_order, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
             label = excluded.label,
             color = excluded.color,
             bucket = excluded.bucket,
             blurb = excluded.blurb,
             updated_at = excluded.updated_at",
        params![
            c.id,
            c.label,
            c.color,
            c.bucket,
            c.blurb,
            built_in as i64,
            sort,
            now
        ],
    )?;
    Ok(())
}

pub fn delete_category_definition(conn: &Connection, id: &str) -> Result<(), String> {
    let id = id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return Err("Category id is required".into());
    }
    if !category_exists(conn, &id) {
        return Ok(());
    }

    let fallback = fallback_category(conn, Some(&id));
    let Some(fallback) = fallback else {
        return Err("At least one category must remain".into());
    };

    conn.execute("DELETE FROM category_definitions WHERE id = ?1", [&id])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE category_rules SET category = ?2 WHERE category = ?1", params![&id, &fallback])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE domain_rules SET category = NULL WHERE category = ?1", [&id])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE projects SET category = ?2 WHERE category = ?1", params![&id, &fallback])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE manual_corrections SET category = ?2 WHERE category = ?1", params![&id, &fallback])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE smart_activity SET category = ?2 WHERE category = ?1", params![&id, &fallback])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE llm_classification SET category = ?2 WHERE category = ?1", params![&id, &fallback])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    pub app_name: String,
    pub seconds: i64,
    pub category: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryUsage {
    pub category: String,
    pub seconds: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketUsage {
    pub bucket: String,
    pub seconds: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodaySummary {
    pub date: String,
    pub total_active_seconds: i64,
    pub total_idle_seconds: i64,
    pub total_browser_seconds: i64,
    pub per_app: Vec<AppUsage>,
    pub per_website: Vec<WebsiteUsage>,
    pub per_category: Vec<CategoryUsage>,
    pub per_bucket: Vec<BucketUsage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedApp {
    pub app_name: String,
    pub total_seconds: i64,
    pub category: Option<String>,
    pub ai_review: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryRule {
    pub app_name: String,
    pub category: String,
}

/// Daily AI review generated by the local LLM (or a local fallback).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DailyAiReview {
    pub date: String,
    pub verdict: String,
    pub wins: Vec<String>,
    pub problems: Vec<String>,
    pub tomorrow: String,
    pub roast: String,
    pub source: String, // "llm" | "fallback"
    pub model: Option<String>,
    pub generated_at: Option<String>,
    pub notes: String,
}

// ---------------------------------------------------------- goals & check-ins

/// A user-defined daily goal ("main mission"). Used for input (add/update) and
/// output (get); `id` is ignored on create and populated on read.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    #[serde(default)]
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub target_minutes: Option<i64>,
    #[serde(default = "default_priority")]
    pub priority: String, // low | medium | high
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub recurring: bool,
}

fn default_priority() -> String {
    "medium".to_string()
}

/// Current state of the quick daily check-in buttons.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CheckinState {
    pub videos_posted: i64,
    pub gym_logged: bool,
    pub wrestled: bool,
    pub studied: bool,
    pub edited_video: bool,
    pub analysed_content: bool,
}

// ------------------------------------------------------------ accountability

/// A focus session ("focus mode"). Enforcement is soft (warnings only).
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FocusSession {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub started_at: String,
    pub duration_minutes: i64,
    #[serde(default)]
    pub ends_at: String,
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
    #[serde(default)]
    pub status: String, // active | completed | ended
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub remaining_seconds: i64, // seconds until ends_at (0 if not active)
}

/// How a finished focus session actually went.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FocusSummary {
    pub goal: Option<String>,
    pub duration_minutes: i64,
    pub status: String,
    pub focused_seconds: i64,
    pub distracted_seconds: i64,
    pub other_seconds: i64,
    pub top_distraction: Option<String>,
    pub adherence: i64, // 0..100 share of tracked time spent focused
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountabilitySettings {
    pub distraction_warn_enabled: bool,
    pub distraction_warn_minutes: i64,
    pub eod_popup_enabled: bool,
    pub eod_popup_time: String,
}

/// One day's roll-up inside the weekly review.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyDay {
    pub day: String,     // YYYY-MM-DD
    pub weekday: String, // Mon, Tue, ...
    pub score: i64,
    pub productive_seconds: i64,
    pub distraction_seconds: i64,
    pub tracked_seconds: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyReview {
    pub start_day: String,
    pub end_day: String,
    pub productive_seconds: i64,
    pub distraction_seconds: i64,
    pub study_seconds: i64,
    pub videos_posted: i64,
    pub best_day: Option<WeeklyDay>,
    pub worst_day: Option<WeeklyDay>,
    pub most_common_leak: Option<String>,
    pub most_common_leak_seconds: i64,
    pub days: Vec<WeeklyDay>,
}

// ---------------------------------------------------------- browser tracking

/// Incoming record from the Chrome extension (POST /ingest).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestPayload {
    pub domain: String,
    pub url: String,
    #[serde(default)]
    pub page_title: String,
    pub timestamp: Option<String>,
    pub duration_seconds: Option<i64>,
    pub is_idle: Option<bool>,
    pub content_type: Option<String>,
    pub raw_text_excerpt: Option<String>,
    pub content_summary: Option<String>,
    pub detected_keywords: Option<Vec<String>>,
}

/// Capture policy handed to the extension (GET /config).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDto {
    pub capture_page_content: bool,
    pub store_raw_text: bool,
    pub max_text_length: i64,
    pub sample_seconds: i64,
    pub idle_seconds: i64,
    pub domain_rules: Vec<DomainRule>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainRule {
    pub domain: String,
    pub category: Option<String>,
    pub capture_mode: String,
    pub ai_review: bool,
}

/// Per-domain aggregate for the dashboard / browser page.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebsiteUsage {
    pub domain: String,
    pub seconds: i64,
    pub category: Option<String>,
    pub page_views: i64,
}

/// One browser page visit row (list item, clickable for details).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPage {
    pub id: i64,
    pub timestamp: String,
    pub domain: String,
    pub url: String,
    pub page_title: String,
    pub duration_seconds: i64,
    pub content_type: Option<String>,
    pub category: String,
    pub content_summary: Option<String>,
    pub detected_keywords: Vec<String>,
    pub has_raw: bool,
    pub is_idle: bool,
    pub project_name: Option<String>,
    pub project_confidence: u8,
    pub project_signals: Vec<String>,
}

/// Full detail for the Activity Details drawer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDetail {
    pub id: i64,
    pub timestamp: String,
    pub domain: String,
    pub url: String,
    pub page_title: String,
    pub duration_seconds: i64,
    pub content_type: Option<String>,
    pub category: String,
    pub classification_reason: String,
    pub content_summary: Option<String>,
    pub detected_keywords: Vec<String>,
    pub raw_text_excerpt: Option<String>,
    pub content_capture_enabled: bool,
    pub is_idle: bool,
    pub project_name: Option<String>,
    pub project_confidence: u8,
    pub project_signals: Vec<String>,
    pub classifier: String, // "llm" | "rule" | "manual"
    pub llm_confidence: Option<f64>,
    pub confidence: f64, // rule-layer confidence, 0.0–1.0
    pub block_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettings {
    pub enabled: bool,
    pub url: String,
    pub model: String,
    pub last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaTestResult {
    pub ok: bool,
    pub message: String,
    pub models: Vec<String>,
    pub model_available: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmError {
    pub timestamp: String,
    pub context: Option<String>,
    pub message: String,
}

// ----------------------------------------------------------- daily score

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScoreLine {
    pub id: String,
    pub label: String,
    pub weight: i64,
    pub threshold: Option<i64>,
    pub has_threshold: bool,
    pub positive: bool,
    pub triggered: bool,
    pub value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryMinutes {
    pub category: String,
    pub minutes: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    pub date: String,
    pub score: i64,
    pub verdict: String,
    pub top_wins: Vec<ScoreLine>,
    pub biggest_leaks: Vec<ScoreLine>,
    pub suggestion: String,
    pub lines: Vec<ScoreLine>,
    pub category_minutes: Vec<CategoryMinutes>,
    pub main_goal_completed: bool,
    pub videos_posted: i64,
    pub gym_logged: bool,
    pub main_goal_name: Option<String>,
}

/// One row in the unified Activity Log (desktop app or browser page).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityLogEntry {
    pub source: String, // "app" | "web"
    pub label: String,  // app name or domain
    pub title: String,  // window or page title
    pub seconds: i64,
    pub category: String,
    pub reason: String,
    pub content_type: Option<String>,
    pub last_seen: String,
    pub detail_id: Option<i64>, // browser_activity id (web rows only)
    pub summary: Option<String>,
    pub project_name: Option<String>,
    pub project_confidence: u8,
    pub project_signals: Vec<String>,
    pub classifier: String, // "llm" | "rule" | "manual"
    pub llm_confidence: Option<f64>,
    pub confidence: f64, // rule-layer confidence, 0.0–1.0
    pub block_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActivityView {
    pub date: String,
    pub per_domain: Vec<WebsiteUsage>,
    pub recent_pages: Vec<BrowserPage>,
}

/// A domain the extension has reported, for the "categorize domains" view.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedDomain {
    pub domain: String,
    pub total_seconds: i64,
    pub category: Option<String>,
    pub capture_mode: String,
    pub ai_review: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    pub capture_page_content: bool,
    pub store_raw_text: bool,
    pub max_text_length: i64,
    pub delete_raw_after_classification: bool,
    pub smart_tracking_enabled: bool,
    pub smart_interval_seconds: i64,
    pub smart_ocr_available: bool,
    pub ingest_port: u16,
    pub ingest_token: String,
    pub endpoint: String,
    pub retention_days: i64,
    pub idle_threshold_seconds: i64,
    pub count_media_as_active: bool,
}

// --------------------------------------------------- proof-of-output detection

fn default_true() -> bool {
    true
}
fn default_debounce() -> i64 {
    5
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WatchedFolder {
    #[serde(default)]
    pub id: i64,
    pub path: String,
    pub label: String,
    #[serde(default)]
    pub project: Option<String>,
    pub output_type: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub extensions: Vec<String>, // lowercased, no dot; [] = any
    #[serde(default)]
    pub min_size_bytes: i64,
    #[serde(default = "default_debounce")]
    pub debounce_seconds: i64,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OutputEvent {
    pub id: i64,
    pub timestamp: String,
    pub day: String,
    pub folder_path: String,
    pub file_path: String,
    pub file_name: String,
    pub extension: Option<String>,
    pub file_size: i64,
    pub event_type: String,
    pub project: Option<String>,
    pub linked_block_key: Option<String>,
    pub linked_label: Option<String>,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
}

// ----------------------------------------------------------- daily lock-in plan

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LockinPlan {
    pub day: String, // the day it was generated FROM (it plans the next day)
    pub main_mission: String,
    pub secondary_missions: Vec<String>,
    pub first_block: String,
    pub distraction_rule: String,
    pub focus_mode: String,
    pub avoid_trap: String,
    pub roast_line: String,
    #[serde(default)]
    pub source: String, // llm | fallback | manual
    #[serde(default)]
    pub edited: bool,
}

// ------------------------------------------------------------------- streaks

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreakDay {
    pub day: String,
    pub met: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Streak {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub metric: String,
    pub threshold: i64,
    pub enabled: bool,
    pub current: i64,
    pub best: i64,
    pub last_completed_day: Option<String>,
    pub calendar: Vec<StreakDay>, // oldest → newest (today last)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreakDefinition {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub threshold: i64,
    pub enabled: bool,
}

// ------------------------------------------------------- proof-of-work timeline

/// One continuous run of activity (adjacent samples merged) for the Timeline.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBlock {
    pub source: String, // "desktop" | "browser" | "screen"
    pub start: String,  // RFC3339 (UTC) of the first sample
    pub end: String,    // RFC3339 (UTC) of the last sample + its duration
    pub duration_seconds: i64,
    pub label: String, // app name or domain
    pub title: String, // most recent window/page title in the run
    pub category: String,
    pub bucket: String, // productive | neutral | distracting
    pub project: Option<String>,
    pub project_confidence: u8,
    pub confidence: f64, // 0.0–1.0
    pub classifier: String, // "rule" | "llm" | "manual"
    pub idle: bool,
    pub summary: Option<String>,
    pub block_key: String,
    pub is_web: bool,
    pub sample_count: i64,
    // Highlight flags (computed across the day).
    pub longest_productive: bool,
    pub biggest_distraction: bool,
    pub first_productive: bool,
    pub goal_related: bool,
    pub output_linked: bool,
}

/// Self-reported outputs for the day (shown as a strip; no per-event times).
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimelineOutputs {
    pub videos_posted: i64,
    pub gym_logged: bool,
    pub wrestled: bool,
    pub studied: bool,
    pub edited_video: bool,
    pub analysed_content: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDay {
    pub day: String,
    pub max_gap_seconds: i64,
    pub blocks: Vec<TimelineBlock>,
    pub outputs: TimelineOutputs,
    pub active_seconds: i64,
    pub idle_seconds: i64,
    pub productive_seconds: i64,
    pub distracted_seconds: i64,
    pub first_productive_start: Option<String>,
    pub goals: Vec<String>, // goal titles for the day (context)
}
