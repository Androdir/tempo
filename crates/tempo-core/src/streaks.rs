//! Consistency streaks for meaningful behaviours (not just app usage).
//!
//! The DB orchestration (building `DayMetrics` from activity/check-ins/outputs)
//! lives in `commands.rs`; this module holds the editable definitions, the pure
//! per-day "was it met?" evaluation, and the streak-run maths — all unit-tested.

use std::collections::HashMap;

use rusqlite::Connection;

/// Everything needed to decide whether each streak was met on a given day.
#[derive(Default, Clone)]
pub struct DayMetrics {
    pub checkins: HashMap<String, i64>, // check-in id → value (toggles 0/1, counters 0..N)
    pub main_goal_completed: bool,
    pub video_exports: i64,
    pub editing_changes: i64,
    pub cat_minutes: HashMap<String, i64>, // study | business | productive | ...
    pub max_productive_block_min: i64,
    pub max_distraction_block_min: i64,
    pub tracked_min: i64,
}

impl DayMetrics {
    pub fn cat_min(&self, c: &str) -> i64 {
        self.cat_minutes.get(c).copied().unwrap_or(0)
    }
    pub fn checkin(&self, id: &str) -> i64 {
        self.checkins.get(id).copied().unwrap_or(0)
    }
}

#[derive(Clone)]
pub struct StreakDef {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub metric: String,
    pub threshold: i64,
    pub enabled: bool,
    pub best_streak: i64,
    pub last_completed_day: Option<String>,
}

/// (id, name, kind, metric, threshold) for the seeded defaults.
pub const DEFAULTS: &[(&str, &str, &str, &str, i64)] = &[
    ("posted_video", "Posted a video", "output", "video_export", 0),
    ("main_goal", "Completed main goal", "goal", "main_goal", 0),
    ("coding_60", "60+ min coding / building", "category", "productive", 60),
    ("business_90", "90+ min business work", "category", "business", 90),
    ("study_60", "60+ min studying", "category", "study", 60),
    ("studied", "Studied", "checkin", "studied", 0),
    ("edited_video", "Edited a video", "checkin", "edited_video", 0),
    ("analysed_content", "Analysed content", "checkin", "analysed_content", 0),
    ("gym", "Gym", "checkin", "gym_logged", 0),
    ("wrestling", "Wrestling", "checkin", "wrestled", 0),
    ("productive_block_60", "60+ min focus block", "block", "productive", 60),
    ("no_major_distraction", "No major distraction", "distraction", "max_block", 30),
];

/// Whether a streak's condition was met on a day with the given metrics.
/// Everything is driven by (kind, metric, threshold); the two seeded streaks
/// with hybrid conditions (check-in OR detected output) keep their id-based
/// special case.
pub fn streak_met(def: &StreakDef, m: &DayMetrics) -> bool {
    let thr = def.threshold.max(1);
    match def.id.as_str() {
        "posted_video" => m.checkin("videos_posted") > 0 || m.video_exports > 0,
        "edited_video" => m.checkin("edited_video") > 0 || m.editing_changes > 0,
        _ => match def.kind.as_str() {
            "checkin" => def
                .metric
                .split(',')
                .any(|id| m.checkin(id.trim()) >= thr),
            "goal" => m.main_goal_completed,
            "category" => m.cat_min(&def.metric) >= thr,
            "output" => match def.metric.as_str() {
                "video_export" => m.video_exports >= thr,
                "editing_project_changed" => m.editing_changes >= thr,
                _ => false,
            },
            "block" => m.max_productive_block_min >= thr,
            "distraction" => m.tracked_min > 0 && m.max_distraction_block_min < thr,
            _ => false,
        },
    }
}

/// Current run: consecutive met days ending today (or yesterday, if today is
/// still in progress and not yet met). `status` is oldest→newest (today last).
pub fn current_run(status: &[bool]) -> i64 {
    if status.is_empty() {
        return 0;
    }
    let mut i = status.len() - 1;
    if !status[i] {
        if i == 0 {
            return 0;
        }
        i -= 1; // today not met yet → don't break the streak, start from yesterday
    }
    let mut c = 0i64;
    loop {
        if status[i] {
            c += 1;
        } else {
            break;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    c
}

/// Longest run of consecutive met days within the window.
pub fn best_run(status: &[bool]) -> i64 {
    let mut best = 0i64;
    let mut cur = 0i64;
    for &s in status {
        if s {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// Seed the default streaks when none exist yet. Production starts empty (you
/// create the streaks you actually want); this is used by tests and by the
/// explicit "Add suggested streaks" action.
pub fn ensure_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM streak_definitions", [], |r| r.get(0)).unwrap_or(0);
    if count > 0 {
        return Ok(());
    }
    seed_suggested(conn)?;
    Ok(())
}

/// Insert every suggested streak that isn't already present. Returns how many
/// were added. Backs the "Add suggested streaks" button.
pub fn seed_suggested(conn: &Connection) -> rusqlite::Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut added = 0i64;
    for (i, (id, name, kind, metric, threshold)) in DEFAULTS.iter().enumerate() {
        added += conn.execute(
            "INSERT OR IGNORE INTO streak_definitions
               (id, name, kind, metric, threshold, enabled, sort_order, best_streak, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0, ?7)",
            rusqlite::params![id, name, kind, metric, threshold, i as i64, now],
        )? as i64;
    }
    Ok(added)
}

pub const STREAK_KINDS: [&str; 6] = ["checkin", "goal", "category", "output", "block", "distraction"];

/// Create (or overwrite) a streak definition, validating the metric against the
/// live check-in / category definitions.
pub fn add_definition(
    conn: &Connection,
    id: &str,
    name: &str,
    kind: &str,
    metric: &str,
    threshold: i64,
) -> Result<(), String> {
    let id = id.trim().to_ascii_lowercase();
    if id.is_empty()
        || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err("Streak id must use lowercase letters, numbers, dashes or underscores".into());
    }
    if name.trim().is_empty() {
        return Err("Streak name is required".into());
    }
    if !STREAK_KINDS.contains(&kind) {
        return Err(format!("Unknown streak kind: {kind}"));
    }
    let metric = metric.trim().to_ascii_lowercase();
    match kind {
        "checkin" => {
            if metric.is_empty() {
                return Err("Pick which check-in this streak tracks".into());
            }
            for m in metric.split(',') {
                if !crate::models::checkin_exists(conn, m.trim()) {
                    return Err(format!("Unknown check-in: {}", m.trim()));
                }
            }
        }
        "category" => {
            if !crate::models::category_exists(conn, &metric) {
                return Err(format!("Unknown category: {metric}"));
            }
        }
        "output" => {
            if !["video_export", "editing_project_changed"].contains(&metric.as_str()) {
                return Err("Output metric must be video_export or editing_project_changed".into());
            }
        }
        _ => {} // goal / block / distraction need no metric
    }
    let now = chrono::Utc::now().to_rfc3339();
    let sort: i64 = conn
        .query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM streak_definitions", [], |r| r.get(0))
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO streak_definitions
           (id, name, kind, metric, threshold, enabled, sort_order, best_streak, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0, ?7)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name, kind = excluded.kind, metric = excluded.metric,
             threshold = excluded.threshold, updated_at = excluded.updated_at",
        rusqlite::params![id, name.trim(), kind, metric, threshold.clamp(0, 100_000), sort, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_definition(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM streak_definitions WHERE id = ?1", [id.trim()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_defs(conn: &Connection) -> Vec<StreakDef> {
    let mut stmt = match conn.prepare(
        "SELECT id, name, kind, metric, threshold, enabled, best_streak, last_completed_day
         FROM streak_definitions ORDER BY sort_order, id",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |r| {
        Ok(StreakDef {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            metric: r.get(3)?,
            threshold: r.get(4)?,
            enabled: r.get::<_, i64>(5)? != 0,
            best_streak: r.get(6)?,
            last_completed_day: r.get(7)?,
        })
    });
    match rows {
        Ok(rs) => rs.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str, kind: &str, metric: &str, threshold: i64) -> StreakDef {
        StreakDef {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
            metric: metric.into(),
            threshold,
            enabled: true,
            best_streak: 0,
            last_completed_day: None,
        }
    }

    #[test]
    fn current_run_handles_today_and_misses() {
        assert_eq!(current_run(&[true, true, true]), 3);
        assert_eq!(current_run(&[true, false, true]), 1); // today met, yesterday missed
        assert_eq!(current_run(&[true, true, false]), 2); // today pending → counts to yesterday
        assert_eq!(current_run(&[true, false, false]), 0); // missed yesterday & today
        assert_eq!(current_run(&[false]), 0);
        assert_eq!(current_run(&[]), 0);
    }

    #[test]
    fn best_run_is_longest_window_run() {
        assert_eq!(best_run(&[true, true, false, true, true, true]), 3);
        assert_eq!(best_run(&[false, false]), 0);
        assert_eq!(best_run(&[true]), 1);
    }

    #[test]
    fn threshold_based_streak_uses_threshold() {
        let mut m = DayMetrics::default();
        m.cat_minutes.insert("study".into(), 70);
        assert!(streak_met(&def("study_60", "category", "study", 60), &m));
        m.cat_minutes.insert("study".into(), 50);
        assert!(!streak_met(&def("study_60", "category", "study", 60), &m));
        // editable threshold: lowering it to 40 makes 50 minutes pass
        assert!(streak_met(&def("study_60", "category", "study", 40), &m));
    }

    #[test]
    fn posted_video_met_by_checkin_or_output() {
        let d = def("posted_video", "output", "video_export", 0);
        let mut m = DayMetrics::default();
        assert!(!streak_met(&d, &m));
        m.video_exports = 1; // detected export alone satisfies it
        assert!(streak_met(&d, &m));
        let mut m2 = DayMetrics::default();
        m2.checkins.insert("videos_posted".into(), 2); // or the manual check-in
        assert!(streak_met(&d, &m2));
    }

    #[test]
    fn custom_checkin_streak_reads_metric() {
        let d = def("growth", "checkin", "growth_research", 0);
        let mut m = DayMetrics::default();
        assert!(!streak_met(&d, &m));
        m.checkins.insert("growth_research".into(), 1);
        assert!(streak_met(&d, &m));
        // counter check-in with a threshold: 2+ videos
        let d2 = def("two_videos", "checkin", "videos_posted", 2);
        let mut m2 = DayMetrics::default();
        m2.checkins.insert("videos_posted".into(), 1);
        assert!(!streak_met(&d2, &m2));
        m2.checkins.insert("videos_posted".into(), 2);
        assert!(streak_met(&d2, &m2));
    }

    #[test]
    fn no_major_distraction_needs_tracked_time() {
        let d = def("no_major_distraction", "distraction", "max_block", 30);
        let mut m = DayMetrics::default();
        assert!(!streak_met(&d, &m)); // no tracked time → not a clean day, just empty
        m.tracked_min = 200;
        m.max_distraction_block_min = 12;
        assert!(streak_met(&d, &m));
        m.max_distraction_block_min = 45;
        assert!(!streak_met(&d, &m));
    }
}
