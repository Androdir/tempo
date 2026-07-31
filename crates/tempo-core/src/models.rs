use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

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
    (
        "productive",
        "Productive",
        "#16a34a",
        "productive",
        "Deep, focused work",
    ),
    (
        "study",
        "Study",
        "#2563eb",
        "productive",
        "Learning & research",
    ),
    (
        "business",
        "Business",
        "#0d9488",
        "productive",
        "Admin, email, ops",
    ),
    (
        "neutral",
        "Neutral",
        "#64748b",
        "neutral",
        "Necessary or ambiguous; no automatic penalty",
    ),
    (
        "distraction",
        "Distraction",
        "#dc2626",
        "distracting",
        "Off-task time",
    ),
    (
        "recovery",
        "Recovery",
        "#9333ea",
        "neutral",
        "Intentional rest",
    ),
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
    conn.query_row(
        "SELECT 1 FROM category_definitions WHERE id = ?1",
        [id],
        |_| Ok(()),
    )
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

pub fn upsert_category_definition(
    conn: &Connection,
    c: &CategoryDefinition,
) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let built_in = is_valid_category(&c.id);
    let sort: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM category_definitions",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
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
    conn.execute(
        "UPDATE category_rules SET category = ?2 WHERE category = ?1",
        params![&id, &fallback],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE domain_rules SET category = NULL WHERE category = ?1",
        [&id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE projects SET category = ?2 WHERE category = ?1",
        params![&id, &fallback],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE manual_corrections SET category = ?2 WHERE category = ?1",
        params![&id, &fallback],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE smart_activity SET category = ?2 WHERE category = ?1",
        params![&id, &fallback],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE llm_classification SET category = ?2 WHERE category = ?1",
        params![&id, &fallback],
    )
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
pub struct TimeBreakdown {
    pub start_date: String,
    pub end_date: String,
    pub day_count: i64,
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
    #[serde(default)]
    pub target_count: Option<i64>,
    #[serde(default)]
    pub target_unit: Option<String>,
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

/// A user-editable check-in definition ("things the tracker can't see" — unless
/// an auto source is set, in which case the tracker *can* see it and ticks the
/// check-in itself).
///
/// `auto_kind`: `""` = manual only · `"target"` = met after `auto_threshold`
/// active (non-idle) minutes on an app/site matching `auto_metric` ·
/// `"output"` = met after `auto_threshold` detected output files
/// (`auto_metric` = output type or watched-folder label, blank = any output).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinDefinition {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub kind: String, // toggle | counter
    pub built_in: bool,
    #[serde(default)]
    pub auto_kind: String, // "" | target | output
    #[serde(default)]
    pub auto_metric: String,
    #[serde(default)]
    pub auto_threshold: i64,
}

/// A check-in definition together with its effective value for a specific day.
/// Toggles use 0/1; counters use 0..N. For auto check-ins the value comes from
/// detection unless a manual row exists (the manual override always wins).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckinValue {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub kind: String,
    pub value: i64,
    /// This check-in has an auto-detection source configured.
    pub auto: bool,
    /// Raw detected amount today: active minutes (target) or file count (output).
    pub detected: i64,
    /// An auto check-in whose value was manually overridden for this day.
    pub overridden: bool,
}

const CHECKIN_DEFAULTS_SEEDED: &str = "checkin_defaults_seeded";

const DEFAULT_CHECKIN_DEFS: &[(&str, &str, &str, &str)] = &[
    ("videos_posted", "Posted video", "🎬", "counter"),
    ("gym_logged", "Went gym", "🏋️", "toggle"),
    ("wrestled", "Wrestled", "🤼", "toggle"),
    ("studied", "Studied", "📚", "toggle"),
    ("edited_video", "Edited video", "✂️", "toggle"),
    ("analysed_content", "Analysed content", "🔍", "toggle"),
];

pub fn ensure_checkin_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let already = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [CHECKIN_DEFAULTS_SEEDED],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .is_some_and(|v| v == "1");
    if already {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    for (i, (id, label, icon, kind)) in DEFAULT_CHECKIN_DEFS.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO checkin_definitions
               (id, label, icon, kind, built_in, sort_order, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
            params![id, label, icon, kind, i as i64, now],
        )?;
    }
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [CHECKIN_DEFAULTS_SEEDED],
    )?;
    Ok(())
}

pub fn list_checkin_definitions(conn: &Connection) -> rusqlite::Result<Vec<CheckinDefinition>> {
    ensure_checkin_defaults(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, label, icon, kind, built_in, auto_kind, auto_metric, auto_threshold
         FROM checkin_definitions ORDER BY sort_order, label",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(CheckinDefinition {
            id: r.get(0)?,
            label: r.get(1)?,
            icon: r.get(2)?,
            kind: r.get(3)?,
            built_in: r.get::<_, i64>(4)? != 0,
            auto_kind: r.get(5)?,
            auto_metric: r.get(6)?,
            auto_threshold: r.get(7)?,
        })
    })?;
    rows.collect()
}

pub fn checkin_exists(conn: &Connection, id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() {
        return false;
    }
    let _ = ensure_checkin_defaults(conn);
    conn.query_row(
        "SELECT 1 FROM checkin_definitions WHERE id = ?1",
        [id],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn upsert_checkin_definition(conn: &Connection, c: &CheckinDefinition) -> Result<(), String> {
    let id = c.id.trim().to_ascii_lowercase();
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return Err(
            "Check-in id must use lowercase letters, numbers, dashes or underscores".into(),
        );
    }
    if c.label.trim().is_empty() {
        return Err("Check-in label is required".into());
    }
    if !["toggle", "counter"].contains(&c.kind.as_str()) {
        return Err("Check-in kind must be toggle or counter".into());
    }
    if !["", "target", "output"].contains(&c.auto_kind.as_str()) {
        return Err("Auto-detect must be blank, target or output".into());
    }
    let auto_metric = c.auto_metric.trim().to_ascii_lowercase();
    if c.auto_kind == "target" && auto_metric.is_empty() {
        return Err("Enter the app/site name this check-in watches".into());
    }
    // Sensible thresholds when unset: 30 active minutes for app/site time,
    // 1 detected file for outputs.
    let auto_threshold = match c.auto_kind.as_str() {
        "target" => {
            if c.auto_threshold > 0 {
                c.auto_threshold.clamp(1, 1440)
            } else {
                30
            }
        }
        "output" => {
            if c.auto_threshold > 0 {
                c.auto_threshold.clamp(1, 999)
            } else {
                1
            }
        }
        _ => 0,
    };
    let _ = ensure_checkin_defaults(conn);
    let now = chrono::Utc::now().to_rfc3339();
    let icon = if c.icon.trim().is_empty() {
        "✅"
    } else {
        c.icon.trim()
    };
    let sort: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM checkin_definitions",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO checkin_definitions
           (id, label, icon, kind, built_in, sort_order, auto_kind, auto_metric, auto_threshold, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
             label = excluded.label,
             icon = excluded.icon,
             kind = excluded.kind,
             auto_kind = excluded.auto_kind,
             auto_metric = excluded.auto_metric,
             auto_threshold = excluded.auto_threshold,
             updated_at = excluded.updated_at",
        params![id, c.label.trim(), icon, c.kind, c.built_in as i64, sort, c.auto_kind, auto_metric, auto_threshold, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_checkin_definition(conn: &Connection, id: &str) -> Result<(), String> {
    let id = id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return Err("Check-in id is required".into());
    }
    conn.execute("DELETE FROM checkin_definitions WHERE id = ?1", [&id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM checkin_values WHERE checkin_id = ?1", [&id])
        .map_err(|e| e.to_string())?;
    // Streaks and score rules that referenced this check-in simply stop matching;
    // remove them so they don't linger as dead rows.
    conn.execute(
        "DELETE FROM streak_definitions WHERE kind = 'checkin' AND metric = ?1",
        [&id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM score_rules WHERE kind = 'checkin' AND (metric = ?1 OR metric LIKE ?1 || ',%' OR metric LIKE '%,' || ?1)",
        [&id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Raw detected amount for an auto check-in on `day`: active (non-idle) minutes
/// on the matching app/site for `target`, or matching output-event count for
/// `output`. Returns 0 for manual check-ins.
pub fn auto_detected_amount(conn: &Connection, day: &str, def: &CheckinDefinition) -> i64 {
    match def.auto_kind.as_str() {
        "target" => {
            // Any of the comma-separated names may match (substring, lowercased),
            // summed across the desktop and browser lanes. Idle time never counts —
            // leaving the app open while AFK doesn't tick the check-in.
            let mut seconds = 0i64;
            for m in def.auto_metric.split(',') {
                let pat = format!("%{}%", m.trim().to_ascii_lowercase());
                if pat == "%%" {
                    continue;
                }
                seconds += conn
                    .query_row(
                        "SELECT COALESCE(SUM(duration_seconds), 0) FROM activity_log
                         WHERE day = ?1 AND is_idle = 0 AND LOWER(app_name) LIKE ?2",
                        params![day, pat],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(0);
                seconds += conn
                    .query_row(
                        "SELECT COALESCE(SUM(duration_seconds), 0) FROM browser_activity
                         WHERE day = ?1 AND is_idle = 0 AND LOWER(domain) LIKE ?2",
                        params![day, pat],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(0);
            }
            seconds / 60
        }
        "output" => {
            let metric = def.auto_metric.trim().to_ascii_lowercase();
            if metric.is_empty() {
                conn.query_row(
                    "SELECT COUNT(*) FROM output_events WHERE day = ?1",
                    [day],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
            } else {
                // Match the event type exactly, or the watched folder's label.
                conn.query_row(
                    "SELECT COUNT(*) FROM output_events e
                     LEFT JOIN watched_folders w ON w.path = e.folder_path
                     WHERE e.day = ?1 AND (e.event_type = ?2 OR LOWER(w.label) LIKE ?3)",
                    params![day, metric, format!("%{metric}%")],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
            }
        }
        _ => 0,
    }
}

/// Effective auto value from a detected amount: toggles flip at the threshold;
/// counters count files (output) or full threshold-blocks of time (target).
fn auto_value(def: &CheckinDefinition, amount: i64) -> i64 {
    let thr = def.auto_threshold.max(1);
    if def.kind == "counter" {
        if def.auto_kind == "target" {
            amount / thr
        } else {
            amount
        }
    } else {
        (amount >= thr) as i64
    }
}

/// All check-in definitions with their effective value for `day` (0 when not
/// logged). Auto check-ins read live detection unless a manual row overrides.
pub fn checkin_values_for_day(conn: &Connection, day: &str) -> Vec<CheckinValue> {
    let _ = ensure_checkin_defaults(conn);
    let mut stmt = match conn.prepare(
        "SELECT d.id, d.label, d.icon, d.kind, d.built_in, d.auto_kind, d.auto_metric,
                d.auto_threshold, v.value
         FROM checkin_definitions d
         LEFT JOIN checkin_values v ON v.checkin_id = d.id AND v.day = ?1
         ORDER BY d.sort_order, d.label",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([day], |r| {
        let def = CheckinDefinition {
            id: r.get(0)?,
            label: r.get(1)?,
            icon: r.get(2)?,
            kind: r.get(3)?,
            built_in: r.get::<_, i64>(4)? != 0,
            auto_kind: r.get(5)?,
            auto_metric: r.get(6)?,
            auto_threshold: r.get(7)?,
        };
        Ok((def, r.get::<_, Option<i64>>(8)?))
    });
    let pairs: Vec<(CheckinDefinition, Option<i64>)> = match rows {
        Ok(rs) => rs.filter_map(Result::ok).collect(),
        Err(_) => return Vec::new(),
    };
    pairs
        .into_iter()
        .map(|(def, manual)| {
            let auto = !def.auto_kind.is_empty();
            let detected = if auto {
                auto_detected_amount(conn, day, &def)
            } else {
                0
            };
            let value = match manual {
                Some(v) => v,
                None if auto => auto_value(&def, detected),
                None => 0,
            };
            CheckinValue {
                id: def.id,
                label: def.label,
                icon: def.icon,
                kind: def.kind,
                value,
                auto,
                detected,
                overridden: auto && manual.is_some(),
            }
        })
        .collect()
}

/// Check-in values for `day` as an id → value map (only definitions that exist).
pub fn checkin_map_for_day(conn: &Connection, day: &str) -> std::collections::HashMap<String, i64> {
    checkin_values_for_day(conn, day)
        .into_iter()
        .map(|c| (c.id, c.value))
        .collect()
}

/// Set one check-in's value for a day. Toggles clamp to 0/1, counters to 0..999.
pub fn set_checkin_value(conn: &Connection, day: &str, id: &str, value: i64) -> Result<(), String> {
    let _ = ensure_checkin_defaults(conn);
    let id = id.trim().to_ascii_lowercase();
    let kind: String = conn
        .query_row(
            "SELECT kind FROM checkin_definitions WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .map_err(|_| format!("unknown check-in: {id}"))?;
    let v = if kind == "counter" {
        value.clamp(0, 999)
    } else {
        (value != 0) as i64
    };
    conn.execute(
        "INSERT INTO checkin_values (day, checkin_id, value, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(day, checkin_id) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![day, id, v, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove a manual check-in row for a day, so an auto check-in falls back to
/// live detection (and a manual one back to 0).
pub fn clear_checkin_value(conn: &Connection, day: &str, id: &str) -> Result<(), String> {
    let id = id.trim().to_ascii_lowercase();
    conn.execute(
        "DELETE FROM checkin_values WHERE day = ?1 AND checkin_id = ?2",
        params![day, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
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
    pub main_goal_deadline: String,
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

/// Week-long roll-up of one check-in: counters sum values, toggles count days.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CheckinTotal {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub kind: String,
    pub total: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyReview {
    pub start_day: String,
    pub end_day: String,
    pub productive_seconds: i64,
    pub distraction_seconds: i64,
    pub study_seconds: i64,
    pub checkin_totals: Vec<CheckinTotal>,
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
    pub checkins: Vec<CheckinValue>, // today's logged check-ins (value > 0)
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
    pub activity_kind: String,
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
    pub tracking_paused_until: Option<String>,
    pub title_excluded_apps: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionHistoryEntry {
    pub id: i64,
    pub block_key: String,
    pub source: String,
    pub label: String,
    pub title: String,
    pub previous_manual_category: Option<String>,
    pub previous_rule_category: Option<String>,
    pub new_category: String,
    pub created_at: String,
    pub undone_at: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingHealth {
    pub status: String,
    pub checked_at: String,
    pub database_ok: bool,
    pub last_desktop_at: Option<String>,
    pub last_browser_at: Option<String>,
    pub last_screen_at: Option<String>,
    pub browser_connected: bool,
    pub smart_enabled: bool,
    pub pending_sync_events: i64,
    pub last_backup_at: Option<String>,
    pub issues: Vec<String>,
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
    /// 0 = daily streak (current/best in days); 1..7 = weekly (current/best in weeks).
    pub days_per_week: i64,
    pub current: i64,
    pub best: i64,
    /// Met days so far in the current ISO week (weekly streaks only).
    pub week_met_days: i64,
    pub last_completed_day: Option<String>,
    pub calendar: Vec<StreakDay>, // oldest → newest (today last)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreakDefinition {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub metric: String,
    pub threshold: i64,
    pub enabled: bool,
    pub days_per_week: i64,
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
    pub confidence: f64,    // 0.0–1.0
    pub classifier: String, // "rule" | "llm" | "manual"
    pub idle: bool,
    pub summary: Option<String>,
    pub block_key: String,
    pub is_web: bool,
    pub sample_count: i64,
    /// Seconds of tiny intervening switches absorbed into this block in overview mode.
    pub absorbed_seconds: i64,
    /// Number of intervening blocks absorbed in overview mode.
    pub absorbed_count: i64,
    // Highlight flags (computed across the day).
    pub longest_productive: bool,
    pub biggest_distraction: bool,
    pub first_productive: bool,
    pub goal_related: bool,
    pub output_linked: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDay {
    pub day: String,
    pub max_gap_seconds: i64,
    /// Exact merged samples, preserving every app/site switch.
    pub blocks: Vec<TimelineBlock>,
    /// Meaningful runs with brief A → B → A switches absorbed into A.
    pub overview_blocks: Vec<TimelineBlock>,
    /// Self-reported check-ins for the day (shown as a strip; no per-event times).
    pub outputs: Vec<CheckinValue>,
    pub active_seconds: i64,
    pub idle_seconds: i64,
    pub productive_seconds: i64,
    pub distracted_seconds: i64,
    pub first_productive_start: Option<String>,
    pub goals: Vec<String>, // goal titles for the day (context)
}
