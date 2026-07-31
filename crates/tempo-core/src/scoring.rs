//! Daily productivity score (0–100) from a user-editable, DB-backed rule set.
//!
//! Each rule reads one signal kind:
//! - `checkin`    — a self-reported check-in (metric = check-in id; comma-separated
//!                  ids mean "any of these"). Threshold = required count (default 1).
//! - `category`   — minutes in a category (metric = category id, threshold = minutes).
//! - `target`     — minutes on a specific app/site (metric matched against the
//!                  block label/domain, threshold = minutes). Usually negative.
//! - `goal`       — main daily goal completed (no metric/threshold).
//! - `no_goal`    — main daily goal NOT completed (penalty; no metric/threshold).
//! - `late_start` — first productive block after threshold o'clock (penalty).
//! - `output`     — detected proof-of-output events (metric = output type).
//!
//! Positive weights trigger when value >= threshold; negative weights trigger when
//! value > threshold (a leak you went over). Everything is stored in `score_rules`
//! and editable from the Daily Score page.

use std::collections::HashMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::models::{CategoryMinutes, CheckinValue, ScoreLine, ScoreReport};

const RULES_SEEDED: &str = "score_rules_seeded";

pub struct Stats {
    pub cat_seconds: HashMap<String, i64>,
    /// Seconds per `target`-rule metric (matched against block label/domain).
    pub target_seconds: HashMap<String, i64>,
    pub first_productive_min: Option<i64>, // minutes since local midnight
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            cat_seconds: HashMap::new(),
            target_seconds: HashMap::new(),
            first_productive_min: None,
        }
    }
}

pub struct Checkins {
    pub main_goal_completed: bool,
    pub values: HashMap<String, i64>, // check-in id → value for the day
}

/// Detected proof-of-output counts for the day (folder watcher).
#[derive(Default)]
pub struct OutputSignals {
    pub video_exports: i64,
    pub code_changes: i64,
    pub study_materials: i64,
}

pub const RULE_KINDS: [&str; 7] = [
    "checkin",
    "category",
    "target",
    "goal",
    "no_goal",
    "late_start",
    "output",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreRule {
    pub id: String,
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub metric: String,
    pub weight: i64,
    #[serde(default)]
    pub threshold: Option<i64>,
    #[serde(default)]
    pub built_in: bool,
}

/// Tempo starts with only universal goal rules. Everything more personal should
/// come from the user's own projects, check-ins, apps/sites, or output watchers.
const DEFAULT_RULES: &[(&str, &str, &str, &str, i64, Option<i64>)] = &[
    (
        "main_goal",
        "Completed main daily goal",
        "goal",
        "",
        30,
        None,
    ),
    (
        "no_main_goal",
        "No main goal completed",
        "no_goal",
        "",
        -25,
        None,
    ),
];

/// Generic rules shipped by older versions. The v2 migration removes a row only
/// when it is still an untouched built-in; edited rules and all custom rules stay.
const LEGACY_PERSONAL_DEFAULTS: &[(&str, &str, &str, &str, i64, Option<i64>)] = &[
    (
        "posted_video",
        "Posted 1+ videos",
        "checkin",
        "videos_posted",
        25,
        Some(1),
    ),
    (
        "business_min",
        "90+ min editing / business work",
        "category",
        "business",
        20,
        Some(90),
    ),
    (
        "study_min",
        "60+ min studying",
        "category",
        "study",
        15,
        Some(60),
    ),
    (
        "coding_min",
        "60+ min coding / building",
        "category",
        "productive",
        15,
        Some(60),
    ),
    (
        "gym",
        "Gym / wrestling logged",
        "checkin",
        "gym_logged,wrestled",
        10,
        Some(1),
    ),
    (
        "instagram",
        "Instagram distraction over 30 min",
        "target",
        "instagram",
        -15,
        Some(30),
    ),
    (
        "youtube",
        "YouTube distraction over 45 min",
        "target",
        "youtube",
        -10,
        Some(45),
    ),
    (
        "recovery",
        "Music / pacing / recovery over 60 min",
        "category",
        "recovery",
        -15,
        Some(60),
    ),
    (
        "late_start",
        "First productive block after 14:00",
        "late_start",
        "",
        -10,
        Some(14),
    ),
    (
        "shipped_video",
        "Exported a video (proof of output)",
        "output",
        "video_export",
        15,
        Some(1),
    ),
    (
        "shipped_code",
        "Shipped code changes",
        "output",
        "code_change",
        10,
        Some(1),
    ),
    (
        "study_output",
        "Created/opened study material",
        "output",
        "study_material",
        5,
        Some(1),
    ),
];

fn migrate_starter_rules_v2(conn: &Connection) -> rusqlite::Result<()> {
    const FLAG: &str = "score_starter_rules_v2";
    let done = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [FLAG],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .is_some_and(|v| v == "1");
    if done {
        return Ok(());
    }
    for (id, label, kind, metric, weight, threshold) in LEGACY_PERSONAL_DEFAULTS {
        conn.execute(
            "DELETE FROM score_rules
             WHERE id = ?1 AND label = ?2 AND kind = ?3 AND metric = ?4
               AND weight = ?5 AND threshold IS ?6 AND built_in = 1",
            params![id, label, kind, metric, weight, threshold],
        )?;
    }
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [FLAG],
    )?;
    Ok(())
}

pub fn ensure_rule_defaults(conn: &Connection) -> rusqlite::Result<()> {
    migrate_starter_rules_v2(conn)?;
    let already = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [RULES_SEEDED],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .is_some_and(|v| v == "1");
    if already {
        return Ok(());
    }
    seed_defaults(conn)?;
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [RULES_SEEDED],
    )?;
    Ok(())
}
fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for (i, (id, label, kind, metric, weight, threshold)) in DEFAULT_RULES.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO score_rules
               (id, label, kind, metric, weight, threshold, built_in, sort_order, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)",
            params![id, label, kind, metric, weight, threshold, i as i64, now],
        )?;
    }
    Ok(())
}

pub fn list_rules(conn: &Connection) -> Vec<ScoreRule> {
    let _ = ensure_rule_defaults(conn);
    let mut stmt = match conn.prepare(
        "SELECT id, label, kind, metric, weight, threshold, built_in
         FROM score_rules ORDER BY sort_order, id",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |r| {
        Ok(ScoreRule {
            id: r.get(0)?,
            label: r.get(1)?,
            kind: r.get(2)?,
            metric: r.get(3)?,
            weight: r.get(4)?,
            threshold: r.get(5)?,
            built_in: r.get::<_, i64>(6)? != 0,
        })
    });
    match rows {
        Ok(rs) => rs.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn upsert_rule(conn: &Connection, rule: &ScoreRule) -> Result<(), String> {
    let id = rule.id.trim().to_ascii_lowercase();
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err("Rule id must use lowercase letters, numbers, dashes or underscores".into());
    }
    if rule.label.trim().is_empty() {
        return Err("Rule label is required".into());
    }
    if !RULE_KINDS.contains(&rule.kind.as_str()) {
        return Err(format!("Unknown rule kind: {}", rule.kind));
    }
    let metric = rule.metric.trim().to_ascii_lowercase();
    match rule.kind.as_str() {
        "checkin" => {
            if metric.is_empty() {
                return Err("Pick which check-in this rule reads".into());
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
        "target" => {
            if metric.is_empty() {
                return Err("Enter the app/site name this rule watches".into());
            }
        }
        "output" => {
            if !["video_export", "code_change", "study_material"].contains(&metric.as_str()) {
                return Err(
                    "Output metric must be video_export, code_change or study_material".into(),
                );
            }
        }
        _ => {} // goal / no_goal / late_start need no metric
    }
    let _ = ensure_rule_defaults(conn);
    let now = chrono::Utc::now().to_rfc3339();
    let sort: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM score_rules",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO score_rules (id, label, kind, metric, weight, threshold, built_in, sort_order, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
             label = excluded.label, kind = excluded.kind, metric = excluded.metric,
             weight = excluded.weight, threshold = excluded.threshold,
             updated_at = excluded.updated_at",
        params![
            id,
            rule.label.trim(),
            rule.kind,
            metric,
            rule.weight.clamp(-100, 100),
            rule.threshold.map(|t| t.clamp(0, 1440)),
            rule.built_in as i64,
            sort,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_rule(conn: &Connection, id: &str) -> Result<(), String> {
    let _ = ensure_rule_defaults(conn);
    conn.execute("DELETE FROM score_rules WHERE id = ?1", [id.trim()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_weight(conn: &Connection, id: &str, weight: i64) -> Result<(), String> {
    let _ = ensure_rule_defaults(conn);
    let n = conn
        .execute(
            "UPDATE score_rules SET weight = ?1, updated_at = ?2 WHERE id = ?3",
            params![weight.clamp(-100, 100), chrono::Utc::now().to_rfc3339(), id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err(format!("unknown rule: {id}"));
    }
    Ok(())
}

pub fn set_threshold(conn: &Connection, id: &str, threshold: i64) -> Result<(), String> {
    let _ = ensure_rule_defaults(conn);
    let has: Option<Option<i64>> = conn
        .query_row(
            "SELECT threshold FROM score_rules WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .ok();
    match has {
        None => Err(format!("unknown rule: {id}")),
        Some(None) => Err(format!("rule {id} has no threshold")),
        Some(Some(_)) => {
            conn.execute(
                "UPDATE score_rules SET threshold = ?1, updated_at = ?2 WHERE id = ?3",
                params![
                    threshold.clamp(0, 1440),
                    chrono::Utc::now().to_rfc3339(),
                    id
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

/// Restore the built-in rule set (drops custom rules and edits).
pub fn reset(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM score_rules", [])
        .map_err(|e| e.to_string())?;
    seed_defaults(conn).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn verdict(score: i64) -> &'static str {
    match score {
        85..=100 => "excellent",
        70..=84 => "good",
        50..=69 => "mid",
        30..=49 => "bad",
        _ => "cooked",
    }
}

fn clock(min_of_day: i64) -> String {
    format!("{:02}:{:02}", min_of_day / 60, min_of_day % 60)
}

/// Evaluate one rule. Returns (triggered, human-readable value).
fn eval_rule(
    rule: &ScoreRule,
    stats: &Stats,
    checkins: &Checkins,
    outputs: &OutputSignals,
    has_main_goal: bool,
) -> (bool, String) {
    let positive = rule.weight >= 0;
    let mins = |cat: &str| stats.cat_seconds.get(cat).copied().unwrap_or(0) / 60;
    match rule.kind.as_str() {
        "goal" => (
            has_main_goal && checkins.main_goal_completed,
            if !has_main_goal {
                "no goal".into()
            } else if checkins.main_goal_completed {
                "done".into()
            } else {
                "not done".into()
            },
        ),
        "no_goal" => (
            has_main_goal && !checkins.main_goal_completed,
            if !has_main_goal {
                "no goal".into()
            } else if checkins.main_goal_completed {
                "completed".into()
            } else {
                "not completed".into()
            },
        ),
        "checkin" => {
            // metric may be a comma-separated list — the max value wins ("any of").
            let value = rule
                .metric
                .split(',')
                .map(|m| checkins.values.get(m.trim()).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            let thr = rule.threshold.unwrap_or(1).max(1);
            let display = if thr <= 1 && value <= 1 {
                if value > 0 {
                    "logged".into()
                } else {
                    "not logged".into()
                }
            } else {
                format!("{value}×")
            };
            (value >= thr, display)
        }
        "category" => {
            let m = mins(&rule.metric);
            let thr = rule.threshold.unwrap_or(0);
            (if positive { m >= thr } else { m > thr }, format!("{m}m"))
        }
        "target" => {
            let m = stats.target_seconds.get(&rule.metric).copied().unwrap_or(0) / 60;
            let thr = rule.threshold.unwrap_or(0);
            (if positive { m >= thr } else { m > thr }, format!("{m}m"))
        }
        "late_start" => {
            let cutoff = rule.threshold.unwrap_or(14) * 60;
            match stats.first_productive_min {
                Some(m) => (m > cutoff, format!("first at {}", clock(m))),
                None => (false, "no productive block".into()),
            }
        }
        "output" => {
            let count = match rule.metric.as_str() {
                "video_export" => outputs.video_exports,
                "code_change" => outputs.code_changes,
                "study_material" => outputs.study_materials,
                _ => 0,
            };
            let thr = rule.threshold.unwrap_or(1).max(1);
            (count >= thr, format!("{count} detected"))
        }
        _ => (false, String::new()),
    }
}

pub fn build_report(
    conn: &Connection,
    day: String,
    stats: &Stats,
    checkins: &Checkins,
    outputs: &OutputSignals,
    main_goal_name: Option<String>,
) -> ScoreReport {
    let rules = list_rules(conn);
    let has_main_goal = main_goal_name.is_some();

    let mut raw = 0i64;
    let mut lines: Vec<ScoreLine> = Vec::new();
    for rule in &rules {
        let (triggered, value) = eval_rule(rule, stats, checkins, outputs, has_main_goal);
        if triggered {
            raw += rule.weight;
        }
        lines.push(ScoreLine {
            id: rule.id.clone(),
            label: rule.label.clone(),
            weight: rule.weight,
            threshold: rule.threshold,
            has_threshold: rule.threshold.is_some(),
            positive: rule.weight >= 0,
            triggered,
            value,
        });
    }

    let score = raw.clamp(0, 100);

    let mut top_wins: Vec<ScoreLine> = lines
        .iter()
        .filter(|l| l.positive && l.triggered)
        .cloned()
        .collect();
    top_wins.sort_by(|a, b| b.weight.cmp(&a.weight));
    top_wins.truncate(3);

    let mut biggest_leaks: Vec<ScoreLine> = lines
        .iter()
        .filter(|l| !l.positive && l.triggered)
        .cloned()
        .collect();
    biggest_leaks.sort_by(|a, b| a.weight.cmp(&b.weight)); // most negative first
    biggest_leaks.truncate(3);

    let goal_deadline = crate::settings::get_setting(conn, crate::settings::MAIN_GOAL_DEADLINE)
        .filter(|value| !value.trim().is_empty());
    let suggestion = build_suggestion(
        &rules,
        &lines,
        stats,
        checkins,
        &main_goal_name,
        goal_deadline.as_deref(),
    );

    let mut category_minutes: Vec<CategoryMinutes> = stats
        .cat_seconds
        .iter()
        .map(|(c, s)| CategoryMinutes {
            category: c.clone(),
            minutes: s / 60,
        })
        .filter(|c| c.minutes > 0)
        .collect();
    category_minutes.sort_by(|a, b| b.minutes.cmp(&a.minutes));

    let logged: Vec<CheckinValue> = crate::models::checkin_values_for_day(conn, &day)
        .into_iter()
        .filter(|c| c.value > 0)
        .collect();

    ScoreReport {
        date: day,
        score,
        verdict: verdict(score).to_string(),
        top_wins,
        biggest_leaks,
        suggestion,
        lines,
        category_minutes,
        main_goal_completed: checkins.main_goal_completed,
        checkins: logged,
        main_goal_name,
    }
}

fn build_suggestion(
    rules: &[ScoreRule],
    lines: &[ScoreLine],
    stats: &Stats,
    checkins: &Checkins,
    main_goal_name: &Option<String>,
    goal_deadline: Option<&str>,
) -> String {
    let mut cands: Vec<(i64, String)> = Vec::new();

    let has_goal_rules = rules
        .iter()
        .any(|r| r.kind == "goal" || r.kind == "no_goal");
    if has_goal_rules && !checkins.main_goal_completed {
        let gain: i64 = rules
            .iter()
            .filter(|r| r.kind == "goal" || r.kind == "no_goal")
            .map(|r| r.weight.abs())
            .sum();
        let suffix = main_goal_name
            .as_ref()
            .map(|n| format!(" ({n})"))
            .unwrap_or_default();
        let timing = goal_deadline
            .and_then(format_goal_deadline)
            .map(|deadline| format!(" by your preferred {deadline} deadline"))
            .unwrap_or_else(|| " when your schedule allows".to_string());
        cands.push((
            gain,
            format!("Finish your main goal{suffix}{timing} — worth {gain} points and removes the penalty."),
        ));
    }

    // Triggered leaks: cut below threshold.
    for l in lines
        .iter()
        .filter(|l| !l.positive && l.triggered && l.has_threshold)
    {
        cands.push((
            l.weight.abs(),
            format!(
                "Cut {} below {}m to save {} points.",
                l.label,
                l.threshold.unwrap_or(0),
                l.weight.abs()
            ),
        ));
    }

    // Almost-there positives: a category rule you've started but not finished.
    for rule in rules
        .iter()
        .filter(|r| r.kind == "category" && r.weight > 0)
    {
        let have = stats.cat_seconds.get(&rule.metric).copied().unwrap_or(0) / 60;
        let need = rule.threshold.unwrap_or(0) - have;
        if have > 0 && need > 0 {
            cands.push((
                rule.weight,
                format!(
                    "{need} more minutes of {} earns {} points.",
                    rule.metric, rule.weight
                ),
            ));
        }
    }

    cands.sort_by(|a, b| b.0.cmp(&a.0));
    cands
        .first()
        .map(|(_, t)| t.clone())
        .unwrap_or_else(|| "Strong, balanced day — keep the momentum tomorrow.".to_string())
}

fn format_goal_deadline(value: &str) -> Option<String> {
    let (hour, minute) = value.split_once(':')?;
    let hour = hour.parse::<u32>().ok()?;
    let minute = minute.parse::<u32>().ok()?;
    if hour >= 24 || minute >= 60 {
        return None;
    }
    let suffix = if hour < 12 { "am" } else { "pm" };
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    Some(if minute == 0 {
        format!("{display_hour}{suffix}")
    } else {
        format!("{display_hour}:{minute:02}{suffix}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn verdict_bands() {
        assert_eq!(verdict(100), "excellent");
        assert_eq!(verdict(85), "excellent");
        assert_eq!(verdict(84), "good");
        assert_eq!(verdict(70), "good");
        assert_eq!(verdict(50), "mid");
        assert_eq!(verdict(49), "bad");
        assert_eq!(verdict(30), "bad");
        assert_eq!(verdict(29), "cooked");
        assert_eq!(verdict(0), "cooked");
    }

    #[test]
    fn formats_optional_goal_deadline() {
        assert_eq!(format_goal_deadline("00:00").as_deref(), Some("12am"));
        assert_eq!(format_goal_deadline("14:30").as_deref(), Some("2:30pm"));
        assert_eq!(format_goal_deadline("24:00"), None);
        assert_eq!(format_goal_deadline(""), None);
    }

    #[test]
    fn rules_seed_once_and_survive_deletion() {
        let conn = db::test_conn();
        let n = list_rules(&conn).len();
        assert_eq!(n, DEFAULT_RULES.len());
        delete_rule(&conn, "main_goal").unwrap();
        assert_eq!(list_rules(&conn).len(), n - 1); // stays deleted
        reset(&conn).unwrap();
        assert_eq!(list_rules(&conn).len(), n); // reset restores defaults
    }

    #[test]
    fn custom_checkin_rule_triggers_on_value() {
        let conn = db::test_conn();
        crate::models::upsert_checkin_definition(
            &conn,
            &crate::models::CheckinDefinition {
                id: "growth_research".into(),
                label: "Growth research".into(),
                icon: "📈".into(),
                kind: "toggle".into(),
                built_in: false,
                auto_kind: String::new(),
                auto_metric: String::new(),
                auto_threshold: 0,
            },
        )
        .unwrap();
        upsert_rule(
            &conn,
            &ScoreRule {
                id: "growth".into(),
                label: "Did growth research".into(),
                kind: "checkin".into(),
                metric: "growth_research".into(),
                weight: 10,
                threshold: Some(1),
                built_in: false,
            },
        )
        .unwrap();

        let stats = Stats::default();
        let outputs = OutputSignals::default();
        let mut values = HashMap::new();
        values.insert("growth_research".to_string(), 1i64);
        let checkins = Checkins {
            main_goal_completed: false,
            values,
        };
        let report = build_report(
            &conn,
            "2026-01-01".into(),
            &stats,
            &checkins,
            &outputs,
            None,
        );
        let line = report.lines.iter().find(|l| l.id == "growth").unwrap();
        assert!(line.triggered);
    }

    #[test]
    fn any_of_checkin_metric_matches_either() {
        let conn = db::test_conn();
        crate::models::ensure_checkin_defaults(&conn).unwrap();
        upsert_rule(
            &conn,
            &ScoreRule {
                id: "gym".into(),
                label: "Gym / wrestling".into(),
                kind: "checkin".into(),
                metric: "gym_logged,wrestled".into(),
                weight: 10,
                threshold: Some(1),
                built_in: false,
            },
        )
        .unwrap();
        let stats = Stats::default();
        let outputs = OutputSignals::default();
        let mut values = HashMap::new();
        values.insert("wrestled".to_string(), 1i64); // gym rule reads gym_logged,wrestled
        let checkins = Checkins {
            main_goal_completed: false,
            values,
        };
        let report = build_report(
            &conn,
            "2026-01-01".into(),
            &stats,
            &checkins,
            &outputs,
            None,
        );
        let gym = report.lines.iter().find(|l| l.id == "gym").unwrap();
        assert!(gym.triggered);
    }

    #[test]
    fn target_rule_reads_target_seconds() {
        let conn = db::test_conn();
        upsert_rule(
            &conn,
            &ScoreRule {
                id: "instagram".into(),
                label: "Instagram over 30 min".into(),
                kind: "target".into(),
                metric: "instagram".into(),
                weight: -15,
                threshold: Some(30),
                built_in: false,
            },
        )
        .unwrap();
        let mut stats = Stats::default();
        stats.target_seconds.insert("instagram".into(), 40 * 60);
        let checkins = Checkins {
            main_goal_completed: false,
            values: HashMap::new(),
        };
        let report = build_report(
            &conn,
            "2026-01-01".into(),
            &stats,
            &checkins,
            &OutputSignals::default(),
            None,
        );
        let insta = report.lines.iter().find(|l| l.id == "instagram").unwrap();
        assert!(insta.triggered); // 40m > 30m threshold
    }

    #[test]
    fn rule_validation_rejects_unknowns() {
        let conn = db::test_conn();
        let bad_cat = ScoreRule {
            id: "x".into(),
            label: "X".into(),
            kind: "category".into(),
            metric: "nope".into(),
            weight: 5,
            threshold: Some(30),
            built_in: false,
        };
        assert!(upsert_rule(&conn, &bad_cat).is_err());
        let bad_checkin = ScoreRule {
            id: "y".into(),
            label: "Y".into(),
            kind: "checkin".into(),
            metric: "nope".into(),
            weight: 5,
            threshold: Some(1),
            built_in: false,
        };
        assert!(upsert_rule(&conn, &bad_checkin).is_err());
    }
}
