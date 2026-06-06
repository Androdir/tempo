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
    pub videos_posted: i64,
    pub gym_logged: bool,
    pub wrestled: bool,
    pub studied: bool,
    pub edited_video: bool,
    pub analysed_content: bool,
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
pub fn streak_met(def: &StreakDef, m: &DayMetrics) -> bool {
    let thr = def.threshold.max(1);
    match def.id.as_str() {
        "posted_video" => m.videos_posted > 0 || m.video_exports > 0,
        "main_goal" => m.main_goal_completed,
        "studied" => m.studied,
        "edited_video" => m.edited_video || m.editing_changes > 0,
        "analysed_content" => m.analysed_content,
        "gym" => m.gym_logged,
        "wrestling" => m.wrestled,
        "coding_60" => m.cat_min("productive") >= thr,
        "business_90" => m.cat_min("business") >= thr,
        "study_60" => m.cat_min("study") >= thr,
        "productive_block_60" => m.max_productive_block_min >= thr,
        "no_major_distraction" => m.tracked_min > 0 && m.max_distraction_block_min < thr,
        // Custom streaks fall back to their kind/metric.
        _ => match def.kind.as_str() {
            "checkin" => match def.metric.as_str() {
                "studied" => m.studied,
                "edited_video" => m.edited_video,
                "analysed_content" => m.analysed_content,
                "gym_logged" => m.gym_logged,
                "wrestled" => m.wrestled,
                "videos_posted" => m.videos_posted > 0,
                _ => false,
            },
            "goal" => m.main_goal_completed,
            "category" => m.cat_min(&def.metric) >= thr,
            "output" => {
                if def.metric == "video_export" {
                    m.video_exports > 0
                } else {
                    false
                }
            }
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

pub fn ensure_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM streak_definitions", [], |r| r.get(0)).unwrap_or(0);
    if count > 0 {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    for (i, (id, name, kind, metric, threshold)) in DEFAULTS.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO streak_definitions
               (id, name, kind, metric, threshold, enabled, sort_order, best_streak, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0, ?7)",
            rusqlite::params![id, name, kind, metric, threshold, i as i64, now],
        )?;
    }
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
        m2.videos_posted = 2; // or the manual check-in
        assert!(streak_met(&d, &m2));
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
